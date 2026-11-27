//! 经 SSH 连接访问网络的本机 SOCKS5 代理：只监听回环地址，用随机生成的用户名密码认证，
//! 每个请求在 SSH 连接上开一条 direct-tcpip channel，由服务器去连接目标地址（域名也由服务器解析）。

use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::{CancellationToken, DropGuard};

use super::{DEFAULT_TIMEOUT, SshClient};
use crate::i18n::Msg;
use crate::util::{join_host_port, new_id};

const VERSION: u8 = 0x05;
const METHOD_USER_PASS: u8 = 0x02;
const METHOD_NO_ACCEPTABLE: u8 = 0xFF;
const USER_PASS_VERSION: u8 = 0x01;
const CMD_CONNECT: u8 = 0x01;
const ATYP_IPV4: u8 = 0x01;
const ATYP_DOMAIN: u8 = 0x03;
const ATYP_IPV6: u8 = 0x04;
const REP_SUCCEEDED: u8 = 0x00;
const REP_HOST_UNREACHABLE: u8 = 0x04;
const REP_COMMAND_NOT_SUPPORTED: u8 = 0x07;
const REP_ADDR_NOT_SUPPORTED: u8 = 0x08;

/// 正在运行的代理，drop 时停止监听并断开经它建立的连接。
pub struct Socks5Proxy {
    addr: SocketAddr,
    user: String,
    pass: String,
    /// 最近一次服务器连不上目标地址的原因。
    error: Arc<Mutex<Option<Msg>>>,
    _stop: DropGuard,
}

impl Socks5Proxy {
    /// 形如 socks5h://user:pass@127.0.0.1:port，用户名密码只含十六进制字符，不用转义。
    pub fn url(&self) -> String {
        format!("socks5h://{}:{}@{}", self.user, self.pass, self.addr)
    }

    pub fn last_error(&self) -> Option<Msg> {
        self.error.lock().unwrap().clone()
    }
}

impl SshClient {
    /// 在本机回环地址上启动经本连接转发的 SOCKS5 代理；jump 是本连接在提示里的名称。
    /// 代理持有本连接，直到代理停止。
    pub async fn socks5_proxy(self: &Arc<Self>, jump: &str) -> io::Result<Socks5Proxy> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let stop = CancellationToken::new();
        let proxy = Socks5Proxy {
            addr: listener.local_addr()?,
            user: new_id(),
            pass: new_id(),
            error: Arc::default(),
            _stop: stop.clone().drop_guard(),
        };
        let ctx = Arc::new(Ctx {
            client: self.clone(),
            jump: jump.to_string(),
            user: proxy.user.clone(),
            pass: proxy.pass.clone(),
            error: proxy.error.clone(),
        });
        tokio::spawn(async move {
            loop {
                let sock = tokio::select! {
                    _ = stop.cancelled() => break,
                    r = listener.accept() => match r {
                        Ok((sock, _)) => sock,
                        Err(_) => {
                            tokio::time::sleep(Duration::from_millis(100)).await;
                            continue;
                        }
                    },
                };
                let (ctx, stop) = (ctx.clone(), stop.clone());
                tokio::spawn(async move {
                    tokio::select! {
                        _ = stop.cancelled() => {}
                        _ = ctx.serve(sock) => {}
                    }
                });
            }
        });
        Ok(proxy)
    }
}

struct Ctx {
    client: Arc<SshClient>,
    jump: String,
    user: String,
    pass: String,
    error: Arc<Mutex<Option<Msg>>>,
}

impl Ctx {
    async fn serve(&self, mut sock: TcpStream) -> io::Result<()> {
        let _ = sock.set_nodelay(true);
        negotiate(&mut sock, &self.user, &self.pass).await?;
        let (host, port) = match read_request(&mut sock).await? {
            Ok(target) => target,
            Err(code) => return reply(&mut sock, code).await,
        };
        let (jump, addr) = (&self.jump, join_host_port(&host, port));
        let open = self.client.handle.channel_open_direct_tcpip(
            host.as_str(),
            port as u32,
            "127.0.0.1",
            0,
        );
        let channel = match tokio::time::timeout(DEFAULT_TIMEOUT, open).await {
            Ok(Ok(ch)) => ch,
            Ok(Err(e)) => {
                let cause = match e {
                    russh::Error::ChannelOpenFailure(f) => f.description().to_string(),
                    e => e.to_string(),
                };
                let m = msg!(
                    "err.ssh.jumpChannelFailed",
                    "跳板机 {jump} 无法连接到 {addr}",
                    jump,
                    addr
                )
                .with_cause(cause);
                *self.error.lock().unwrap() = Some(m);
                return reply(&mut sock, REP_HOST_UNREACHABLE).await;
            }
            Err(_) => {
                let m = msg!(
                    "err.ssh.jumpChannelTimeout",
                    "跳板机 {jump} 连接 {addr} 超时",
                    jump,
                    addr
                );
                *self.error.lock().unwrap() = Some(m);
                return reply(&mut sock, REP_HOST_UNREACHABLE).await;
            }
        };
        reply(&mut sock, REP_SUCCEEDED).await?;
        let mut stream = channel.into_stream();
        tokio::io::copy_bidirectional(&mut sock, &mut stream).await?;
        Ok(())
    }
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_string())
}

/// 方法协商和 RFC 1929 用户名密码认证，只接受用户名密码方式。
async fn negotiate<S>(c: &mut S, user: &str, pass: &str) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut hdr = [0u8; 2];
    c.read_exact(&mut hdr).await?;
    if hdr[0] != VERSION {
        return Err(invalid("SOCKS 版本号错误"));
    }
    let mut methods = vec![0u8; hdr[1] as usize];
    c.read_exact(&mut methods).await?;
    if !methods.contains(&METHOD_USER_PASS) {
        let _ = c.write_all(&[VERSION, METHOD_NO_ACCEPTABLE]).await;
        return Err(invalid("客户端没有提供用户名密码认证"));
    }
    c.write_all(&[VERSION, METHOD_USER_PASS]).await?;

    let mut ver = [0u8; 1];
    c.read_exact(&mut ver).await?;
    let u = read_len_prefixed(c).await?;
    let p = read_len_prefixed(c).await?;
    if ver[0] != USER_PASS_VERSION || u != user.as_bytes() || p != pass.as_bytes() {
        let _ = c.write_all(&[USER_PASS_VERSION, 0x01]).await;
        return Err(invalid("用户名或密码错误"));
    }
    c.write_all(&[USER_PASS_VERSION, 0x00]).await
}

async fn read_len_prefixed<S: AsyncRead + Unpin>(c: &mut S) -> io::Result<Vec<u8>> {
    let mut n = [0u8; 1];
    c.read_exact(&mut n).await?;
    let mut b = vec![0u8; n[0] as usize];
    c.read_exact(&mut b).await?;
    Ok(b)
}

/// 读取 CONNECT 请求，返回目标主机和端口；请求不受支持时返回要回复的错误码。
async fn read_request<S: AsyncRead + Unpin>(c: &mut S) -> io::Result<Result<(String, u16), u8>> {
    let mut hdr = [0u8; 4];
    c.read_exact(&mut hdr).await?;
    if hdr[0] != VERSION {
        return Err(invalid("SOCKS 版本号错误"));
    }
    // direct-tcpip 只能转发 TCP，BIND 和 UDP ASSOCIATE 都不支持。
    if hdr[1] != CMD_CONNECT {
        return Ok(Err(REP_COMMAND_NOT_SUPPORTED));
    }
    let host = match hdr[3] {
        ATYP_IPV4 => {
            let mut ip = [0u8; 4];
            c.read_exact(&mut ip).await?;
            Ipv4Addr::from(ip).to_string()
        }
        ATYP_IPV6 => {
            let mut ip = [0u8; 16];
            c.read_exact(&mut ip).await?;
            Ipv6Addr::from(ip).to_string()
        }
        ATYP_DOMAIN => {
            let name = read_len_prefixed(c).await?;
            if name.is_empty() {
                return Ok(Err(REP_ADDR_NOT_SUPPORTED));
            }
            String::from_utf8_lossy(&name).into_owned()
        }
        _ => return Ok(Err(REP_ADDR_NOT_SUPPORTED)),
    };
    let mut port = [0u8; 2];
    c.read_exact(&mut port).await?;
    Ok(Ok((host, u16::from_be_bytes(port))))
}

async fn reply<S: AsyncWrite + Unpin>(c: &mut S, code: u8) -> io::Result<()> {
    c.write_all(&[VERSION, code, 0x00, ATYP_IPV4, 0, 0, 0, 0, 0, 0])
        .await?;
    c.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn handshake_and_request() {
        let (mut client, mut server) = tokio::io::duplex(256);
        let srv = tokio::spawn(async move {
            negotiate(&mut server, "u", "p").await?;
            read_request(&mut server).await
        });
        client
            .write_all(&[VERSION, 1, METHOD_USER_PASS])
            .await
            .unwrap();
        client
            .write_all(&[USER_PASS_VERSION, 1, b'u', 1, b'p'])
            .await
            .unwrap();
        client
            .write_all(&[VERSION, CMD_CONNECT, 0, ATYP_DOMAIN, 10])
            .await
            .unwrap();
        client.write_all(b"github.com").await.unwrap();
        client.write_all(&443u16.to_be_bytes()).await.unwrap();
        let mut resp = [0u8; 4];
        client.read_exact(&mut resp).await.unwrap();
        assert_eq!(resp, [VERSION, METHOD_USER_PASS, USER_PASS_VERSION, 0x00]);
        assert_eq!(
            srv.await.unwrap().unwrap(),
            Ok(("github.com".to_string(), 443))
        );
    }

    #[tokio::test]
    async fn rejects_wrong_password_and_no_auth() {
        let (mut client, mut server) = tokio::io::duplex(256);
        let srv = tokio::spawn(async move { negotiate(&mut server, "u", "p").await });
        client
            .write_all(&[VERSION, 1, METHOD_USER_PASS])
            .await
            .unwrap();
        client
            .write_all(&[USER_PASS_VERSION, 1, b'u', 1, b'x'])
            .await
            .unwrap();
        let mut resp = [0u8; 4];
        client.read_exact(&mut resp).await.unwrap();
        assert_eq!(resp[2..], [USER_PASS_VERSION, 0x01]);
        assert!(srv.await.unwrap().is_err());

        let (mut client, mut server) = tokio::io::duplex(256);
        let srv = tokio::spawn(async move { negotiate(&mut server, "u", "p").await });
        client.write_all(&[VERSION, 1, 0x00]).await.unwrap();
        let mut resp = [0u8; 2];
        client.read_exact(&mut resp).await.unwrap();
        assert_eq!(resp, [VERSION, METHOD_NO_ACCEPTABLE]);
        assert!(srv.await.unwrap().is_err());
    }

    #[tokio::test]
    async fn unsupported_command() {
        let (mut client, mut server) = tokio::io::duplex(256);
        client
            .write_all(&[VERSION, 0x03, 0, ATYP_IPV4, 1, 2, 3, 4, 0, 53])
            .await
            .unwrap();
        assert_eq!(
            read_request(&mut server).await.unwrap(),
            Err(REP_COMMAND_NOT_SUPPORTED)
        );
    }
}
