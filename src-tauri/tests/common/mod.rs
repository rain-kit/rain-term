//! 集成测试共用的进程内 SSH 服务器。shell 是模拟的，认识的命令见 [`ServerHandler::run`]；
//! SFTP 子系统见 [`sftp_fs`]。
#![allow(dead_code)]

pub mod sftp_fs;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::keys::{Algorithm, PrivateKey, PublicKey};
use russh::server::{self, Auth, ChannelOpenHandle, Msg, Session, VerifiedPublicKey};
use russh::{Channel, ChannelId, Disconnect, Pty};
use tokio::io::copy_bidirectional;
use tokio::net::{TcpListener, TcpStream};

pub const USER: &str = "tester";
pub const PASSWORD: &str = "pa55";

pub struct TestServer {
    pub addr: SocketAddr,
    pub key: PrivateKey,
    /// 允许登录的公钥。
    authorized: Arc<Mutex<Vec<PublicKey>>>,
    /// 最近一次 PTY 请求的终端类型和模式。
    pub pty: Arc<Mutex<Option<(String, Vec<(Pty, u32)>)>>>,
    /// 接受过的 TCP 连接数。
    pub accepted: Arc<AtomicUsize>,
    handles: Arc<Mutex<Vec<server::Handle>>>,
    /// SFTP 的 / 对应的本地目录。
    pub sftp_root: PathBuf,
    /// 为 true 时拒绝 sftp 子系统请求。
    pub sftp_off: Arc<AtomicBool>,
    /// SFTP 每个读写请求额外等待的毫秒数。
    pub sftp_delay_ms: Arc<AtomicU64>,
}

impl TestServer {
    pub async fn start() -> TestServer {
        Self::start_with_key(random_key()).await
    }

    pub async fn start_with_key(key: PrivateKey) -> TestServer {
        Self::start_on("127.0.0.1:0", key).await
    }

    pub async fn start_on(bind: &str, key: PrivateKey) -> TestServer {
        let cfg = Arc::new(server::Config {
            keys: vec![key.clone()],
            auth_rejection_time: Duration::from_millis(10),
            auth_rejection_time_initial: Some(Duration::ZERO),
            ..Default::default()
        });
        let ln = TcpListener::bind(bind).await.unwrap();
        let addr = ln.local_addr().unwrap();
        let authorized = Arc::new(Mutex::new(Vec::new()));
        let pty = Arc::new(Mutex::new(None));
        let accepted = Arc::new(AtomicUsize::new(0));
        let handles = Arc::new(Mutex::new(Vec::new()));
        let sftp_root = temp_dir();
        std::fs::create_dir_all(sftp_root.join(sftp_fs::HOME.trim_start_matches('/'))).unwrap();
        let sftp_off = Arc::new(AtomicBool::new(false));
        let sftp_delay_ms = Arc::new(AtomicU64::new(0));
        let (auth, p, n, hs) = (
            authorized.clone(),
            pty.clone(),
            accepted.clone(),
            handles.clone(),
        );
        let (root, off, delay) = (sftp_root.clone(), sftp_off.clone(), sftp_delay_ms.clone());
        tokio::spawn(async move {
            while let Ok((tcp, _)) = ln.accept().await {
                n.fetch_add(1, Ordering::SeqCst);
                let h = ServerHandler {
                    authorized: auth.clone(),
                    pty: p.clone(),
                    shells: HashMap::new(),
                    channels: HashMap::new(),
                    sftp_root: root.clone(),
                    sftp_off: off.clone(),
                    sftp_delay_ms: delay.clone(),
                };
                let (cfg, hs) = (cfg.clone(), hs.clone());
                tokio::spawn(async move {
                    if let Ok(rs) = server::run_stream(cfg, tcp, h).await {
                        hs.lock().unwrap().push(rs.handle());
                        let _ = rs.await;
                    }
                });
            }
        });
        TestServer {
            addr,
            key,
            authorized,
            pty,
            accepted,
            handles,
            sftp_root,
            sftp_off,
            sftp_delay_ms,
        }
    }

    /// 远程路径对应的本地文件。
    pub fn remote_file(&self, path: &str) -> PathBuf {
        self.sftp_root.join(path.trim_start_matches('/'))
    }

    pub fn authorize(&self, key: &PublicKey) {
        self.authorized.lock().unwrap().push(key.clone());
    }

    /// 模拟服务器主动断开所有连接。
    pub async fn kick_all(&self) {
        let hs: Vec<_> = self.handles.lock().unwrap().drain(..).collect();
        for h in hs {
            let _ = h
                .disconnect(Disconnect::ByApplication, "bye".into(), String::new())
                .await;
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.sftp_root);
    }
}

pub fn random_key() -> PrivateKey {
    PrivateKey::random(&mut russh::keys::key::safe_rng(), Algorithm::Ed25519).unwrap()
}

#[derive(Default)]
struct FakeShell {
    cols: u32,
    rows: u32,
    line: Vec<u8>,
    cwd: String,
}

struct ServerHandler {
    authorized: Arc<Mutex<Vec<PublicKey>>>,
    pty: Arc<Mutex<Option<(String, Vec<(Pty, u32)>)>>>,
    shells: HashMap<ChannelId, FakeShell>,
    /// 还没决定用途（shell 或 sftp）的 session channel。
    channels: HashMap<ChannelId, Channel<Msg>>,
    sftp_root: PathBuf,
    sftp_off: Arc<AtomicBool>,
    sftp_delay_ms: Arc<AtomicU64>,
}

impl ServerHandler {
    /// 执行一行命令；返回 false 表示 shell 已退出。
    /// 认识：echo、stty size、cd（参数可带单引号）、pwd、flood N（输出 N 个 x 后输出 done）、
    /// lines N（输出约 N 字节带编号的文本行后输出 done）、exit N、disconnect（服务器断开整条连接）。
    fn run(
        &mut self,
        ch: ChannelId,
        line: &str,
        session: &mut Session,
    ) -> Result<bool, russh::Error> {
        let sh = self.shells.get_mut(&ch).unwrap();
        let out = if let Some(text) = line.strip_prefix("echo ") {
            format!("{text}\r\n")
        } else if line == "stty size" {
            format!("{} {}\r\n", sh.rows, sh.cols)
        } else if let Some(dir) = line.strip_prefix("cd ") {
            sh.cwd = dir.replace('\'', "");
            String::new()
        } else if line == "pwd" {
            format!(
                "{}\r\n",
                if sh.cwd.is_empty() {
                    "/home/tester"
                } else {
                    &sh.cwd
                }
            )
        } else if let Some(n) = line.strip_prefix("flood ") {
            let mut left: usize = n.trim().parse().unwrap_or(0);
            let h = session.handle();
            tokio::spawn(async move {
                while left > 0 {
                    let k = left.min(16 * 1024);
                    if h.data(ch, vec![b'x'; k]).await.is_err() {
                        return;
                    }
                    left -= k;
                }
                let _ = h.data(ch, b"\r\ndone\r\n$ ".to_vec()).await;
            });
            return Ok(true);
        } else if let Some(n) = line.strip_prefix("lines ") {
            let total: usize = n.trim().parse().unwrap_or(0);
            let h = session.handle();
            tokio::spawn(async move {
                let (mut sent, mut i) = (0usize, 0u64);
                while sent < total {
                    let mut chunk = String::with_capacity(16 * 1024);
                    while chunk.len() < 16 * 1024 {
                        i += 1;
                        chunk.push_str(&format!(
                            "{i:08} The quick brown fox jumps over the lazy dog 中文宽度 🙂\r\n"
                        ));
                    }
                    sent += chunk.len();
                    if h.data(ch, chunk.into_bytes()).await.is_err() {
                        return;
                    }
                }
                let _ = h.data(ch, b"done\r\n$ ".to_vec()).await;
            });
            return Ok(true);
        } else if line == "disconnect" {
            session.disconnect(Disconnect::ByApplication, "bye", "")?;
            return Ok(false);
        } else if let Some(code) = line.strip_prefix("exit ") {
            session.exit_status_request(ch, code.trim().parse().unwrap_or(0))?;
            session.eof(ch)?;
            session.close(ch)?;
            return Ok(false);
        } else if line.is_empty() {
            String::new()
        } else {
            format!("{line}: command not found\r\n")
        };
        session.data(ch, format!("{out}$ "))?;
        Ok(true)
    }
}

impl server::Handler for ServerHandler {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if user == USER && password == PASSWORD {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        key: VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        let ok = user == USER
            && self
                .authorized
                .lock()
                .unwrap()
                .iter()
                .any(|k| k.key_data() == key.public_key.key_data());
        Ok(if ok { Auth::Accept } else { Auth::reject() })
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.shells.insert(channel.id(), FakeShell::default());
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        ch: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let channel = self.channels.remove(&ch);
        self.shells.remove(&ch);
        match channel {
            Some(channel) if name == "sftp" && !self.sftp_off.load(Ordering::SeqCst) => {
                session.channel_success(ch)?;
                let fs = sftp_fs::FakeFs::new(self.sftp_root.clone(), self.sftp_delay_ms.clone());
                russh_sftp::server::run(channel.into_stream(), fs).await;
            }
            _ => session.channel_failure(ch)?,
        }
        Ok(())
    }

    /// 充当跳板机：连接目标地址，在 channel 和 TCP 之间转发数据。
    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host: &str,
        port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        match TcpStream::connect((host, port as u16)).await {
            Ok(mut tcp) => {
                reply.accept().await;
                tokio::spawn(async move {
                    let mut ch = channel.into_stream();
                    let _ = copy_bidirectional(&mut tcp, &mut ch).await;
                });
            }
            Err(_) => reply.reject(russh::ChannelOpenFailure::ConnectFailed).await,
        }
        Ok(())
    }

    async fn pty_request(
        &mut self,
        ch: ChannelId,
        term: &str,
        cols: u32,
        rows: u32,
        _pix_width: u32,
        _pix_height: u32,
        modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        *self.pty.lock().unwrap() = Some((term.to_string(), modes.to_vec()));
        if let Some(sh) = self.shells.get_mut(&ch) {
            sh.cols = cols;
            sh.rows = rows;
        }
        session.channel_success(ch)?;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        ch: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // shell 的输入走 data 回调，channel 本身不再读，留着会塞满它的缓冲。
        self.channels.remove(&ch);
        session.channel_success(ch)?;
        session.data(ch, "welcome\r\n$ ")?;
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        ch: ChannelId,
        cols: u32,
        rows: u32,
        _pix_width: u32,
        _pix_height: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(sh) = self.shells.get_mut(&ch) {
            sh.cols = cols;
            sh.rows = rows;
        }
        Ok(())
    }

    /// 模拟终端的回显和行编辑：回车执行，DEL 删除前一个字符。
    async fn data(
        &mut self,
        ch: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        for &b in data {
            let Some(sh) = self.shells.get_mut(&ch) else {
                return Ok(());
            };
            match b {
                b'\r' | b'\n' => {
                    let line = String::from_utf8_lossy(&std::mem::take(&mut sh.line)).to_string();
                    session.data(ch, "\r\n")?;
                    if !self.run(ch, &line, session)? {
                        self.shells.remove(&ch);
                        return Ok(());
                    }
                }
                0x7f => {
                    if sh.line.pop().is_some() {
                        session.data(ch, "\x08 \x08")?;
                    }
                }
                _ => {
                    sh.line.push(b);
                    session.data(ch, vec![b])?;
                }
            }
        }
        Ok(())
    }
}

pub fn temp_dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("rainterm-it-{}", rain_term::util::new_id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 可重复的伪随机内容。
pub fn pattern(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

pub fn write(path: &Path, data: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, data).unwrap();
}

pub fn assert_same_tree(a: &Path, b: &Path) {
    let names = |p: &Path| {
        let mut v: Vec<_> = std::fs::read_dir(p)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        v.sort();
        v
    };
    assert_eq!(
        names(a),
        names(b),
        "{} 与 {} 的文件列表不同",
        a.display(),
        b.display()
    );
    for n in names(a) {
        let (x, y) = (a.join(&n), b.join(&n));
        if x.is_dir() {
            assert!(y.is_dir());
            assert_same_tree(&x, &y);
        } else {
            assert!(
                std::fs::read(&x).unwrap() == std::fs::read(&y).unwrap(),
                "{} 内容不同",
                y.display()
            );
        }
    }
}

/// 启动一个内存里的 ssh-agent，加入给定的密钥，返回 SSH_AUTH_SOCK 应设置的值。
#[cfg(windows)]
pub async fn start_agent(keys: &[PrivateKey]) -> String {
    use tokio::net::windows::named_pipe::ServerOptions;
    let name = format!(
        r"\\.\pipe\rain-term-test-agent-{}",
        rain_term::util::new_id()
    );
    let first = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&name)
        .unwrap();
    let n = name.clone();
    let incoming = futures::stream::unfold(first, move |server| {
        let n = n.clone();
        async move {
            server.connect().await.ok()?;
            let next = ServerOptions::new().create(&n).ok()?;
            Some((Ok::<_, std::io::Error>(server), next))
        }
    });
    tokio::spawn(russh::keys::agent::server::serve(Box::pin(incoming), ()));
    let mut client = russh::keys::agent::client::AgentClient::connect_named_pipe(&name)
        .await
        .unwrap();
    for k in keys {
        client.add_identity(k, &[]).await.unwrap();
    }
    name
}

#[cfg(unix)]
pub async fn start_agent(keys: &[PrivateKey]) -> String {
    let path = temp_dir().join("agent.sock");
    let ln = tokio::net::UnixListener::bind(&path).unwrap();
    let incoming = futures::stream::unfold(ln, |ln| async move {
        let r = ln.accept().await.map(|(s, _)| s);
        Some((r, ln))
    });
    tokio::spawn(russh::keys::agent::server::serve(Box::pin(incoming), ()));
    let mut client = russh::keys::agent::client::AgentClient::connect_uds(&path)
        .await
        .unwrap();
    for k in keys {
        client.add_identity(k, &[]).await.unwrap();
    }
    path.display().to_string()
}
