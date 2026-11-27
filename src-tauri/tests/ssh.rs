mod common;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rain_dial::core::rdp::cert::CertStore;
use rain_dial::core::ssh::known_hosts::HostKeyStore;
use rain_dial::core::ssh::shell::{ExitStatus, PtySize, ShellEvent, ShellReader};
use rain_dial::core::ssh::{self, Connected, DialOptions, SshError, SshTarget, TrustFn};
use rain_dial::error;
use rain_dial::events::{self, EventSink};
use rain_dial::model::{AuthType, ConnParams, Connection, SaveConnReq, SshParams};
use rain_dial::secrets::{MemoryBackend, Secrets};
use rain_dial::service::conn::ConnService;
use rain_dial::service::hostkey::HostKeyService;
use rain_dial::store::ConnStore;
use russh::Pty;
use russh::keys::ssh_key::LineEnding;
use tokio_util::sync::CancellationToken;

use common::{PASSWORD, TestServer, USER, random_key, temp_dir};

const WAIT: Duration = Duration::from_secs(5);

struct Env {
    dir: PathBuf,
    store: Arc<HostKeyStore>,
}

impl Env {
    fn new() -> Env {
        let dir = temp_dir();
        let store = Arc::new(HostKeyStore::new(dir.join("known_hosts"), vec![]));
        Env { dir, store }
    }

    fn opts(&self, trust: Option<TrustFn>) -> DialOptions {
        DialOptions {
            host_keys: self.store.clone(),
            trust,
            timeout: WAIT,
            keepalive: None,
        }
    }

    /// 自动信任未知主机，并记下询问次数。
    fn trusting(&self, count: &Arc<AtomicUsize>) -> DialOptions {
        self.opts(Some(answer(count.clone(), true)))
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn answer(count: Arc<AtomicUsize>, accept: bool) -> TrustFn {
    Arc::new(move |_q| {
        count.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { accept })
    })
}

fn password_target(srv: &TestServer, password: &str) -> SshTarget {
    SshTarget {
        host: "127.0.0.1".into(),
        port: srv.addr.port(),
        user: USER.into(),
        auth: AuthType::Password,
        password: password.into(),
        ..Default::default()
    }
}

fn key_target(srv: &TestServer, path: &std::path::Path, passphrase: &str) -> SshTarget {
    SshTarget {
        auth: AuthType::Key,
        key_path: path.display().to_string(),
        key_passphrase: passphrase.into(),
        ..password_target(srv, "")
    }
}

async fn dial(t: &SshTarget, opts: &DialOptions) -> Result<Connected, SshError> {
    ssh::connect(t, opts, &CancellationToken::new()).await
}

fn expect_err(r: Result<Connected, SshError>) -> SshError {
    match r {
        Ok(_) => panic!("应连接失败"),
        Err(e) => e,
    }
}

/// 一直读到输出里出现 needle，返回读到的全部输出。
async fn read_until(r: &mut ShellReader, needle: &str) -> String {
    let mut out = Vec::new();
    let found = tokio::time::timeout(WAIT, async {
        loop {
            match r.next().await {
                Some(ShellEvent::Output(d)) => {
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
    assert!(found.is_ok(), "等待 {needle:?} 超时，已收到 {text:?}");
    text
}

async fn wait_exit(r: &mut ShellReader) -> ExitStatus {
    loop {
        match tokio::time::timeout(WAIT, r.next())
            .await
            .expect("等待退出超时")
        {
            Some(ShellEvent::Output(_)) => continue,
            Some(ShellEvent::Exit(s)) => return s,
            None => panic!("没有收到退出状态"),
        }
    }
}

#[tokio::test]
async fn shell_echo_resize_and_exit() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let count = Arc::new(AtomicUsize::new(0));
    let conn = dial(&password_target(&srv, PASSWORD), &env.trusting(&count))
        .await
        .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(
        conn.client
            .server_version()
            .unwrap()
            .starts_with("SSH-2.0-")
    );

    let (w, mut r) = conn
        .client
        .open_shell(PtySize { cols: 80, rows: 24 })
        .await
        .unwrap();
    let (term, modes) = srv.pty.lock().unwrap().clone().unwrap();
    assert_eq!(term, "xterm-256color");
    assert!(modes.contains(&(Pty::IUTF8, 1)) && modes.contains(&(Pty::VERASE, 127)));
    read_until(&mut r, "$ ").await;

    w.write(b"echo hello\r").await.unwrap();
    assert!(
        read_until(&mut r, "hello\r\n$ ")
            .await
            .contains("echo hello\r\nhello\r\n")
    );
    w.write("echo 你好\r".as_bytes()).await.unwrap();
    read_until(&mut r, "你好\r\n$ ").await;

    w.write(b"stty size\r").await.unwrap();
    read_until(&mut r, "\r\n24 80\r\n").await;
    w.resize(PtySize {
        cols: 132,
        rows: 40,
    })
    .await
    .unwrap();
    w.write(b"stty size\r").await.unwrap();
    read_until(&mut r, "\r\n40 132\r\n").await;

    w.write(b"exit 3\r").await.unwrap();
    assert_eq!(wait_exit(&mut r).await, ExitStatus::Code(3));
    assert!(
        tokio::time::timeout(WAIT, r.next())
            .await
            .unwrap()
            .is_none()
    );
    assert!(!conn.client.is_closed(), "关闭一个 shell 不影响 SSH 连接");
    conn.client.close();
    assert!(conn.client.is_closed());
}

#[tokio::test]
async fn shells_share_one_connection() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let count = Arc::new(AtomicUsize::new(0));
    let conn = dial(&password_target(&srv, PASSWORD), &env.trusting(&count))
        .await
        .unwrap();
    let (w1, mut r1) = conn
        .client
        .open_shell(PtySize { cols: 80, rows: 24 })
        .await
        .unwrap();
    let (w2, mut r2) = conn
        .client
        .open_shell(PtySize {
            cols: 100,
            rows: 30,
        })
        .await
        .unwrap();
    w2.write(b"stty size\r").await.unwrap();
    w1.write(b"stty size\r").await.unwrap();
    read_until(&mut r1, "24 80\r\n").await;
    read_until(&mut r2, "30 100\r\n").await;

    w1.close().await;
    w2.write(b"echo still here\r").await.unwrap();
    read_until(&mut r2, "still here\r\n").await;
}

#[tokio::test]
async fn password_denied() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let count = Arc::new(AtomicUsize::new(0));
    let e = expect_err(dial(&password_target(&srv, "wrong"), &env.trusting(&count)).await);
    assert!(e.is_permanent());
    assert_eq!(e.msg().i18n, "err.ssh.authDenied");
}

#[tokio::test]
async fn key_auth() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let count = Arc::new(AtomicUsize::new(0));
    let opts = env.trusting(&count);

    let key = random_key();
    srv.authorize(key.public_key());
    let plain = env.dir.join("id_plain");
    std::fs::write(&plain, key.to_openssh(LineEnding::LF).unwrap().as_bytes()).unwrap();
    let enc = env.dir.join("id_enc");
    let encrypted = key
        .encrypt(&mut russh::keys::key::safe_rng(), "s3cret")
        .unwrap();
    std::fs::write(
        &enc,
        encrypted.to_openssh(LineEnding::LF).unwrap().as_bytes(),
    )
    .unwrap();
    let other = env.dir.join("id_other");
    std::fs::write(
        &other,
        random_key().to_openssh(LineEnding::LF).unwrap().as_bytes(),
    )
    .unwrap();

    dial(&key_target(&srv, &plain, ""), &opts).await.unwrap();
    dial(&key_target(&srv, &enc, "s3cret"), &opts)
        .await
        .unwrap();

    let cases = [
        (key_target(&srv, &enc, ""), "err.ssh.keyEncrypted"),
        (key_target(&srv, &enc, "bad"), "err.ssh.keyDecryptFailed"),
        (key_target(&srv, &other, ""), "err.ssh.authDenied"),
        (
            key_target(&srv, &env.dir.join("missing"), ""),
            "err.ssh.keyReadFailed",
        ),
    ];
    for (t, want) in cases {
        let e = expect_err(dial(&t, &opts).await);
        assert_eq!(e.msg().i18n, want, "{}", t.key_path);
        assert!(e.is_permanent());
    }
}

#[tokio::test]
async fn host_key_trust_reject_and_mismatch() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let t = password_target(&srv, PASSWORD);

    assert!(matches!(
        expect_err(dial(&t, &env.opts(None)).await),
        SshError::HostKeyRejected
    ));
    let count = Arc::new(AtomicUsize::new(0));
    let e = expect_err(dial(&t, &env.opts(Some(answer(count.clone(), false)))).await);
    assert!(matches!(e, SshError::HostKeyRejected));
    assert!(!env.dir.join("known_hosts").exists(), "拒绝后不应记录指纹");

    dial(&t, &env.trusting(&count)).await.unwrap();
    dial(&t, &env.trusting(&count)).await.unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 2, "信任后再连不应再询问");

    let changed = TestServer::start().await;
    env.store
        .add("127.0.0.1", changed.addr.port(), random_key().public_key())
        .unwrap();
    let e = expect_err(dial(&password_target(&changed, PASSWORD), &env.trusting(&count)).await);
    assert!(matches!(e, SshError::HostKeyMismatch { .. }) && e.is_permanent());
}

#[tokio::test]
async fn connect_refused() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let env = Env::new();
    let t = SshTarget {
        host: "127.0.0.1".into(),
        port,
        user: USER.into(),
        ..Default::default()
    };
    let e = expect_err(dial(&t, &env.opts(None)).await);
    assert_eq!(e.msg().i18n, "err.ssh.connectFailed");
}

#[derive(Default)]
struct Recorder {
    events: Mutex<Vec<String>>,
    shown: AtomicUsize,
}

impl EventSink for Recorder {
    fn emit(&self, event: &str, _payload: serde_json::Value) {
        self.events.lock().unwrap().push(event.to_string());
    }
    fn show_main_window(&self) {
        self.shown.fetch_add(1, Ordering::SeqCst);
    }
}

fn ssh_req(srv: &TestServer, password: &str) -> SaveConnReq {
    let params = SshParams {
        host: "127.0.0.1".into(),
        port: srv.addr.port() as i32,
        user: USER.into(),
        password: password.into(),
        ..Default::default()
    };
    SaveConnReq {
        conn: Connection {
            name: "本机".into(),
            params: ConnParams::Ssh(params),
            ..Default::default()
        },
        ..Default::default()
    }
}

#[tokio::test]
async fn conn_test_asks_host_key() {
    let srv = TestServer::start().await;
    let dir = temp_dir();
    let sink = Arc::new(Recorder::default());
    let host_keys = HostKeyService::new(
        HostKeyStore::new(dir.join("known_hosts"), vec![]),
        CertStore::new(dir.join("rdp_known_hosts")),
        sink.clone(),
    );
    let svc = Arc::new(ConnService::new(
        ConnStore::new(dir.join("connections.json")),
        Secrets::new(Box::new(MemoryBackend::default())),
        host_keys.clone(),
        sink.clone(),
    ));

    let task = tokio::spawn({
        let (svc, req) = (svc.clone(), ssh_req(&srv, PASSWORD));
        async move { svc.test(req).await }
    });
    let prompt = tokio::time::timeout(WAIT, async {
        loop {
            if let Some(p) = host_keys.pending().pop() {
                return p;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("应询问指纹");
    assert_eq!(prompt.conn_name, "本机");
    assert!(prompt.fingerprint.starts_with("SHA256:"));
    assert_eq!(
        sink.shown.load(Ordering::SeqCst),
        1,
        "需要确认时应显示主窗口"
    );

    host_keys.answer(&prompt.request_id, true).unwrap();
    let r = task.await.unwrap().unwrap();
    assert!(r.server_version.unwrap().starts_with("SSH-2.0-"));
    assert!(host_keys.pending().is_empty());
    assert_eq!(
        host_keys.answer(&prompt.request_id, true).unwrap_err().code,
        error::HOST_KEY_EXPIRED.0
    );
    let evs = sink.events.lock().unwrap().clone();
    assert!(
        evs.iter().any(|e| e == events::HOST_KEY) && evs.iter().any(|e| e == events::HOST_KEY_DONE)
    );

    // 已信任，不再询问；密码错误时返回具体原因。
    svc.test(ssh_req(&srv, PASSWORD)).await.unwrap();
    let e = svc.test(ssh_req(&srv, "wrong")).await.unwrap_err();
    assert_eq!(
        (e.code, e.msg.i18n),
        (error::SSH_CONNECT_FAILED.0, "err.ssh.authDenied")
    );
    assert_eq!(sink.shown.load(Ordering::SeqCst), 1);
    let _ = std::fs::remove_dir_all(dir);
}
