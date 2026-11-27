//! SSH 连接池：同一连接的终端标签、SFTP 面板共用一条 SSH 连接，最后一个使用者释放时断开。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::conn::ConnService;
use crate::core::ssh::SshClient;
use crate::error::ApiResult;
use crate::i18n::Msg;
use crate::model::{Connection, SshParams};

pub struct Client {
    conn_id: String,
    pub ssh: Arc<SshClient>,
    /// SSH 连接断开（服务器关闭、保活超时、被关闭）后取消，原因见 [`Client::reason`]。
    pub dead: CancellationToken,
    reason: Mutex<Option<Msg>>,
    /// 由连接池主动关闭，断开时不记警告日志。
    closing: AtomicBool,
}

impl Client {
    pub fn alive(&self) -> bool {
        !self.dead.is_cancelled() && !self.ssh.is_closed()
    }

    /// 断开原因；dead 取消后才有值，服务器没说明原因时为 None。
    pub fn reason(&self) -> Option<Msg> {
        self.reason.lock().unwrap().clone()
    }

    fn shutdown(&self) {
        self.closing.store(true, Ordering::SeqCst);
        self.ssh.close();
    }
}

struct Shared {
    client: Arc<Client>,
    /// 使用者数，含正在打开的。
    refs: usize,
}

pub struct ClientPool {
    conns: Arc<ConnService>,
    clients: Mutex<HashMap<String, Shared>>,
    /// 每个连接一把锁：同时打开多个标签时只建一条 SSH 连接。
    dialing: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl ClientPool {
    pub fn new(conns: Arc<ConnService>) -> Arc<Self> {
        Arc::new(ClientPool {
            conns,
            clients: Mutex::default(),
            dialing: Mutex::default(),
        })
    }

    /// 取得连接的 SSH 客户端，没有可用的就新建；返回值的第二项表示是否复用了已有连接。
    /// 用完后必须调用 [`ClientPool::release`]。
    pub async fn acquire(
        &self,
        conn: &Connection,
        p: &SshParams,
    ) -> ApiResult<(Arc<Client>, bool)> {
        let gate = self
            .dialing
            .lock()
            .unwrap()
            .entry(conn.id.clone())
            .or_default()
            .clone();
        let _gate = gate.lock().await;
        if let Some(c) = self.reuse(&conn.id) {
            return Ok((c, true));
        }

        let connected = self
            .conns
            .dial_ssh(conn, p, true)
            .await
            .inspect_err(|e| warn!(conn = %conn.name, err = %e.msg.text(), "SSH 连接失败"))?;
        info!(conn = %conn.name, elapsed_ms = connected.elapsed.as_millis() as u64, "SSH 已连接");
        let client = Arc::new(Client {
            conn_id: conn.id.clone(),
            ssh: connected.client,
            dead: CancellationToken::new(),
            reason: Mutex::new(None),
            closing: AtomicBool::new(false),
        });

        let (c, name, disconnected) = (client.clone(), conn.name.clone(), connected.disconnected);
        tokio::spawn(async move {
            let reason = disconnected.await.ok();
            if !c.closing.load(Ordering::SeqCst) {
                let err = reason
                    .as_ref()
                    .map(|m| m.text().to_string())
                    .unwrap_or_default();
                warn!(conn = %name, err = %err, "SSH 连接已断开");
            }
            *c.reason.lock().unwrap() = reason;
            c.dead.cancel();
        });

        self.clients.lock().unwrap().insert(
            conn.id.clone(),
            Shared {
                client: client.clone(),
                refs: 1,
            },
        );
        Ok((client, false))
    }

    /// 取得连接已建立的 SSH 客户端，没有可用的时返回 None，不新建连接。
    /// 取到后同样必须调用 [`ClientPool::release`]。
    pub fn existing(&self, conn_id: &str) -> Option<Arc<Client>> {
        self.reuse(conn_id)
    }

    fn reuse(&self, conn_id: &str) -> Option<Arc<Client>> {
        let mut clients = self.clients.lock().unwrap();
        let shared = clients.get_mut(conn_id)?;
        if shared.client.alive() {
            shared.refs += 1;
            return Some(shared.client.clone());
        }
        // 已断开的连接上剩下的使用者很快会自己结束，这里先让出位置。
        let dead = clients.remove(conn_id)?;
        dead.client.shutdown();
        None
    }

    /// 不再使用 client；最后一个使用者释放时断开 SSH 连接。
    pub fn release(&self, client: &Arc<Client>) {
        let mut clients = self.clients.lock().unwrap();
        let Some(shared) = clients.get_mut(&client.conn_id) else {
            return;
        };
        if !Arc::ptr_eq(&shared.client, client) {
            return;
        }
        shared.refs -= 1;
        if shared.refs == 0 {
            clients.remove(&client.conn_id);
            drop(clients);
            client.shutdown();
        }
    }

    /// 断开全部 SSH 连接：程序退出、前端页面重新加载时调用。
    pub fn close_all(&self) {
        let clients = std::mem::take(&mut *self.clients.lock().unwrap());
        for c in clients.values() {
            c.client.shutdown();
        }
    }

    /// 当前保持着的 SSH 连接数。
    pub fn count(&self) -> usize {
        self.clients.lock().unwrap().len()
    }
}
