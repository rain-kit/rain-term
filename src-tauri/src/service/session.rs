//! 终端会话：同一连接的多个标签共用连接池里的一条 SSH 连接，每个标签一个 channel。

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::Notify;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::conn::ConnService;
use super::pool::{Client, ClientPool};
use crate::core::ssh::shell::{ExitStatus, PtySize, ShellEvent, ShellReader, ShellWriter};
use crate::core::ssh::{SshClient, SshError};
use crate::error::{self, ApiResult};
use crate::i18n::Msg;
use crate::model::ConnParams;
use crate::util::new_id;

/// 输出攒够这么多字节，或者从第一个字节起等了 FLUSH_INTERVAL，就合并发给前端。
pub const FLUSH_BYTES: usize = 32 * 1024;
pub const FLUSH_INTERVAL: Duration = Duration::from_millis(16);
/// 已发给前端、还没确认处理完的字节数达到它时暂停读取，前端处理不过来时内存不会一直涨。
/// 暂停期间同一条 SSH 连接上的其他会话也会停住（russh 在连接的主循环里等待 channel 有空位）。
pub const MAX_UNACKED: usize = 1024 * 1024;
/// channel 没有退出状态就结束、且 SSH 连接已断时，等断开原因的最长时间。
const REASON_GRACE: Duration = Duration::from_millis(500);
/// 关闭会话时给服务器发 EOF / close 的最长等待时间。
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);

/// 会话结束：shell 退出或连接断开。用户主动关闭时不发。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Closed {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_signal: Option<String>,
    /// 连接异常断开的原因，shell 正常退出时为空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Msg>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    Output(Vec<u8>),
    /// 一定是最后一条。
    Closed(Closed),
}

/// 会话输出的去处，commands 层把它接到 tauri::ipc::Channel。
pub type SessionSink = Box<dyn Fn(SessionEvent) + Send + Sync>;

#[derive(Default)]
struct Flow {
    unacked: AtomicUsize,
    acked: Notify,
}

struct Session {
    writer: Arc<ShellWriter>,
    cancel: CancellationToken,
    flow: Arc<Flow>,
}

pub struct SessionService {
    conns: Arc<ConnService>,
    pool: Arc<ClientPool>,
    sessions: Mutex<HashMap<String, Session>>,
}

enum End {
    /// 用户关闭了会话。
    Closed,
    /// channel 结束（shell 退出或连接断开）。
    Eof,
    Dead,
}

impl SessionService {
    pub fn new(conns: Arc<ConnService>, pool: Arc<ClientPool>) -> Arc<Self> {
        Arc::new(SessionService {
            conns,
            pool,
            sessions: Mutex::default(),
        })
    }

    /// 打开终端会话，返回会话 ID；输出和结束通知从 sink 推出。
    pub async fn open(
        self: &Arc<Self>,
        conn_id: &str,
        size: PtySize,
        sink: SessionSink,
    ) -> ApiResult<String> {
        let conn = self.conns.resolve(conn_id)?;
        let ConnParams::Ssh(p) = &conn.params;
        let size = PtySize {
            cols: size.cols.max(1),
            rows: size.rows.max(1),
        };

        // 复用的连接可能刚断开、还没被发现，这时重新连接再试一次。
        let mut retry = true;
        let (client, writer, reader) = loop {
            let (client, reused) = self.pool.acquire(&conn, p).await?;
            match start_shell(&client.ssh, size, &p.init_dir).await {
                Ok((w, r)) => break (client, w, r),
                Err(e) => {
                    self.pool.release(&client);
                    if reused && retry && !client.alive() {
                        retry = false;
                        continue;
                    }
                    warn!(conn = %conn.name, err = %e, "打开终端失败");
                    return Err(e.into());
                }
            }
        };

        let id = new_id();
        let cancel = CancellationToken::new();
        let flow = Arc::new(Flow::default());
        let writer = Arc::new(writer);
        self.sessions.lock().unwrap().insert(
            id.clone(),
            Session {
                writer,
                cancel: cancel.clone(),
                flow: flow.clone(),
            },
        );
        info!(conn = %conn.name, session = %id, "终端会话已打开");

        let svc = self.clone();
        let (sid, name) = (id.clone(), conn.name.clone());
        tokio::spawn(async move {
            let closed = pump(reader, &sink, &client, &flow, &cancel).await;
            match &closed {
                None => info!(conn = %name, session = %sid, "终端会话已关闭"),
                Some(c) => match &c.error {
                    Some(e) => warn!(conn = %name, session = %sid, err = %e.text(), "终端会话断开"),
                    None => {
                        info!(conn = %name, session = %sid, exit = ?c.exit_code, signal = ?c.exit_signal, "shell 已退出")
                    }
                },
            }
            svc.sessions.lock().unwrap().remove(&sid);
            svc.pool.release(&client);
            if let Some(c) = closed {
                sink(SessionEvent::Closed(c));
            }
        });
        Ok(id)
    }

    fn writer(&self, id: &str) -> ApiResult<Arc<ShellWriter>> {
        let sessions = self.sessions.lock().unwrap();
        sessions
            .get(id)
            .map(|s| s.writer.clone())
            .ok_or_else(|| error::SESSION_NOT_FOUND.err())
    }

    pub async fn write(&self, id: &str, data: &[u8]) -> ApiResult<()> {
        let w = self.writer(id)?;
        w.write(data)
            .await
            .map_err(|e| error::SESSION_WRITE_FAILED.msg(e.msg()))
    }

    pub async fn resize(&self, id: &str, size: PtySize) -> ApiResult<()> {
        let w = self.writer(id)?;
        let size = PtySize {
            cols: size.cols.max(1),
            rows: size.rows.max(1),
        };
        w.resize(size)
            .await
            .map_err(|e| error::SESSION_WRITE_FAILED.msg(e.msg()))
    }

    /// 前端已处理完 bytes 字节输出。
    pub fn ack(&self, id: &str, bytes: usize) {
        let sessions = self.sessions.lock().unwrap();
        if let Some(s) = sessions.get(id) {
            let _ = s
                .flow
                .unacked
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                    Some(n.saturating_sub(bytes))
                });
            s.flow.acked.notify_one();
        }
    }

    /// 关闭会话；不再推送任何消息。
    pub async fn close(&self, id: &str) {
        let Some(s) = self.sessions.lock().unwrap().remove(id) else {
            return;
        };
        // 先停掉读取：读取暂停时 russh 的主循环可能正卡着，发 close 会一直等。
        s.cancel.cancel();
        let _ = tokio::time::timeout(CLOSE_TIMEOUT, s.writer.close()).await;
    }

    /// 关闭全部会话：程序退出、前端页面重新加载时调用，之后应再关闭连接池。
    pub fn close_all(&self) {
        let sessions = std::mem::take(&mut *self.sessions.lock().unwrap());
        for s in sessions.values() {
            s.cancel.cancel();
        }
        if !sessions.is_empty() {
            info!(count = sessions.len(), "已关闭全部终端会话");
        }
    }

    pub fn session_count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }
}

async fn start_shell(
    ssh: &SshClient,
    size: PtySize,
    init_dir: &str,
) -> Result<(ShellWriter, ShellReader), SshError> {
    let (w, r) = ssh.open_shell(size).await?;
    if !init_dir.is_empty()
        && let Err(e) = w.write(cd_command(init_dir).as_bytes()).await
    {
        w.close().await;
        return Err(e);
    }
    Ok((w, r))
}

/// 进入登录后初始目录的命令。开头的 ~/ 不加引号，shell 才会把它展开成主目录。
fn cd_command(dir: &str) -> String {
    let quote = |s: &str| format!("'{}'", s.replace('\'', r"'\''"));
    let arg = match dir.strip_prefix("~/") {
        _ if dir == "~" => "~".to_string(),
        Some("") => "~/".to_string(),
        Some(rest) => format!("~/{}", quote(rest)),
        None => quote(dir),
    };
    format!("cd {arg}\r")
}

/// 把 shell 输出合并后推给 sink，直到会话结束；用户关闭时返回 None。
async fn pump(
    mut reader: ShellReader,
    sink: &SessionSink,
    client: &Client,
    flow: &Flow,
    cancel: &CancellationToken,
) -> Option<Closed> {
    let mut buf: Vec<u8> = Vec::with_capacity(FLUSH_BYTES);
    let mut deadline: Option<Instant> = None;
    let mut exit = None;
    let flush = |buf: &mut Vec<u8>| {
        if !buf.is_empty() {
            let data = std::mem::replace(buf, Vec::with_capacity(FLUSH_BYTES));
            flow.unacked.fetch_add(data.len(), Ordering::SeqCst);
            sink(SessionEvent::Output(data));
        }
    };

    let end = 'pump: loop {
        while flow.unacked.load(Ordering::SeqCst) >= MAX_UNACKED {
            tokio::select! {
                _ = cancel.cancelled() => break 'pump End::Closed,
                _ = client.dead.cancelled() => break 'pump End::Dead,
                _ = flow.acked.notified() => {}
            }
        }
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break End::Closed,
            ev = reader.next() => match ev {
                Some(ShellEvent::Output(d)) => {
                    buf.extend_from_slice(&d);
                    if buf.len() >= FLUSH_BYTES {
                        flush(&mut buf);
                        deadline = None;
                    } else if deadline.is_none() {
                        deadline = Some(Instant::now() + FLUSH_INTERVAL);
                    }
                }
                Some(ShellEvent::Exit(s)) => exit = Some(s),
                None => break End::Eof,
            },
            _ = tokio::time::sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {
                flush(&mut buf);
                deadline = None;
            }
            _ = client.dead.cancelled() => break End::Dead,
        }
    };

    if matches!(end, End::Closed) {
        return None;
    }
    flush(&mut buf);
    let mut closed = Closed::default();
    match exit {
        Some(ExitStatus::Code(c)) => closed.exit_code = Some(c),
        Some(ExitStatus::Signal(s)) => closed.exit_signal = Some(s),
        None if matches!(end, End::Dead) || client.ssh.is_closed() => {
            let _ = tokio::time::timeout(REASON_GRACE, client.dead.cancelled()).await;
            closed.error = Some(
                client
                    .reason()
                    .unwrap_or_else(|| msg!("err.ssh.connectionLost", "连接断开")),
            );
        }
        None => {}
    }
    Some(closed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cd_quotes_dir() {
        assert_eq!(cd_command("/srv/app"), "cd '/srv/app'\r");
        assert_eq!(cd_command("/data/it's here"), "cd '/data/it'\\''s here'\r");
        assert_eq!(cd_command("~"), "cd ~\r");
        assert_eq!(cd_command("~/"), "cd ~/\r");
        assert_eq!(cd_command("~/my logs"), "cd ~/'my logs'\r");
        assert_eq!(cd_command("~root"), "cd '~root'\r");
    }

    #[test]
    fn closed_json() {
        let c = Closed {
            exit_code: Some(0),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&c).unwrap(),
            serde_json::json!({ "exitCode": 0 })
        );
        let c = Closed {
            error: Some(msg!("err.ssh.serverClosed", "服务器关闭了连接")),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&c).unwrap(),
            serde_json::json!({ "error": { "i18n": "err.ssh.serverClosed" } })
        );
    }
}
