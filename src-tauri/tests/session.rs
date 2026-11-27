mod common;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rain_dial::core::rdp::cert::CertStore;
use rain_dial::core::ssh::known_hosts::HostKeyStore;
use rain_dial::core::ssh::shell::PtySize;
use rain_dial::error;
use rain_dial::events::NoopSink;
use rain_dial::model::{ConnParams, Connection, SaveConnReq, SshParams};
use rain_dial::secrets::{MemoryBackend, Secrets};
use rain_dial::service::conn::ConnService;
use rain_dial::service::hostkey::HostKeyService;
use rain_dial::service::pool::ClientPool;
use rain_dial::service::session::{Closed, FLUSH_BYTES, MAX_UNACKED, SessionEvent, SessionService};
use rain_dial::store::ConnStore;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use common::{PASSWORD, TestServer, USER, temp_dir};

const WAIT: Duration = Duration::from_secs(10);
const SIZE: PtySize = PtySize { cols: 80, rows: 24 };

struct Fixture {
    svc: Arc<SessionService>,
    pool: Arc<ClientPool>,
    conns: Arc<ConnService>,
    dir: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.svc.close_all();
        self.pool.close_all();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 服务器指纹预先信任，不会弹出确认。
fn fixture(srv: &TestServer) -> Fixture {
    let dir = temp_dir();
    let known = dir.join("known_hosts");
    HostKeyStore::new(known.clone(), vec![])
        .add("127.0.0.1", srv.addr.port(), srv.key.public_key())
        .unwrap();
    let host_keys = HostKeyService::new(
        HostKeyStore::new(known, vec![]),
        CertStore::new(dir.join("rdp_known_hosts")),
        Arc::new(NoopSink),
    );
    let conns = Arc::new(ConnService::new(
        ConnStore::new(dir.join("connections.json")),
        Secrets::new(Box::new(MemoryBackend::default())),
        host_keys,
        Arc::new(NoopSink),
    ));
    let pool = ClientPool::new(conns.clone());
    Fixture {
        svc: SessionService::new(conns.clone(), pool.clone()),
        pool,
        conns,
        dir,
    }
}

impl Fixture {
    /// 保存一个连到 port 的 SSH 连接，返回连接 ID。
    fn add_conn(&self, port: u16, keepalive: i32, init_dir: &str) -> String {
        let params = SshParams {
            host: "127.0.0.1".into(),
            port: port as i32,
            user: USER.into(),
            password: PASSWORD.into(),
            keepalive_seconds: keepalive,
            init_dir: init_dir.into(),
            ..Default::default()
        };
        let conn = Connection {
            name: format!("srv{port}"),
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

    async fn open(&self, conn_id: &str) -> Term {
        self.open_with(conn_id, false).await
    }

    async fn open_with(&self, conn_id: &str, report_cwd: bool) -> Term {
        let (tx, rx) = unbounded_channel();
        let id = self
            .svc
            .open(conn_id, SIZE, report_cwd, Box::new(move |e| drop(tx.send(e))))
            .await
            .unwrap();
        Term {
            id,
            rx,
            svc: self.svc.clone(),
        }
    }
}

/// 测试里的一个终端标签：收集输出，收到后立即确认。
struct Term {
    id: String,
    rx: UnboundedReceiver<SessionEvent>,
    svc: Arc<SessionService>,
}

impl Term {
    async fn send(&self, line: &str) {
        self.svc
            .write(&self.id, format!("{line}\r").as_bytes())
            .await
            .unwrap();
    }

    async fn read_until(&mut self, needle: &str) -> String {
        let mut out = Vec::new();
        let r = tokio::time::timeout(WAIT, async {
            loop {
                match self.rx.recv().await {
                    Some(SessionEvent::Output(d)) => {
                        self.svc.ack(&self.id, d.len());
                        out.extend_from_slice(&d);
                        if String::from_utf8_lossy(&out).contains(needle) {
                            return;
                        }
                    }
                    other => panic!("等待 {needle:?} 时收到 {other:?}"),
                }
            }
        })
        .await;
        let text = String::from_utf8_lossy(&out).to_string();
        assert!(r.is_ok(), "等待 {needle:?} 超时，已收到 {text:?}");
        text
    }

    async fn wait_closed(&mut self) -> Closed {
        tokio::time::timeout(WAIT, async {
            loop {
                match self.rx.recv().await {
                    Some(SessionEvent::Output(_)) => continue,
                    Some(SessionEvent::Closed(c)) => {
                        assert!(self.rx.recv().await.is_none(), "Closed 之后不应再有消息");
                        return c;
                    }
                    None => panic!("没有收到 Closed"),
                }
            }
        })
        .await
        .expect("等待会话结束超时")
    }
}

async fn eventually(what: &str, f: impl Fn() -> bool) {
    let ok = tokio::time::timeout(WAIT, async {
        while !f() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(ok.is_ok(), "{what}");
}

#[tokio::test]
async fn tabs_share_one_connection() {
    let srv = TestServer::start().await;
    let f = fixture(&srv);
    let with_dir = f.add_conn(srv.addr.port(), 30, "/srv/app");

    let (mut a, mut b) = tokio::join!(f.open(&with_dir), f.open(&with_dir));
    assert_eq!(
        srv.accepted.load(Ordering::SeqCst),
        1,
        "同时打开两个标签只建一条连接"
    );
    assert_eq!((f.pool.count(), f.svc.session_count()), (1, 2));

    a.send("pwd").await;
    a.read_until("\r\n/srv/app\r\n").await;
    b.send("stty size").await;
    b.read_until("\r\n24 80\r\n").await;
    f.svc
        .resize(
            &b.id,
            PtySize {
                cols: 120,
                rows: 50,
            },
        )
        .await
        .unwrap();
    b.send("stty size").await;
    b.read_until("\r\n50 120\r\n").await;

    f.svc.close(&a.id).await;
    assert!(
        tokio::time::timeout(WAIT, a.rx.recv())
            .await
            .unwrap()
            .is_none(),
        "主动关闭时不推 Closed"
    );
    let err = f.svc.write(&a.id, b"x").await.unwrap_err();
    assert_eq!(err.code, error::SESSION_NOT_FOUND.0);
    b.send("echo still here").await;
    b.read_until("still here\r\n").await;
    assert_eq!(f.pool.count(), 1);

    f.svc.close(&b.id).await;
    eventually("最后一个标签关闭后断开 SSH 连接", || {
        f.pool.count() == 0
    })
    .await;
    let mut c = f.open(&with_dir).await;
    c.send("echo again").await;
    c.read_until("again\r\n").await;
    assert_eq!(srv.accepted.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn cwd_report_only_when_enabled() {
    let srv = TestServer::start().await;
    let f = fixture(&srv);
    let id = f.add_conn(srv.addr.port(), 30, "/srv/app");

    let mut on = f.open_with(&id, true).await;
    on.send("echo end").await;
    let out = on.read_until("end\r\n").await;
    let (setup, cd) = (out.find("__rt_cwd").unwrap(), out.find("/srv/app").unwrap());
    assert!(setup < cd, "先开启目录报告再进入初始目录：{out:?}");

    let mut off = f.open_with(&id, false).await;
    off.send("echo end").await;
    let out = off.read_until("end\r\n").await;
    assert!(!out.contains("__rt_cwd"), "关闭时不注入：{out:?}");
}

#[tokio::test]
async fn shell_exit_reports_status() {
    let srv = TestServer::start().await;
    let f = fixture(&srv);
    let id = f.add_conn(srv.addr.port(), 30, "");
    let mut t = f.open(&id).await;
    t.read_until("$ ").await;
    t.send("exit 3").await;
    assert_eq!(
        t.wait_closed().await,
        Closed {
            exit_code: Some(3),
            ..Default::default()
        }
    );
    assert_eq!(f.svc.session_count(), 0);
    eventually("没有会话后断开 SSH 连接", || f.pool.count() == 0).await;
}

#[tokio::test]
async fn server_disconnect_closes_all_tabs() {
    let srv = TestServer::start().await;
    let f = fixture(&srv);
    let id = f.add_conn(srv.addr.port(), 30, "");
    let mut a = f.open(&id).await;
    let mut b = f.open(&id).await;
    a.read_until("$ ").await;
    a.send("disconnect").await;
    for t in [&mut a, &mut b] {
        let c = t.wait_closed().await;
        assert_eq!(c.error.map(|e| e.i18n), Some("err.ssh.serverClosed"));
    }
    assert_eq!((f.pool.count(), f.svc.session_count()), (0, 0));

    let mut c = f.open(&id).await;
    c.send("echo back").await;
    c.read_until("back\r\n").await;
}

#[tokio::test]
async fn output_batched_and_flow_controlled() {
    const TOTAL: usize = 16 * 1024 * 1024;
    let srv = TestServer::start().await;
    let f = fixture(&srv);
    let id = f.add_conn(srv.addr.port(), 30, "");
    let mut t = f.open(&id).await;
    t.read_until("$ ").await;
    t.send(&format!("flood {TOTAL}")).await;

    // 不确认：发出 MAX_UNACKED 左右就停下。
    let (mut unacked, mut chunks, mut got) = (0usize, 0usize, 0usize);
    loop {
        match tokio::time::timeout(Duration::from_millis(500), t.rx.recv()).await {
            Ok(Some(SessionEvent::Output(d))) => {
                assert!(
                    d.len() < FLUSH_BYTES * 2,
                    "单次发送 {} 字节，应合并到 {FLUSH_BYTES} 左右",
                    d.len()
                );
                unacked += d.len();
                got += d.iter().filter(|&&b| b == b'x').count();
                chunks += 1;
            }
            Ok(other) => panic!("收到 {other:?}"),
            Err(_) => break,
        }
    }
    assert!(
        unacked >= MAX_UNACKED && unacked < MAX_UNACKED + FLUSH_BYTES * 2,
        "暂停时未确认 {unacked} 字节"
    );

    assert!(
        chunks <= MAX_UNACKED / FLUSH_BYTES + 1,
        "暂停前发了 {chunks} 条，应合并成约 32KB 一条"
    );

    // 确认后继续，直到全部收完。
    f.svc.ack(&t.id, unacked);
    let mut tail = Vec::new();
    tokio::time::timeout(Duration::from_secs(60), async {
        while !String::from_utf8_lossy(&tail).contains("done") {
            match t.rx.recv().await {
                Some(SessionEvent::Output(d)) => {
                    f.svc.ack(&t.id, d.len());
                    got += d.iter().filter(|&&b| b == b'x').count();
                    tail.extend_from_slice(&d[d.len().saturating_sub(16)..]);
                    tail.drain(..tail.len().saturating_sub(16));
                }
                other => panic!("收到 {other:?}"),
            }
        }
    })
    .await
    .expect("接收大量输出超时");
    assert_eq!(got, TOTAL);
}

/// 转发到 target 的 TCP 代理；freeze 后不再转发任何数据，模拟网络中断。
async fn freezable_proxy(target: SocketAddr) -> (u16, Arc<AtomicBool>) {
    let ln = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = ln.local_addr().unwrap().port();
    let frozen = Arc::new(AtomicBool::new(false));
    let fz = frozen.clone();
    tokio::spawn(async move {
        while let Ok((inbound, _)) = ln.accept().await {
            let outbound = TcpStream::connect(target).await.unwrap();
            let (ri, wi) = inbound.into_split();
            let (ro, wo) = outbound.into_split();
            tokio::spawn(copy(ri, wo, fz.clone()));
            tokio::spawn(copy(ro, wi, fz.clone()));
        }
    });
    (port, frozen)
}

async fn copy(
    mut r: tokio::net::tcp::OwnedReadHalf,
    mut w: tokio::net::tcp::OwnedWriteHalf,
    frozen: Arc<AtomicBool>,
) {
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        while frozen.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        match r.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                while frozen.load(Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                if w.write_all(&buf[..n]).await.is_err() {
                    return;
                }
            }
        }
    }
}

#[tokio::test]
async fn keepalive_timeout_detected() {
    let srv = TestServer::start().await;
    let f = fixture(&srv);
    let (port, frozen) = freezable_proxy(srv.addr).await;
    HostKeyStore::new(f.dir.join("known_hosts"), vec![])
        .add("127.0.0.1", port, srv.key.public_key())
        .unwrap();
    let id = f.add_conn(port, 1, "");
    let mut t = f.open(&id).await;
    t.send("echo ok").await;
    t.read_until("ok\r\n").await;

    frozen.store(true, Ordering::SeqCst);
    let start = std::time::Instant::now();
    let c = t.wait_closed().await;
    assert_eq!(c.error.map(|e| e.i18n), Some("err.ssh.keepaliveTimeout"));
    assert!(
        start.elapsed() < Duration::from_secs(8),
        "保活 1 秒、最多 3 次，应在几秒内发现断开"
    );
    assert_eq!(f.pool.count(), 0);
}
