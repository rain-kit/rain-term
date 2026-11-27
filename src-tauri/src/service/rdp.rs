//! 远程桌面会话：每个标签一条 RDP 连接；设置了跳板机时经连接池里的 SSH 连接转发。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::conn::ConnService;
use super::hostkey::HostKeyService;
use super::pool::{Client, ClientPool};
use crate::core::rdp::input::InputEvent;
use crate::core::rdp::{self, BoxStream, Command, Event, Flow, RdpTarget};
use crate::error::{self, ApiResult};
use crate::i18n::Msg;
use crate::model::{ConnParams, Connection, RdpParams, RdpResolution};
use crate::util::new_id;

/// 会话推给前端的通知；画面和指针走二进制消息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RdpNotice {
    /// 远程分辨率。
    Size { width: u16, height: u16 },
    /// 远程复制的文本，由前端写入本机剪贴板。
    Clipboard { text: String },
    /// 会话结束，一定是最后一条；用户主动关闭时不发。
    Closed {
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<Msg>,
    },
}

#[derive(Debug)]
pub enum RdpEvent {
    /// 画面或指针，格式见 core::rdp::session。
    Data(Vec<u8>),
    Notice(RdpNotice),
}

/// 会话数据的去处，commands 层把它接到 tauri::ipc::Channel。
pub type RdpSink = Box<dyn Fn(RdpEvent) + Send + Sync>;

struct Session {
    commands: mpsc::UnboundedSender<Command>,
    flow: Arc<Flow>,
    cancel: CancellationToken,
}

pub struct RdpService {
    conns: Arc<ConnService>,
    pool: Arc<ClientPool>,
    host_keys: Arc<HostKeyService>,
    sessions: Mutex<HashMap<String, Session>>,
}

/// 远程桌面的参数，其他类型的连接返回错误。
fn rdp_params(conn: &Connection) -> ApiResult<&RdpParams> {
    match &conn.params {
        ConnParams::Rdp(p) => Ok(p),
        _ => Err(error::CONN_INVALID.msg(msg!("err.conn.notRdp", "这不是远程桌面连接"))),
    }
}

impl RdpService {
    pub fn new(
        conns: Arc<ConnService>,
        pool: Arc<ClientPool>,
        host_keys: Arc<HostKeyService>,
    ) -> Arc<Self> {
        Arc::new(RdpService {
            conns,
            pool,
            host_keys,
            sessions: Mutex::default(),
        })
    }

    /// 打开远程桌面，返回会话 ID。width、height 是窗口大小（物理像素），分辨率跟随窗口时使用；
    /// scale 是系统缩放比例（百分比）。
    pub async fn open(
        self: &Arc<Self>,
        conn_id: &str,
        width: u32,
        height: u32,
        scale: u32,
        sink: RdpSink,
    ) -> ApiResult<String> {
        let conn = self.conns.resolve(conn_id)?;
        let p = rdp_params(&conn)?;
        let (w, h) = match p.resolution {
            RdpResolution::Fit => (width, height),
            RdpResolution::Fixed => (p.width as u32, p.height as u32),
        };
        let target = RdpTarget::from_params(p, w, h, scale);
        let opts = self.host_keys.rdp_options(&conn.id, &conn.name);
        let cancel = CancellationToken::new();

        let (stream, jump) = self.stream(&conn, p, &cancel).await?;
        let begin = Instant::now();
        let connected = match rdp::connect(stream, &target, &opts, &cancel, begin).await {
            Ok(c) => c,
            Err(e) => {
                if let Some(j) = &jump {
                    self.pool.release(j);
                }
                warn!(conn = %conn.name, err = %e, "连接远程桌面失败");
                return Err(e.into());
            }
        };

        let id = new_id();
        let (tx, rx) = mpsc::unbounded_channel();
        let flow = Arc::new(Flow::default());
        self.sessions.lock().unwrap().insert(
            id.clone(),
            Session {
                commands: tx,
                flow: flow.clone(),
                cancel: cancel.clone(),
            },
        );
        info!(conn = %conn.name, session = %id, elapsed_ms = connected.elapsed.as_millis() as u64, "远程桌面已连接");

        let svc = self.clone();
        let (sid, name) = (id.clone(), conn.name.clone());
        let jump_name = conn
            .jump_ref()
            .and_then(|j| self.conns.get(j).ok())
            .map(|v| v.conn.name)
            .unwrap_or_default();
        tokio::spawn(async move {
            let emit = |ev: Event| {
                sink(match ev {
                    Event::Frame(b) | Event::Pointer(b) => RdpEvent::Data(b),
                    Event::Size { width, height } => {
                        RdpEvent::Notice(RdpNotice::Size { width, height })
                    }
                    Event::Clipboard(text) => RdpEvent::Notice(RdpNotice::Clipboard { text }),
                })
            };
            let mut end = rdp::run(connected, rx, emit, flow, cancel.clone()).await;
            if let Some(j) = &jump
                && !j.alive()
            {
                let reason = j
                    .reason()
                    .unwrap_or_else(|| msg!("err.ssh.connectionLost", "连接断开"));
                end = Some(msg!(
                    "err.rdp.jumpLost",
                    "跳板机 {name} 断开：{reason}",
                    name = jump_name,
                    reason
                ));
            }
            svc.sessions.lock().unwrap().remove(&sid);
            if let Some(j) = &jump {
                svc.pool.release(j);
            }
            if cancel.is_cancelled() {
                info!(conn = %name, session = %sid, "远程桌面已关闭");
                return;
            }
            match &end {
                Some(e) => warn!(conn = %name, session = %sid, err = %e.text(), "远程桌面断开"),
                None => info!(conn = %name, session = %sid, "远程桌面会话已结束"),
            }
            sink(RdpEvent::Notice(RdpNotice::Closed { error: end }));
        });
        Ok(id)
    }

    /// 连到远程桌面端口的字节流：直连，或经跳板机转发。经跳板机时同时返回占用的连接池连接。
    async fn stream(
        &self,
        conn: &Connection,
        p: &RdpParams,
        cancel: &CancellationToken,
    ) -> ApiResult<(BoxStream, Option<Arc<Client>>)> {
        let port = p.port as u16;
        let Some(jump_id) = conn.jump_ref() else {
            let s = rdp::dial_tcp(&p.host, port, rdp::DEFAULT_TIMEOUT, cancel).await?;
            return Ok((s, None));
        };
        let jump = self.conns.resolve(jump_id)?;
        let jp = jump.ssh_params().map_err(|m| error::CONN_INVALID.msg(m))?;
        // 复用的连接可能刚断开、还没被发现，这时重新连接再试一次。
        let mut retry = true;
        loop {
            let (client, reused) = self.pool.acquire(&jump, jp).await?;
            match client
                .ssh
                .open_tunnel(&jump.name, &p.host, port, rdp::DEFAULT_TIMEOUT, cancel)
                .await
            {
                Ok(s) => return Ok((Box::new(s), Some(client))),
                Err(e) => {
                    self.pool.release(&client);
                    if reused && retry && !client.alive() {
                        retry = false;
                        continue;
                    }
                    warn!(conn = %conn.name, err = %e, "经跳板机连接远程桌面失败");
                    return Err(error::RDP_CONNECT_FAILED.msg(e.msg()));
                }
            }
        }
    }

    fn send(&self, id: &str, cmd: Command) -> ApiResult<()> {
        let sessions = self.sessions.lock().unwrap();
        let s = sessions.get(id).ok_or_else(|| error::RDP_NOT_FOUND.err())?;
        s.commands
            .send(cmd)
            .map_err(|_| error::RDP_NOT_FOUND.err())
    }

    pub fn input(&self, id: &str, events: Vec<InputEvent>) -> ApiResult<()> {
        self.send(id, Command::Input(events))
    }

    pub fn ctrl_alt_del(&self, id: &str) -> ApiResult<()> {
        self.send(id, Command::CtrlAltDel)
    }

    pub fn release_all(&self, id: &str) -> ApiResult<()> {
        self.send(id, Command::ReleaseAll)
    }

    pub fn resize(&self, id: &str, width: u32, height: u32, scale: u32) -> ApiResult<()> {
        self.send(
            id,
            Command::Resize {
                width,
                height,
                scale,
            },
        )
    }

    pub fn clipboard(&self, id: &str, text: String) -> ApiResult<()> {
        self.send(id, Command::Clipboard(text))
    }

    /// 前端画完了一条画面消息。
    pub fn ack(&self, id: &str) {
        if let Some(s) = self.sessions.lock().unwrap().get(id) {
            s.flow.ack();
        }
    }

    /// 关闭会话；不再推送任何消息。
    pub fn close(&self, id: &str) {
        if let Some(s) = self.sessions.lock().unwrap().remove(id) {
            s.cancel.cancel();
        }
    }

    /// 关闭全部会话：程序退出、前端页面重新加载时调用，之后应再关闭连接池。
    pub fn close_all(&self) {
        let sessions = std::mem::take(&mut *self.sessions.lock().unwrap());
        for s in sessions.values() {
            s.cancel.cancel();
        }
        if !sessions.is_empty() {
            info!(count = sessions.len(), "已关闭全部远程桌面");
        }
    }
}
