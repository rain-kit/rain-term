pub mod cert;
mod clipboard;
pub mod input;
mod session;

use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use ironrdp_cliprdr::CliprdrClient;
use ironrdp_connector::credssp::CredsspSequence;
use ironrdp_connector::sspi::generator::NetworkRequest;
use ironrdp_connector::{
    BitmapConfig, ClientConnector, ClientConnectorState, Config, ConnectionResult, ConnectorError,
    ConnectorErrorKind, ConnectorResult, Credentials, DesktopSize, ServerName,
};
use ironrdp_core::WriteBuf;
use ironrdp_displaycontrol::client::DisplayControlClient;
use ironrdp_dvc::DrdynvcClient;
use ironrdp_egfx::client::{GraphicsPipelineClient, GraphicsPipelineHandler};
use ironrdp_egfx::pdu::{
    CapabilitiesV8Flags, CapabilitiesV10Flags, CapabilitiesV103Flags, CapabilitiesV104Flags,
    CapabilitiesV107Flags, CapabilitySet,
};
use ironrdp_pdu::gcc;
use ironrdp_pdu::nego::FailureCode;
use ironrdp_pdu::rdp::capability_sets::{MajorPlatformType, RailSupportLevel};
use ironrdp_pdu::rdp::client_info::CompressionType;
use ironrdp_tokio::{MovableTokioFramed, NetworkClient};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use x509_cert::der::Encode;

use crate::i18n::Msg;
use crate::model::{RDP_MAX_SIZE, RDP_MIN_SIZE, RdpParams};
use crate::util::join_host_port;
use cert::{Check, CertStore, fingerprint};
pub use session::{Command, Event, Frames, run};

/// TCP 连接、TLS 握手和登录各自的超时，等用户确认证书的时间不计入。
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

pub trait Transport: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Transport for T {}

/// 直连的 TCP 或经跳板机转发的 SSH channel。
pub type BoxStream = Box<dyn Transport>;
type Framed = MovableTokioFramed<BoxStream>;

#[derive(Debug, thiserror::Error)]
pub enum RdpError {
    #[error("已取消")]
    Cancelled,
    #[error("已拒绝信任该服务器的证书")]
    CertRejected,
    #[error("{}", .0.text())]
    Other(Msg),
}

impl RdpError {
    pub fn msg(&self) -> Msg {
        match self {
            RdpError::Cancelled => msg!("err.ssh.cancelled", "已取消"),
            RdpError::CertRejected => msg!("err.rdp.certRejected", "已拒绝信任该服务器的证书"),
            RdpError::Other(m) => m.clone(),
        }
    }
}

/// 建立远程桌面连接需要的信息。
#[derive(Debug, Clone, Default)]
pub struct RdpTarget {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub domain: String,
    pub password: String,
    pub width: u16,
    pub height: u16,
    /// 缩放比例（百分比），100 表示不缩放。
    pub scale: u32,
    pub clipboard: bool,
}

impl RdpTarget {
    /// 端口已由连接校验保证在 1 ~ 65535 之间；宽高会被限制在 RDP 允许的范围内。
    pub fn from_params(p: &RdpParams, width: u32, height: u32, scale: u32) -> Self {
        RdpTarget {
            host: p.host.clone(),
            port: p.port as u16,
            user: p.user.clone(),
            domain: p.domain.clone(),
            password: p.password.clone(),
            width: clamp_size(width) & !1,
            height: clamp_size(height),
            scale: scale.clamp(100, 500),
            clipboard: p.clipboard,
        }
    }

    pub fn addr(&self) -> String {
        join_host_port(&self.host, self.port)
    }

    /// 用户名可以写成 `域\用户`，没有单独填域时从中拆出。
    fn credentials(&self) -> (String, Option<String>) {
        if self.domain.is_empty()
            && let Some((domain, user)) = self.user.split_once('\\')
        {
            return (user.to_string(), Some(domain.to_string()));
        }
        let domain = (!self.domain.is_empty()).then(|| self.domain.clone());
        (self.user.clone(), domain)
    }

    fn config(&self) -> Config {
        let (username, domain) = self.credentials();
        Config {
            desktop_size: DesktopSize {
                width: self.width,
                height: self.height,
            },
            monitor_layout: None,
            desktop_scale_factor: self.scale,
            desktop_physical_size: None,
            desktop_orientation: None,
            device_scale_factor: None,
            monitor_extended_layout: None,
            relative_mouse_input: false,
            enable_tls: true,
            enable_credssp: true,
            enable_standard_rdp_security: false,
            autologon: !self.password.is_empty(),
            credentials: Credentials::UsernamePassword {
                username,
                password: self.password.clone(),
            },
            domain,
            client_build: 0,
            client_name: client_name(),
            keyboard_type: gcc::KeyboardType::IBM_ENHANCED,
            keyboard_subtype: 0,
            keyboard_functional_keys_count: 12,
            keyboard_layout: 0,
            connection_type: gcc::ConnectionType::Lan,
            ime_file_name: String::new(),
            bitmap: Some(BitmapConfig {
                lossy_compression: true,
                color_depth: 32,
                codecs: ironrdp_pdu::rdp::capability_sets::client_codecs_capabilities(&[])
                    .expect("默认编解码器配置有效"),
            }),
            dig_product_id: String::new(),
            client_dir: String::new(),
            alternate_shell: String::new(),
            work_dir: String::new(),
            remote_application_mode: false,
            rail_support_level: RailSupportLevel::empty(),
            platform: platform(),
            hardware_id: None,
            request_data: None,
            enable_audio_playback: false,
            enable_audio_capture: false,
            performance_flags: Default::default(),
            license_cache: None,
            timezone_info: Default::default(),
            compression_type: Some(CompressionType::K64),
            enable_server_pointer: true,
            pointer_software_rendering: false,
            multitransport_flags: None,
            support_dyn_vc_gfx_protocol: true,
        }
    }
}

/// 图形管道（RDPGFX）：服务器改用 RemoteFX 渐进编码、ClearCodec 等现代编码发画面，
/// 比旧的位图更新省带宽。没有 H.264 解码器，只声明不带 AVC 的能力集。
struct Gfx;

impl GraphicsPipelineHandler for Gfx {
    fn capabilities(&self) -> Vec<CapabilitySet> {
        vec![
            CapabilitySet::V10_7 {
                flags: CapabilitiesV107Flags::SMALL_CACHE | CapabilitiesV107Flags::AVC_DISABLED,
            },
            CapabilitySet::V10_6 {
                flags: CapabilitiesV104Flags::SMALL_CACHE | CapabilitiesV104Flags::AVC_DISABLED,
            },
            CapabilitySet::V10_5 {
                flags: CapabilitiesV104Flags::SMALL_CACHE | CapabilitiesV104Flags::AVC_DISABLED,
            },
            CapabilitySet::V10_4 {
                flags: CapabilitiesV104Flags::SMALL_CACHE | CapabilitiesV104Flags::AVC_DISABLED,
            },
            CapabilitySet::V10_3 {
                flags: CapabilitiesV103Flags::AVC_DISABLED,
            },
            CapabilitySet::V10_2 {
                flags: CapabilitiesV10Flags::SMALL_CACHE | CapabilitiesV10Flags::AVC_DISABLED,
            },
            CapabilitySet::V10 {
                flags: CapabilitiesV10Flags::SMALL_CACHE | CapabilitiesV10Flags::AVC_DISABLED,
            },
            CapabilitySet::V8 {
                flags: CapabilitiesV8Flags::SMALL_CACHE,
            },
        ]
    }
}

fn clamp_size(v: u32) -> u16 {
    v.clamp(RDP_MIN_SIZE as u32, RDP_MAX_SIZE as u32) as u16
}

fn client_name() -> String {
    let name = hostname();
    if name.is_empty() {
        "rain-term".into()
    } else {
        name
    }
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_default()
}

fn platform() -> MajorPlatformType {
    if cfg!(windows) {
        MajorPlatformType::WINDOWS
    } else if cfg!(target_os = "macos") {
        MajorPlatformType::MACINTOSH
    } else {
        MajorPlatformType::UNIX
    }
}

/// 遇到未知证书或证书变化时询问用户是否信任；cancel 取消时应尽快返回 false。
pub struct CertQuery {
    pub host: String,
    /// 证书的使用者，例如 CN=WIN-SERVER。
    pub subject: String,
    pub fingerprint: String,
    /// 证书变化时，上次信任的指纹。
    pub previous: Option<String>,
    pub cancel: CancellationToken,
}

pub type TrustFn =
    Arc<dyn Fn(CertQuery) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

pub struct DialOptions {
    pub certs: Arc<CertStore>,
    /// 为 None 时未知证书一律拒绝。
    pub trust: Option<TrustFn>,
    pub timeout: Duration,
}

/// 连接远程桌面服务器的 TCP 端口。
pub async fn dial_tcp(
    host: &str,
    port: u16,
    timeout: Duration,
    cancel: &CancellationToken,
) -> Result<BoxStream, RdpError> {
    let addr = join_host_port(host, port);
    let tcp = tokio::select! {
        _ = cancel.cancelled() => return Err(RdpError::Cancelled),
        r = tokio::time::timeout(timeout, TcpStream::connect((host, port))) => match r {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return Err(RdpError::Other(msg!("err.rdp.connectFailed", "连接 {addr} 失败", addr).with_cause(e))),
            Err(_) => return Err(RdpError::Other(msg!("err.rdp.connectTimeout", "连接 {addr} 超时", addr))),
        },
    };
    let _ = tcp.set_nodelay(true);
    Ok(Box::new(tcp))
}

/// 完成 TLS 握手、确认过证书，还没有发送密码的连接。
struct Secured {
    framed: Framed,
    connector: ClientConnector,
    upgraded: ironrdp_tokio::Upgraded,
    server_public_key: Vec<u8>,
}

/// 登录完成、可以开始会话的连接。
pub struct Connected {
    framed: Framed,
    result: ConnectionResult,
    clipboard: bool,
    /// 从发起连接到登录完成的耗时，不含等待用户确认证书的时间。
    pub elapsed: Duration,
}

/// 建立连接并登录到远程桌面。stream 是已经连上服务器端口的字节流，begin 是开始建立它的时间。
pub async fn connect(
    stream: BoxStream,
    t: &RdpTarget,
    opts: &DialOptions,
    cancel: &CancellationToken,
    begin: Instant,
) -> Result<Connected, RdpError> {
    let (s, waited) = secure(stream, t, opts, cancel).await?;
    let Secured {
        mut framed,
        connector,
        upgraded,
        server_public_key,
    } = s;
    let addr = t.addr();
    let mut network = NoNetwork;
    let login = boxed(ironrdp_tokio::connect_finalize(
        upgraded,
        connector,
        &mut framed,
        &mut network,
        ServerName::new(&t.host),
        server_public_key,
        None,
    ));
    let outcome = tokio::select! {
        _ = cancel.cancelled() => return Err(RdpError::Cancelled),
        r = tokio::time::timeout(opts.timeout, login) => match r {
            Ok(r) => r.map_err(|e| connector_error(&e))?,
            Err(_) => return Err(RdpError::Other(msg!("err.rdp.loginTimeout", "登录 {addr} 超时", addr))),
        },
    };
    let result = outcome.into_connected().map_err(|e| connector_error(&e))?;
    Ok(Connected {
        framed,
        result,
        clipboard: t.clipboard,
        elapsed: begin.elapsed().saturating_sub(waited),
    })
}

/// 测试连接：只验证网络、证书和用户名密码，不登录桌面，避免挤掉这个用户正在使用的会话。
/// 返回不含等待用户确认证书的耗时。
pub async fn probe(
    stream: BoxStream,
    t: &RdpTarget,
    opts: &DialOptions,
    cancel: &CancellationToken,
    begin: Instant,
) -> Result<Duration, RdpError> {
    let (s, waited) = secure(stream, t, opts, cancel).await?;
    let Secured {
        mut framed,
        connector,
        server_public_key,
        ..
    } = s;
    if let ClientConnectorState::Credssp {
        selected_protocol, ..
    } = connector.state
    {
        let addr = t.addr();
        let (sequence, ts_request) = CredsspSequence::init(
            connector.config.credentials.clone(),
            connector.config.domain.as_deref(),
            selected_protocol,
            ServerName::new(&t.host),
            server_public_key,
            None,
        )
        .map_err(|e| connector_error(&e))?;
        drop(connector);
        let auth = async {
            let mut buf = WriteBuf::new();
            boxed(ironrdp_tokio::perform_credssp(&mut framed, &mut NoNetwork, &mut buf, sequence, ts_request))
                .await
        };
        tokio::select! {
            _ = cancel.cancelled() => return Err(RdpError::Cancelled),
            r = tokio::time::timeout(opts.timeout, auth) => match r {
                Ok(r) => r.map_err(|e| connector_error(&e))?,
                Err(_) => return Err(RdpError::Other(msg!("err.rdp.loginTimeout", "登录 {addr} 超时", addr))),
            },
        }
    }
    Ok(begin.elapsed().saturating_sub(waited))
}

/// 协商安全协议、升级到 TLS 并确认证书；返回的时长是等待用户确认证书的时间。
async fn secure(
    stream: BoxStream,
    t: &RdpTarget,
    opts: &DialOptions,
    cancel: &CancellationToken,
) -> Result<(Secured, Duration), RdpError> {
    let addr = t.addr();
    let client_addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
    let mut connector = ClientConnector::new(t.config(), client_addr).with_static_channel(
        DrdynvcClient::new()
            .with_dynamic_channel(DisplayControlClient::new(|_| Ok(Vec::new())))
            .with_dynamic_channel(GraphicsPipelineClient::new(Box::new(Gfx), None)),
    );
    if t.clipboard {
        connector.attach_static_channel(CliprdrClient::new(Box::new(
            clipboard::TextClipboard::default(),
        )));
    }
    let mut framed = Framed::new(stream);

    let handshake = async {
        let should_upgrade = boxed(ironrdp_tokio::connect_begin(&mut framed, &mut connector))
            .await
            .map_err(|e| connector_error(&e))?;
        let (stream, leftover) = framed.into_inner();
        let (tls, cert) = ironrdp_tls::upgrade(stream, &t.host).await.map_err(|e| {
            RdpError::Other(msg!("err.rdp.tlsFailed", "与 {addr} 的 TLS 握手失败", addr).with_cause(e))
        })?;
        Ok::<_, RdpError>((should_upgrade, tls, leftover, cert))
    };
    let (should_upgrade, tls, leftover, cert) = tokio::select! {
        _ = cancel.cancelled() => return Err(RdpError::Cancelled),
        r = tokio::time::timeout(opts.timeout, handshake) => match r {
            Ok(r) => r?,
            Err(_) => return Err(RdpError::Other(msg!("err.rdp.handshakeTimeout", "与 {addr} 的远程桌面握手超时", addr))),
        },
    };

    let waiting = Instant::now();
    verify_cert(t, &cert, opts, cancel).await?;
    let waited = waiting.elapsed();

    let server_public_key = ironrdp_tls::extract_tls_server_public_key(&cert)
        .ok_or_else(|| RdpError::Other(msg!("err.rdp.certInvalid", "服务器证书无效")))?
        .to_owned();
    let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector);
    let framed = Framed::new_with_leftover(Box::new(tls) as BoxStream, leftover);
    Ok((
        Secured {
            framed,
            connector,
            upgraded,
            server_public_key,
        },
        waited,
    ))
}

/// 和上次信任的证书比较，未知或变化时询问用户。
async fn verify_cert(
    t: &RdpTarget,
    cert: &x509_cert::Certificate,
    opts: &DialOptions,
    cancel: &CancellationToken,
) -> Result<(), RdpError> {
    let der = cert
        .to_der()
        .map_err(|e| RdpError::Other(msg!("err.rdp.certInvalid", "服务器证书无效").with_cause(e)))?;
    let fp = fingerprint(&der);
    let store_err =
        |e: std::io::Error| RdpError::Other(msg!("err.rdp.certStore", "读写证书记录失败").with_cause(e));
    let _guard = opts.certs.lock().await;
    let previous = match opts.certs.check(&t.host, t.port, &fp).map_err(store_err)? {
        Check::Known => return Ok(()),
        Check::Unknown => None,
        Check::Changed(old) => Some(old),
    };
    let Some(trust) = &opts.trust else {
        return Err(RdpError::CertRejected);
    };
    let query = CertQuery {
        host: t.addr(),
        subject: cert.tbs_certificate().subject().to_string(),
        fingerprint: fp.clone(),
        previous,
        cancel: cancel.clone(),
    };
    if !trust(query).await {
        return Err(if cancel.is_cancelled() {
            RdpError::Cancelled
        } else {
            RdpError::CertRejected
        });
    }
    opts.certs.set(&t.host, t.port, &fp).map_err(store_err)
}

/// 装箱成 dyn Send。IronRDP 的一些 async fn 带 `&dyn` 参数，直接 await 时编译器证明不了
/// spawn 出去的 future 是 Send，在具体的调用处装箱就能通过。
pub(crate) fn boxed<'a, T>(
    f: impl Future<Output = T> + Send + 'a,
) -> Pin<Box<dyn Future<Output = T> + Send + 'a>> {
    Box::pin(f)
}

/// 不配置 Kerberos 时 CredSSP 只用 NTLM，不会发起网络请求。
struct NoNetwork;

impl NetworkClient for NoNetwork {
    async fn send(&mut self, _: &NetworkRequest) -> ConnectorResult<Vec<u8>> {
        Err(ConnectorError::new("Kerberos", ConnectorErrorKind::General))
    }
}

/// 把连接阶段的错误转成提示，尽量说清楚是谁的问题。
fn connector_error(e: &ConnectorError) -> RdpError {
    let cause = e.report().to_string();
    let m = match e.kind() {
        ConnectorErrorKind::Negotiation(f) => match f.code() {
            FailureCode::SSL_NOT_ALLOWED_BY_SERVER => msg!(
                "err.rdp.legacySecurity",
                "服务器只允许旧的 RDP 安全层，请在服务器上启用 TLS 或网络级身份验证"
            ),
            FailureCode::SSL_CERT_NOT_ON_SERVER => {
                msg!("err.rdp.noServerCert", "服务器没有可用于 TLS 的证书")
            }
            _ => msg!("err.rdp.negotiationFailed", "与服务器协商安全协议失败").with_cause(cause),
        },
        ConnectorErrorKind::Credssp(_) => msg!(
            "err.rdp.authFailed",
            "身份验证失败，请检查用户名、密码和域"
        )
        .with_cause(cause),
        ConnectorErrorKind::AccessDenied => {
            msg!("err.rdp.accessDenied", "该用户没有远程登录这台计算机的权限")
        }
        _ => msg!("err.rdp.protocol", "远程桌面协议错误").with_cause(cause),
    };
    RdpError::Other(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_from_params() {
        let p = RdpParams {
            host: "win".into(),
            user: r"CORP\alice".into(),
            ..Default::default()
        };
        let t = RdpTarget::from_params(&p, 1281, 50, 90);
        assert_eq!((t.width, t.height, t.scale), (1280, 200, 100));
        assert_eq!(t.credentials(), ("alice".into(), Some("CORP".into())));
        let p = RdpParams {
            user: r"CORP\alice".into(),
            domain: "OTHER".into(),
            ..p
        };
        let t = RdpTarget::from_params(&p, 1920, 1080, 150);
        assert_eq!(t.credentials(), (r"CORP\alice".into(), Some("OTHER".into())));
        assert_eq!(t.addr(), "win:3389");
    }
}
