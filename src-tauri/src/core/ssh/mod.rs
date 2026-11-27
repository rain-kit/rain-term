pub mod auth;
pub mod jump;
pub mod known_hosts;
pub mod proxy;
pub mod sftp;
pub mod shell;

use std::borrow::Cow;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use russh::client::{self, DisconnectReason, Handle};
use russh::keys::PublicKeyOrCertificate;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::time::Instant;
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};

use crate::i18n::Msg;
use crate::model::{AuthType, SshParams};
use crate::util::join_host_port;
use known_hosts::{Check, HostKeyStore, fingerprint, normalize};

/// TCP 连接加 SSH 握手、认证的默认超时，等用户确认指纹的时间不计入。
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);
/// 连续这么多次保活无响应就判定断开。
const KEEPALIVE_MAX: usize = 3;

#[derive(Debug, thiserror::Error)]
pub enum SshError {
    /// 重试也不会成功（认证失败、私钥错误等），不应自动重连。
    #[error("{}", .0.text())]
    Permanent(Msg),
    #[error("用户拒绝信任该主机指纹")]
    HostKeyRejected,
    #[error("{}", mismatch(.host, .got, .expected).text())]
    HostKeyMismatch {
        host: String,
        got: String,
        expected: String,
    },
    #[error("已取消")]
    Cancelled,
    #[error("{}", .0.text())]
    Other(Msg),
    #[error("{}", describe(.0).text())]
    Russh(#[from] russh::Error),
}

impl SshError {
    pub fn is_permanent(&self) -> bool {
        matches!(
            self,
            SshError::Permanent(_) | SshError::HostKeyRejected | SshError::HostKeyMismatch { .. }
        )
    }

    /// 返回给前端的消息。
    pub fn msg(&self) -> Msg {
        match self {
            SshError::Permanent(m) | SshError::Other(m) => m.clone(),
            SshError::HostKeyRejected => msg!("err.ssh.hostKeyRejected", "用户拒绝信任该主机指纹"),
            SshError::HostKeyMismatch {
                host,
                got,
                expected,
            } => mismatch(host, got, expected),
            SshError::Cancelled => msg!("err.ssh.cancelled", "已取消"),
            SshError::Russh(e) => describe(e),
        }
    }
}

fn mismatch(host: &str, got: &str, expected: &str) -> Msg {
    msg!(
        "err.ssh.hostKeyMismatch",
        "主机 {host} 的指纹与已记录的不一致（收到 {got}，记录为 {expected}），可能存在中间人攻击",
        host,
        got,
        expected
    )
}

pub(crate) fn describe(e: &russh::Error) -> Msg {
    use russh::Error as E;
    match e {
        E::KeepaliveTimeout => msg!(
            "err.ssh.keepaliveTimeout",
            "连续多个周期保活无响应，连接已断开"
        ),
        E::HUP | E::Disconnect => msg!("err.ssh.serverClosed", "服务器关闭了连接"),
        E::ConnectionTimeout => msg!("err.ssh.timeout", "连接超时"),
        E::IO(io) => msg!("err.ssh.connectionLost", "连接断开").with_cause(io),
        E::NoCommonAlgo { kind, .. } => {
            msg!(
                "err.ssh.noCommonAlgo",
                "与服务器没有共同支持的 {kind} 算法",
                kind = format!("{kind:?}")
            )
        }
        E::Version => msg!(
            "err.ssh.notSshServer",
            "对方不是 SSH 服务器（版本标识无效）"
        ),
        other => msg!("err.ssh.other", "SSH 错误").with_cause(other),
    }
}

/// 建立 SSH 连接需要的信息；终端会话、SFTP 和跳板机共用。
#[derive(Debug, Clone, Default)]
pub struct SshTarget {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: AuthType,
    pub key_path: String,
    pub password: String,
    pub key_passphrase: String,
}

impl SshTarget {
    /// 端口已由连接校验保证在 1 ~ 65535 之间。
    pub fn from_params(p: &SshParams) -> Self {
        SshTarget {
            host: p.host.clone(),
            port: p.port as u16,
            user: p.user.clone(),
            auth: p.auth,
            key_path: p.key_path.clone(),
            password: p.password.clone(),
            key_passphrase: p.key_passphrase.clone(),
        }
    }

    pub fn addr(&self) -> String {
        join_host_port(&self.host, self.port)
    }
}

/// 遇到未知主机时询问用户是否信任；cancel 取消时应尽快返回 false。
pub struct HostKeyQuery {
    pub host: String,
    pub key_type: String,
    pub fingerprint: String,
    pub cancel: CancellationToken,
}

pub type TrustFn =
    Arc<dyn Fn(HostKeyQuery) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

pub struct DialOptions {
    pub host_keys: Arc<HostKeyStore>,
    /// 为 None 时未知主机一律拒绝。
    pub trust: Option<TrustFn>,
    pub timeout: Duration,
    pub keepalive: Option<Duration>,
}

/// 握手超时计时器：等用户确认指纹的时间不计入。
#[derive(Default)]
struct PauseClock {
    state: Mutex<(Duration, Option<Instant>)>,
}

impl PauseClock {
    fn pause(&self) {
        self.state.lock().unwrap().1 = Some(Instant::now());
    }

    fn resume(&self) {
        let mut s = self.state.lock().unwrap();
        if let Some(since) = s.1.take() {
            s.0 += since.elapsed();
        }
    }

    /// 从 start 起实际计时的时长（扣掉暂停的时间）。
    fn active(&self, start: Instant) -> Duration {
        start.elapsed().saturating_sub(self.state.lock().unwrap().0)
    }

    async fn expired(&self, start: Instant, timeout: Duration) {
        loop {
            let (paused, since) = *self.state.lock().unwrap();
            if since.is_some() {
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
            let elapsed = start.elapsed().saturating_sub(paused);
            if elapsed >= timeout {
                return;
            }
            tokio::time::sleep(timeout - elapsed).await;
        }
    }
}

pub struct ClientHandler {
    host: String,
    port: u16,
    store: Arc<HostKeyStore>,
    trust: Option<TrustFn>,
    cancel: CancellationToken,
    clock: Arc<PauseClock>,
    disconnect_tx: Option<oneshot::Sender<Msg>>,
}

impl client::Handler for ClientHandler {
    type Error = SshError;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, SshError> {
        let key = key.public_key();
        let _guard = self.store.lock().await;
        let name = normalize(&self.host, self.port);
        match self.store.check(&self.host, self.port, &key) {
            Err(e) => Err(SshError::Permanent(
                msg!("err.ssh.knownHostsReadFailed", "读取 known_hosts 失败").with_cause(e),
            )),
            Ok(Check::Known) => Ok(true),
            Ok(Check::Mismatch(expected)) => Err(SshError::HostKeyMismatch {
                host: name,
                got: fingerprint(&key),
                expected: expected.join(", "),
            }),
            Ok(Check::Unknown) => {
                let Some(trust) = &self.trust else {
                    return Err(SshError::HostKeyRejected);
                };
                let query = HostKeyQuery {
                    host: name,
                    key_type: key.algorithm().as_str().to_string(),
                    fingerprint: fingerprint(&key),
                    cancel: self.cancel.clone(),
                };
                self.clock.pause();
                let ok = trust(query).await;
                self.clock.resume();
                if !ok {
                    return Err(SshError::HostKeyRejected);
                }
                self.store.add(&self.host, self.port, &key).map_err(|e| {
                    SshError::Permanent(
                        msg!("err.ssh.knownHostsWriteFailed", "写入 known_hosts 失败")
                            .with_cause(e),
                    )
                })?;
                Ok(true)
            }
        }
    }

    async fn disconnected(&mut self, reason: DisconnectReason<SshError>) -> Result<(), SshError> {
        let msg = match &reason {
            DisconnectReason::ReceivedDisconnect(info) if info.message.is_empty() => {
                msg!("err.ssh.serverClosed", "服务器关闭了连接")
            }
            DisconnectReason::ReceivedDisconnect(info) => {
                msg!("err.ssh.serverClosed", "服务器关闭了连接").with_cause(&info.message)
            }
            DisconnectReason::Error(e) => e.msg(),
        };
        if let Some(tx) = self.disconnect_tx.take() {
            let _ = tx.send(msg);
        }
        match reason {
            DisconnectReason::ReceivedDisconnect(_) => Ok(()),
            DisconnectReason::Error(e) => Err(e),
        }
    }
}

/// 从连接开头的数据里找出服务器的版本标识（如 SSH-2.0-OpenSSH_9.6）。
#[derive(Default)]
struct Banner {
    buf: Vec<u8>,
    line: Option<String>,
}

impl Banner {
    /// 返回 true 表示不用再看后面的数据了。
    fn feed(&mut self, data: &[u8]) -> bool {
        self.buf.extend_from_slice(data);
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line).trim().to_string();
            if line.starts_with("SSH-") {
                self.line = Some(line);
                self.buf = Vec::new();
                return true;
            }
        }
        if self.buf.len() > 1024 {
            self.buf = Vec::new();
            return true;
        }
        false
    }
}

/// 包一层底层连接（TCP，或跳板机上的 direct-tcpip channel）：kill 取消后读写立即报错，SSH 会话随之结束。
/// 用来在握手卡住、用户关闭会话时可靠地关闭连接（russh 的会话跑在独立任务里）。
struct Killable<S> {
    inner: S,
    killed: Pin<Box<WaitForCancellationFutureOwned>>,
    /// 读到版本标识后置为 None。
    banner: Option<Arc<Mutex<Banner>>>,
}

impl<S> Killable<S> {
    fn check(&mut self, cx: &mut Context<'_>) -> io::Result<()> {
        match self.killed.as_mut().poll(cx) {
            Poll::Ready(()) => Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "连接已关闭",
            )),
            Poll::Pending => Ok(()),
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Killable<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.check(cx)?;
        let before = buf.filled().len();
        let r = Pin::new(&mut this.inner).poll_read(cx, buf);
        if let (Poll::Ready(Ok(())), Some(banner)) = (&r, &this.banner)
            && buf.filled().len() > before
            && banner.lock().unwrap().feed(&buf.filled()[before..])
        {
            this.banner = None;
        }
        r
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Killable<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.check(cx)?;
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

pub struct SshClient {
    handle: Handle<ClientHandler>,
    kill: CancellationToken,
    banner: Arc<Mutex<Banner>>,
    /// 经跳板机连接时的上一级连接，只供这条连接使用，随它一起关闭。
    via: Option<Arc<SshClient>>,
}

/// 一次成功的连接：客户端句柄和断开通知。
pub struct Connected {
    pub client: Arc<SshClient>,
    pub disconnected: oneshot::Receiver<Msg>,
    /// 从发起 TCP 连接到认证完成的耗时，不含等待用户确认指纹的时间。
    pub elapsed: Duration,
}

impl SshClient {
    /// 服务器的版本标识，例如 SSH-2.0-OpenSSH_9.6。
    pub fn server_version(&self) -> Option<String> {
        self.banner.lock().unwrap().line.clone()
    }

    pub fn close(&self) {
        self.kill.cancel();
        if let Some(v) = &self.via {
            v.close();
        }
    }

    pub fn is_closed(&self) -> bool {
        self.kill.is_cancelled()
            || self.handle.is_closed()
            || self.via.as_ref().is_some_and(|v| v.is_closed())
    }

    /// 在一条一次性 channel 上执行命令，丢弃输出，等命令结束。用于无痕模式清理历史。
    pub async fn exec(&self, command: &str) -> Result<(), SshError> {
        let mut channel = self.handle.channel_open_session().await?;
        channel.exec(false, command.as_bytes()).await?;
        while let Some(msg) = channel.wait().await {
            if matches!(msg, russh::ChannelMsg::Close) {
                break;
            }
        }
        Ok(())
    }

    /// 在一条一次性 channel 上执行命令，把 input 写到它的标准输入，等命令结束后返回标准输出。
    /// 输出超过 limit 字节的部分丢弃；标准错误丢弃。
    pub async fn exec_output(
        &self,
        command: &str,
        input: &[u8],
        limit: usize,
    ) -> Result<Vec<u8>, SshError> {
        let mut channel = self.handle.channel_open_session().await?;
        channel.exec(false, command.as_bytes()).await?;
        channel.data(input).await?;
        channel.eof().await?;
        let mut out = Vec::new();
        while let Some(msg) = channel.wait().await {
            match msg {
                russh::ChannelMsg::Data { data } => {
                    let n = data.len().min(limit - out.len());
                    out.extend_from_slice(&data[..n]);
                }
                russh::ChannelMsg::Close => break,
                _ => {}
            }
        }
        Ok(out)
    }

    /// 通过 direct-tcpip 让服务器连接 t，在这条 channel 上建立到 t 的 SSH 连接；jump 是本连接在提示里的名称。
    /// 返回的连接持有本连接，关闭时一起关闭。
    pub async fn connect_via(
        self: &Arc<Self>,
        jump: &str,
        t: &SshTarget,
        opts: &DialOptions,
        cancel: &CancellationToken,
    ) -> Result<Connected, SshError> {
        let method = auth::prepare(t).await?;
        let begin = Instant::now();
        let stream = self
            .open_tunnel(jump, &t.host, t.port, opts.timeout, cancel)
            .await?;
        handshake(
            stream,
            t,
            method,
            opts,
            cancel,
            begin,
            Some(self.clone()),
        )
        .await
    }

    /// 通过 direct-tcpip 让服务器连接 host:port，返回这条 channel 上的字节流；jump 是本连接在提示里的名称。
    pub async fn open_tunnel(
        &self,
        jump: &str,
        host: &str,
        port: u16,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<russh::ChannelStream<client::Msg>, SshError> {
        let addr = join_host_port(host, port);
        let open = self
            .handle
            .channel_open_direct_tcpip(host, port as u32, "127.0.0.1", 0);
        let channel = tokio::select! {
            _ = cancel.cancelled() => return Err(SshError::Cancelled),
            r = tokio::time::timeout(timeout, open) => match r {
                Ok(Ok(ch)) => ch,
                Ok(Err(russh::Error::ChannelOpenFailure(f))) => {
                    return Err(SshError::Other(
                        msg!("err.ssh.jumpChannelFailed", "跳板机 {jump} 无法连接到 {addr}", jump, addr).with_cause(f.description()),
                    ));
                }
                Ok(Err(e)) => {
                    return Err(SshError::Other(msg!("err.ssh.jumpChannelFailed", "跳板机 {jump} 无法连接到 {addr}", jump, addr).with_cause(e)));
                }
                Err(_) => return Err(SshError::Other(msg!("err.ssh.jumpChannelTimeout", "跳板机 {jump} 连接 {addr} 超时", jump, addr))),
            },
        };
        Ok(channel.into_stream())
    }
}

impl Drop for SshClient {
    fn drop(&mut self) {
        self.kill.cancel();
    }
}

/// 建立 SSH 连接并完成认证；cancel 取消会中断 TCP 连接和握手。
pub async fn connect(
    t: &SshTarget,
    opts: &DialOptions,
    cancel: &CancellationToken,
) -> Result<Connected, SshError> {
    let method = auth::prepare(t).await?;
    let addr = t.addr();
    let begin = Instant::now();

    let tcp = tokio::select! {
        _ = cancel.cancelled() => return Err(SshError::Cancelled),
        r = tokio::time::timeout(opts.timeout, TcpStream::connect((t.host.as_str(), t.port))) => match r {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return Err(SshError::Other(msg!("err.ssh.connectFailed", "连接 {addr} 失败", addr).with_cause(e))),
            Err(_) => return Err(SshError::Other(msg!("err.ssh.connectTimeout", "连接 {addr} 超时", addr))),
        },
    };
    let _ = tcp.set_nodelay(true);
    handshake(tcp, t, method, opts, cancel, begin, None).await
}

/// 在已建立的底层连接上完成 SSH 握手和认证；begin 是开始建立底层连接的时间，用于统计耗时。
async fn handshake<S>(
    stream: S,
    t: &SshTarget,
    method: auth::AuthMethod,
    opts: &DialOptions,
    cancel: &CancellationToken,
    begin: Instant,
    via: Option<Arc<SshClient>>,
) -> Result<Connected, SshError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let addr = t.addr();
    let kill = cancel.child_token();
    let guard = kill.clone().drop_guard();
    let clock = Arc::new(PauseClock::default());
    let (disconnect_tx, disconnected) = oneshot::channel();
    let handler = ClientHandler {
        host: t.host.clone(),
        port: t.port,
        store: opts.host_keys.clone(),
        trust: opts.trust.clone(),
        cancel: kill.clone(),
        clock: clock.clone(),
        disconnect_tx: Some(disconnect_tx),
    };

    let mut config = client::Config {
        keepalive_interval: opts.keepalive,
        keepalive_max: KEEPALIVE_MAX,
        nodelay: true,
        ..Default::default()
    };
    let algos = opts.host_keys.known_algorithms(&t.host, t.port);
    if !algos.is_empty() {
        config.preferred.key = Cow::Owned(algos);
    }

    let banner = Arc::new(Mutex::new(Banner::default()));
    let stream = Killable {
        inner: stream,
        killed: Box::pin(kill.clone().cancelled_owned()),
        banner: Some(banner.clone()),
    };
    let start = Instant::now();
    let login = async {
        let mut handle = client::connect_stream(Arc::new(config), stream, handler).await?;
        auth::authenticate(&mut handle, &t.user, method).await?;
        Ok::<_, SshError>(handle)
    };
    let handle = tokio::select! {
        r = login => r?,
        _ = cancel.cancelled() => return Err(SshError::Cancelled),
        _ = clock.expired(start, opts.timeout) => {
            return Err(SshError::Other(msg!("err.ssh.handshakeTimeout", "与 {addr} 的 SSH 握手超时", addr)));
        }
    };
    guard.disarm();
    Ok(Connected {
        client: Arc::new(SshClient {
            handle,
            kill,
            banner,
            via,
        }),
        disconnected,
        elapsed: clock.active(begin),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_skips_preamble() {
        let mut b = Banner::default();
        assert!(!b.feed(b"hello\r\nSSH-2.0-Open"));
        assert!(b.feed(b"SSH_9.6\r\n\x00\x00"));
        assert_eq!(b.line.as_deref(), Some("SSH-2.0-OpenSSH_9.6"));

        let mut b = Banner::default();
        assert!(b.feed(&[b'x'; 2000]));
        assert_eq!(b.line, None);
    }
}
