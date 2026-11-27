mod common;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rain_dial::core::rdp::cert::CertStore;
use rain_dial::core::ssh::known_hosts::HostKeyStore;
use rain_dial::core::ssh::shell::PtySize;
use rain_dial::error::{self, ApiError};
use rain_dial::events::NoopSink;
use rain_dial::i18n::Param;
use rain_dial::model::{ConnParams, Connection, SaveConnReq, SshParams};
use rain_dial::secrets::{MemoryBackend, Secrets};
use rain_dial::service::conn::ConnService;
use rain_dial::service::hostkey::HostKeyService;
use rain_dial::service::pool::ClientPool;
use rain_dial::service::session::{Closed, SessionEvent, SessionService};
use rain_dial::store::ConnStore;
use tokio::net::TcpListener;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use common::{PASSWORD, TestServer, USER, temp_dir};

const WAIT: Duration = Duration::from_secs(10);
const SIZE: PtySize = PtySize { cols: 80, rows: 24 };

struct Fixture {
    svc: Arc<SessionService>,
    pool: Arc<ClientPool>,
    conns: Arc<ConnService>,
    host_keys: Arc<HostKeyService>,
    dir: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.svc.close_all();
        self.pool.close_all();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// trusted 里的服务器指纹预先信任，其他服务器连接时会询问。
fn fixture(trusted: &[&TestServer]) -> Fixture {
    let dir = temp_dir();
    let known = dir.join("known_hosts");
    let store = HostKeyStore::new(known.clone(), vec![]);
    for srv in trusted {
        store
            .add("127.0.0.1", srv.addr.port(), srv.key.public_key())
            .unwrap();
    }
    let host_keys = HostKeyService::new(
        HostKeyStore::new(known, vec![]),
        CertStore::new(dir.join("rdp_known_hosts")),
        Arc::new(NoopSink),
    );
    let conns = Arc::new(ConnService::new(
        ConnStore::new(dir.join("connections.json")),
        Secrets::new(Box::new(MemoryBackend::default())),
        host_keys.clone(),
        Arc::new(NoopSink),
    ));
    let pool = ClientPool::new(conns.clone());
    Fixture {
        svc: SessionService::new(conns.clone(), pool.clone()),
        pool,
        conns,
        host_keys,
        dir,
    }
}

impl Fixture {
    /// 保存一个 SSH 连接，返回连接 ID。
    fn add(&self, name: &str, port: u16, password: &str, jump_id: &str) -> String {
        let params = SshParams {
            host: "127.0.0.1".into(),
            port: port as i32,
            user: USER.into(),
            password: password.into(),
            jump_id: jump_id.into(),
            ..Default::default()
        };
        let conn = Connection {
            name: name.into(),
            params: ConnParams::Ssh(params),
            ..Default::default()
        };
        self.conns
            .save(SaveConnReq {
                conn,
                ..Default::default()
            })
            .unwrap()
            .conn
            .id
    }

    async fn try_open(
        &self,
        conn_id: &str,
    ) -> Result<(String, UnboundedReceiver<SessionEvent>), ApiError> {
        let (tx, rx) = unbounded_channel();
        let id = self
            .svc
            .open(conn_id, SIZE, false, Box::new(move |e| drop(tx.send(e))))
            .await?;
        Ok((id, rx))
    }

    /// 打开终端，执行 echo 确认能用，返回会话 ID 和输出接收端。
    async fn open_and_echo(&self, conn_id: &str) -> (String, UnboundedReceiver<SessionEvent>) {
        let (id, mut rx) = self.try_open(conn_id).await.unwrap();
        self.svc.write(&id, b"echo via jump\r").await.unwrap();
        let mut out = Vec::new();
        tokio::time::timeout(WAIT, async {
            while !String::from_utf8_lossy(&out).contains("\r\nvia jump\r\n") {
                match rx.recv().await {
                    Some(SessionEvent::Output(d)) => {
                        self.svc.ack(&id, d.len());
                        out.extend_from_slice(&d);
                    }
                    other => panic!("收到 {other:?}"),
                }
            }
        })
        .await
        .expect("等待 echo 输出超时");
        (id, rx)
    }
}

fn text_param(e: &ApiError, name: &str) -> String {
    e.msg
        .params
        .get(name)
        .map(|p| p.to_string())
        .unwrap_or_default()
}

fn nested_key(e: &ApiError, name: &str) -> &'static str {
    match e.msg.params.get(name) {
        Some(Param::Msg(m)) => m.i18n,
        other => panic!("参数 {name} 应是嵌套消息，实际为 {other:?}"),
    }
}

#[tokio::test]
async fn one_jump() {
    let (jump, target) = (TestServer::start().await, TestServer::start().await);
    let f = fixture(&[&jump, &target]);
    let j = f.add("跳板", jump.addr.port(), PASSWORD, "");
    let t = f.add("内网", target.addr.port(), PASSWORD, &j);

    f.open_and_echo(&t).await;
    f.open_and_echo(&t).await;
    assert_eq!(
        (
            jump.accepted.load(Ordering::SeqCst),
            target.accepted.load(Ordering::SeqCst)
        ),
        (1, 1),
        "两个标签共用一条跳板链"
    );

    let conn = f.conns.get(&t).unwrap().conn;
    let r = f
        .conns
        .test(SaveConnReq {
            conn,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(r.server_version.unwrap().starts_with("SSH-2.0-"));

    // 跳板机密码错误：提示是哪一台。
    let bad = f.add("坏跳板", jump.addr.port(), "wrong", "");
    let t2 = f.add("内网2", target.addr.port(), PASSWORD, &bad);
    let e = f.try_open(&t2).await.unwrap_err();
    assert_eq!(
        (e.code, e.msg.i18n),
        (error::SSH_CONNECT_FAILED.0, "err.ssh.jumpFailed")
    );
    assert_eq!(text_param(&e, "name"), "坏跳板");
    assert_eq!(nested_key(&e, "reason"), "err.ssh.authDenied");

    // 目标主机密码错误：直接是目标主机的错误。
    let t3 = f.add("内网3", target.addr.port(), "wrong", &j);
    let e = f.try_open(&t3).await.unwrap_err();
    assert_eq!(e.msg.i18n, "err.ssh.authDenied");
}

#[tokio::test]
async fn two_jumps_verify_every_host_key() {
    let (j1, j2, target) = (
        TestServer::start().await,
        TestServer::start().await,
        TestServer::start().await,
    );
    let f = fixture(&[]);
    let a = f.add("一级", j1.addr.port(), PASSWORD, "");
    let b = f.add("二级", j2.addr.port(), PASSWORD, &a);
    let t = f.add("内网", target.addr.port(), PASSWORD, &b);

    let asked = Arc::new(Mutex::new(Vec::new()));
    let answerer = tokio::spawn({
        let (hk, asked) = (f.host_keys.clone(), asked.clone());
        async move {
            loop {
                for p in hk.pending() {
                    asked.lock().unwrap().push(p.conn_name.clone());
                    hk.answer(&p.request_id, true).unwrap();
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    });
    f.open_and_echo(&t).await;
    answerer.abort();
    assert_eq!(
        asked.lock().unwrap().as_slice(),
        ["一级", "二级", "内网"],
        "每一级都确认指纹"
    );
    for srv in [&j1, &j2, &target] {
        assert_eq!(srv.accepted.load(Ordering::SeqCst), 1);
    }

    // 指纹已记住，再测试连接不会询问。
    let conn = f.conns.get(&t).unwrap().conn;
    f.conns
        .test(SaveConnReq {
            conn,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(f.host_keys.pending().is_empty());
}

#[tokio::test]
async fn jump_cannot_reach_target() {
    let jump = TestServer::start().await;
    let f = fixture(&[&jump]);
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    let j = f.add("跳板", jump.addr.port(), PASSWORD, "");
    let t = f.add("内网", port, PASSWORD, &j);
    let e = f.try_open(&t).await.unwrap_err();
    assert_eq!(e.msg.i18n, "err.ssh.jumpChannelFailed");
    assert_eq!(text_param(&e, "jump"), "跳板");
    assert_eq!(text_param(&e, "addr"), format!("127.0.0.1:{port}"));
}

#[tokio::test]
async fn jump_disconnect_closes_session() {
    let (jump, target) = (TestServer::start().await, TestServer::start().await);
    let f = fixture(&[&jump, &target]);
    let j = f.add("跳板", jump.addr.port(), PASSWORD, "");
    let t = f.add("内网", target.addr.port(), PASSWORD, &j);
    let (_, mut rx) = f.open_and_echo(&t).await;

    jump.kick_all().await;
    let closed: Closed = tokio::time::timeout(WAIT, async {
        loop {
            match rx.recv().await {
                Some(SessionEvent::Output(_)) => continue,
                Some(SessionEvent::Closed(c)) => return c,
                None => panic!("没有收到 Closed"),
            }
        }
    })
    .await
    .expect("跳板机断开后会话应结束");
    let e = closed.error.expect("应带断开原因");
    assert_eq!(e.i18n, "err.ssh.jumpDisconnected");
    assert_eq!(
        e.params.get("name").map(|p| p.to_string()).as_deref(),
        Some("跳板")
    );
    assert_eq!(f.pool.count(), 0);
}
