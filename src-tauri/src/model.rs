use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use crate::i18n::Msg;

pub const DEFAULT_SSH_PORT: i32 = 22;
pub const DEFAULT_KEEPALIVE: i32 = 30;
pub const MAX_KEEPALIVE: i32 = 3600;
pub const DEFAULT_RDP_PORT: i32 = 3389;
pub const DEFAULT_RDP_WIDTH: i32 = 1920;
pub const DEFAULT_RDP_HEIGHT: i32 = 1080;
pub const RDP_MIN_SIZE: i32 = 200;
pub const RDP_MAX_SIZE: i32 = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnKind {
    Ssh,
    Rdp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AuthType {
    #[default]
    Password,
    Key,
    Agent,
}

/// SFTP 是否用 sudo 以 root 身份打开。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SftpSudo {
    /// 按顶栏“工具”里的设置。
    #[default]
    Default,
    Off,
    On,
}

impl SftpSudo {
    pub fn is_default(&self) -> bool {
        *self == SftpSudo::Default
    }

    /// global 是顶栏“工具”里的设置。
    pub fn enabled(self, global: bool) -> bool {
        match self {
            SftpSudo::Default => global,
            SftpSudo::Off => false,
            SftpSudo::On => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SshParams {
    pub host: String,
    pub port: i32,
    pub user: String,
    pub auth: AuthType,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub key_path: String,
    /// 跳板机：另一个 SSH 连接的 ID。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub jump_id: String,
    /// 保活间隔（秒），0 表示不发送。
    pub keepalive_seconds: i32,
    /// 登录后进入的目录，为空时使用服务器默认目录。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub init_dir: String,
    /// 无痕模式：禁止本次会话把命令写入历史文件。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub incognito: bool,
    #[serde(skip_serializing_if = "SftpSudo::is_default")]
    pub sftp_sudo: SftpSudo,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub password: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub key_passphrase: String,
}

impl Default for SshParams {
    fn default() -> Self {
        SshParams {
            host: String::new(),
            port: DEFAULT_SSH_PORT,
            user: String::new(),
            auth: AuthType::Password,
            key_path: String::new(),
            jump_id: String::new(),
            keepalive_seconds: DEFAULT_KEEPALIVE,
            init_dir: String::new(),
            incognito: false,
            sftp_sudo: SftpSudo::Default,
            password: String::new(),
            key_passphrase: String::new(),
        }
    }
}

/// 远程桌面的分辨率：跟随窗口大小，或固定为 width × height。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RdpResolution {
    #[default]
    Fit,
    Fixed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RdpParams {
    pub host: String,
    pub port: i32,
    pub user: String,
    /// 通过这个 SSH 连接转发到远程桌面。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub jump_id: String,
    pub resolution: RdpResolution,
    pub width: i32,
    pub height: i32,
    /// 共享文本剪贴板。
    pub clipboard: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub password: String,
}

impl Default for RdpParams {
    fn default() -> Self {
        RdpParams {
            host: String::new(),
            port: DEFAULT_RDP_PORT,
            user: String::new(),
            jump_id: String::new(),
            resolution: RdpResolution::Fit,
            width: DEFAULT_RDP_WIDTH,
            height: DEFAULT_RDP_HEIGHT,
            clipboard: true,
            password: String::new(),
        }
    }
}

/// 连接参数，JSON 中用 kind 字段区分类型，和公共字段平铺在一起。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ConnParams {
    Ssh(SshParams),
    Rdp(RdpParams),
}

impl Default for ConnParams {
    fn default() -> Self {
        ConnParams::Ssh(SshParams::default())
    }
}

/// 一个连接的持久化定义，保存在 connections.json。
/// 密码类字段不写入文件，由 secrets 存进系统凭据库。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group: String,
    /// 标记颜色，取值见前端 src/lib/format.ts 的 CONN_COLORS，为空表示不标记。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// 生产环境：列表和标签显示红色，终端粘贴前确认。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub production: bool,
    #[serde(default)]
    pub created_at: Option<DateTime<Local>>,
    #[serde(flatten)]
    pub params: ConnParams,
}

fn valid_port(p: i32) -> bool {
    (1..=65535).contains(&p)
}

fn trim(s: &mut String) {
    let t = s.trim();
    if t.len() != s.len() {
        *s = t.to_string();
    }
}

impl Connection {
    pub fn kind(&self) -> ConnKind {
        match self.params {
            ConnParams::Ssh(_) => ConnKind::Ssh,
            ConnParams::Rdp(_) => ConnKind::Rdp,
        }
    }

    /// SSH 连接的参数，其他类型的连接返回错误。
    pub fn ssh_params(&self) -> Result<&SshParams, Msg> {
        match &self.params {
            ConnParams::Ssh(p) => Ok(p),
            _ => Err(msg!("err.conn.notSsh", "这个功能只支持 SSH 连接")),
        }
    }

    /// 跳板机的连接 ID，没有设置时返回 None。
    pub fn jump_ref(&self) -> Option<&str> {
        let id = match &self.params {
            ConnParams::Ssh(p) => &p.jump_id,
            ConnParams::Rdp(p) => &p.jump_id,
        };
        (!id.is_empty()).then_some(id.as_str())
    }

    pub fn jump_ref_mut(&mut self) -> &mut String {
        match &mut self.params {
            ConnParams::Ssh(p) => &mut p.jump_id,
            ConnParams::Rdp(p) => &mut p.jump_id,
        }
    }

    /// 去掉首尾空白、填充默认端口和名称，清掉当前认证方式用不到的字段。
    pub fn normalize(&mut self) {
        trim(&mut self.name);
        trim(&mut self.group);
        trim(&mut self.color);
        match &mut self.params {
            ConnParams::Ssh(p) => {
                for s in [
                    &mut p.host,
                    &mut p.user,
                    &mut p.key_path,
                    &mut p.jump_id,
                    &mut p.init_dir,
                ] {
                    trim(s);
                }
                if p.port == 0 {
                    p.port = DEFAULT_SSH_PORT;
                }
                if p.auth != AuthType::Key {
                    p.key_path.clear();
                }
            }
            ConnParams::Rdp(p) => {
                for s in [&mut p.host, &mut p.user, &mut p.jump_id] {
                    trim(s);
                }
                if p.port == 0 {
                    p.port = DEFAULT_RDP_PORT;
                }
                if p.width == 0 || p.height == 0 {
                    p.width = DEFAULT_RDP_WIDTH;
                    p.height = DEFAULT_RDP_HEIGHT;
                }
            }
        }
        if self.name.is_empty() {
            self.name = self.default_name();
        }
    }

    fn default_name(&self) -> String {
        match &self.params {
            ConnParams::Ssh(p) => format!("{}@{}", p.user, p.host),
            ConnParams::Rdp(p) if p.user.is_empty() => p.host.clone(),
            ConnParams::Rdp(p) => format!("{}@{}", p.user, p.host),
        }
    }

    /// 校验本连接自身的字段，引用关系由 [`validate_refs`] 检查。
    pub fn validate(&self) -> Result<(), Msg> {
        match &self.params {
            ConnParams::Ssh(p) => {
                if p.host.is_empty() {
                    return Err(msg!("err.conn.hostRequired", "地址不能为空"));
                }
                if !valid_port(p.port) {
                    let port = p.port;
                    return Err(msg!("err.conn.portInvalid", "端口无效：{port}", port));
                }
                if p.user.is_empty() {
                    return Err(msg!("err.conn.userRequired", "用户名不能为空"));
                }
                if p.auth == AuthType::Key && p.key_path.is_empty() {
                    return Err(msg!("err.conn.keyPathRequired", "私钥认证需要选择私钥文件"));
                }
                if !(0..=MAX_KEEPALIVE).contains(&p.keepalive_seconds) {
                    let (value, max) = (p.keepalive_seconds, MAX_KEEPALIVE);
                    return Err(msg!(
                        "err.conn.keepaliveInvalid",
                        "保活间隔应在 0 到 {max} 秒之间：{value}",
                        max,
                        value
                    ));
                }
            }
            ConnParams::Rdp(p) => {
                if p.host.is_empty() {
                    return Err(msg!("err.conn.hostRequired", "地址不能为空"));
                }
                if !valid_port(p.port) {
                    let port = p.port;
                    return Err(msg!("err.conn.portInvalid", "端口无效：{port}", port));
                }
                if p.user.is_empty() {
                    return Err(msg!("err.conn.userRequired", "用户名不能为空"));
                }
                let range = RDP_MIN_SIZE..=RDP_MAX_SIZE;
                if p.resolution == RdpResolution::Fixed
                    && !(range.contains(&p.width) && range.contains(&p.height))
                {
                    let (width, height, min, max) = (p.width, p.height, RDP_MIN_SIZE, RDP_MAX_SIZE);
                    return Err(msg!(
                        "err.conn.rdpSizeInvalid",
                        "分辨率的宽和高应在 {min} 到 {max} 之间：{width} × {height}",
                        min,
                        max,
                        width,
                        height
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn clear_secrets(&mut self) {
        match &mut self.params {
            ConnParams::Ssh(p) => {
                p.password.clear();
                p.key_passphrase.clear();
            }
            ConnParams::Rdp(p) => p.password.clear(),
        }
    }

    /// 密码类字段：(凭据种类, 值)。只包含当前类型和认证方式用得到的种类。
    pub fn secrets_mut(&mut self) -> Vec<(&'static str, &mut String)> {
        use crate::secrets::{KEY_PASSPHRASE, PASSWORD};
        match &mut self.params {
            ConnParams::Ssh(p) => match p.auth {
                AuthType::Password => vec![(PASSWORD, &mut p.password)],
                AuthType::Key => vec![(KEY_PASSPHRASE, &mut p.key_passphrase)],
                AuthType::Agent => vec![],
            },
            ConnParams::Rdp(p) => vec![(PASSWORD, &mut p.password)],
        }
    }
}

/// 校验 `conn` 的跳板机：必须指向已有的 SSH 连接，不能指向自己，跳板链不能成环。
/// `all` 是全部连接，可以包含 `conn` 的旧版本（按 ID 以 `conn` 为准）。
pub fn validate_refs(conn: &Connection, all: &[Connection]) -> Result<(), Msg> {
    let Some(target) = conn.jump_ref() else {
        return Ok(());
    };
    let not_ssh = || msg!("err.conn.jumpNotSsh", "跳板机必须是其他 SSH 连接");
    if target == conn.id {
        return Err(not_ssh());
    }
    let by_id: HashMap<&str, &Connection> = all
        .iter()
        .filter(|c| c.id != conn.id)
        .map(|c| (c.id.as_str(), c))
        .chain(std::iter::once((conn.id.as_str(), conn)))
        .collect();
    match by_id.get(target) {
        None => return Err(msg!("err.conn.jumpNotFound", "跳板机不存在，可能已被删除")),
        Some(c) if c.kind() != ConnKind::Ssh => return Err(not_ssh()),
        _ => {}
    }

    let mut seen = HashSet::from([conn.id.as_str()]);
    let mut chain = vec![conn.name.as_str()];
    let mut cur = target;
    loop {
        let Some(c) = by_id.get(cur) else {
            return Ok(());
        };
        chain.push(c.name.as_str());
        if !seen.insert(cur) {
            let chain = chain.join(" → ");
            return Err(msg!("err.conn.jumpCycle", "跳板机形成循环：{chain}", chain));
        }
        match c.jump_ref() {
            Some(next) => cur = next,
            None => return Ok(()),
        }
    }
}

/// 返回给前端的连接：不包含任何密码，只告诉前端是否已保存。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnView {
    #[serde(flatten)]
    pub conn: Connection,
    pub has_password: bool,
    pub has_key_passphrase: bool,
}

/// 新建或修改连接。编辑已有连接时，密码类字段留空表示保持原值。
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SaveConnReq {
    #[serde(flatten)]
    pub conn: Connection,
    #[serde(default)]
    pub clear_password: bool,
    #[serde(default)]
    pub clear_key_passphrase: bool,
}

/// 测试连接的结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    pub elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
}

/// 首次连接某台服务器时，请前端弹窗确认指纹。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyPrompt {
    pub request_id: String,
    pub kind: ConnKind,
    /// 测试尚未保存的连接时为空。
    pub conn_id: String,
    pub conn_name: String,
    pub host: String,
    /// SSH 是密钥类型，远程桌面是证书的使用者。
    pub key_type: String,
    pub fingerprint: String,
    /// 远程桌面证书变化时，之前信任的指纹。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn ssh(id: &str, jump: &str) -> Connection {
        let mut c = Connection {
            id: id.into(),
            name: id.into(),
            params: ConnParams::Ssh(SshParams {
                host: "example.com".into(),
                user: "root".into(),
                jump_id: jump.into(),
                password: "pw".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        c.normalize();
        c
    }

    pub fn rdp(id: &str, jump: &str) -> Connection {
        let mut c = Connection {
            id: id.into(),
            name: id.into(),
            params: ConnParams::Rdp(RdpParams {
                host: "win.example.com".into(),
                user: "admin".into(),
                jump_id: jump.into(),
                password: "pw".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        c.normalize();
        c
    }

    pub fn ssh_of(c: &Connection) -> &SshParams {
        match &c.params {
            ConnParams::Ssh(p) => p,
            _ => panic!("不是 SSH 连接"),
        }
    }

    pub fn ssh_of_mut(c: &mut Connection) -> &mut SshParams {
        match &mut c.params {
            ConnParams::Ssh(p) => p,
            _ => panic!("不是 SSH 连接"),
        }
    }

    pub fn rdp_of(c: &Connection) -> &RdpParams {
        match &c.params {
            ConnParams::Rdp(p) => p,
            _ => panic!("不是远程桌面连接"),
        }
    }

    #[test]
    fn normalize_fills_defaults() {
        let mut c = Connection {
            params: ConnParams::Ssh(SshParams {
                host: " srv ".into(),
                user: "u".into(),
                port: 0,
                key_path: "/k".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        c.normalize();
        let p = ssh_of(&c);
        assert_eq!(p.host, "srv");
        assert_eq!(p.port, DEFAULT_SSH_PORT);
        assert_eq!(p.key_path, "", "密码认证用不到私钥路径");
        assert_eq!(c.name, "u@srv");
    }

    #[test]
    fn validate_fields() {
        let mut c = ssh("a", "");
        assert!(c.validate().is_ok());
        let p = ssh_of_mut(&mut c);
        p.port = 70000;
        assert_eq!(c.validate().unwrap_err().i18n, "err.conn.portInvalid");
        let p = ssh_of_mut(&mut c);
        p.port = 22;
        p.auth = AuthType::Key;
        assert_eq!(c.validate().unwrap_err().i18n, "err.conn.keyPathRequired");
    }

    #[test]
    fn json_is_flat_and_without_secrets() {
        let mut c = ssh("s", "j");
        c.clear_secrets();
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["kind"], "ssh");
        assert_eq!(v["jumpId"], "j");
        assert_eq!(v["auth"], "password");
        assert!(v.get("password").is_none());
        assert!(v.get("production").is_none(), "不是生产环境时不写这个字段");
        assert!(v.get("incognito").is_none(), "未开无痕时不写这个字段");
        assert!(v.get("sftpSudo").is_none(), "按工具设置时不写这个字段");
        let back: Connection = serde_json::from_value(v).unwrap();
        assert_eq!(back, c);

        c.production = true;
        let p = ssh_of_mut(&mut c);
        p.incognito = true;
        p.sftp_sudo = SftpSudo::On;
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["production"], true);
        assert_eq!(v["incognito"], true);
        assert_eq!(v["sftpSudo"], "on");
        assert_eq!(serde_json::from_value::<Connection>(v).unwrap(), c);

        let minimal: Connection =
            serde_json::from_str(r#"{"id":"x","kind":"ssh","host":"h"}"#).unwrap();
        let p = ssh_of(&minimal);
        assert_eq!(
            (p.host.as_str(), p.port, p.keepalive_seconds),
            ("h", DEFAULT_SSH_PORT, DEFAULT_KEEPALIVE)
        );
    }

    #[test]
    fn sftp_sudo_follows_global_by_default() {
        assert!(SftpSudo::Default.enabled(true));
        assert!(!SftpSudo::Default.enabled(false));
        assert!(!SftpSudo::Off.enabled(true));
        assert!(SftpSudo::On.enabled(false));
    }

    #[test]
    fn save_request_from_frontend() {
        let json = r#"{"id":"","name":"","group":"生产","color":"","note":"","kind":"ssh","host":"10.0.0.12","port":22,
            "user":"deploy","auth":"password","keyPath":"","password":"secret","keyPassphrase":"","jumpId":"s1",
            "keepaliveSeconds":30,"initDir":"","production":true,"clearPassword":false,"clearKeyPassphrase":true}"#;
        let req: SaveConnReq = serde_json::from_str(json).unwrap();
        assert!(req.clear_key_passphrase);
        assert_eq!(req.conn.group, "生产");
        assert!(req.conn.production);
        let p = ssh_of(&req.conn);
        assert_eq!(
            (
                p.host.as_str(),
                p.port,
                p.password.as_str(),
                p.jump_id.as_str()
            ),
            ("10.0.0.12", 22, "secret", "s1")
        );
        assert!(
            serde_json::from_str::<SaveConnReq>(r#"{"kind":"mysql","host":"h"}"#).is_err(),
            "不再支持数据库连接"
        );
    }

    #[test]
    fn refs_must_point_to_other_ssh() {
        let all = vec![ssh("s1", "")];
        assert!(validate_refs(&ssh("s2", "s1"), &all).is_ok());
        assert_eq!(
            validate_refs(&ssh("s2", "nope"), &all).unwrap_err().i18n,
            "err.conn.jumpNotFound"
        );
        assert_eq!(
            validate_refs(&ssh("s1", "s1"), &all).unwrap_err().i18n,
            "err.conn.jumpNotSsh",
            "不能指向自己"
        );
    }

    #[test]
    fn jump_cycle_detected() {
        // a → b → c，再把 c 的跳板机改成 a 就成环。
        let all = vec![ssh("a", "b"), ssh("b", "c"), ssh("c", "")];
        let err = validate_refs(&ssh("c", "a"), &all).unwrap_err();
        assert_eq!(err.i18n, "err.conn.jumpCycle");
        assert_eq!(err.text(), "跳板机形成循环：c → a → b → c");
        assert!(validate_refs(&ssh("c", ""), &all).is_ok());
    }

    #[test]
    fn rdp_defaults_and_json() {
        let minimal: Connection =
            serde_json::from_str(r#"{"id":"x","kind":"rdp","host":" win ","user":"u"}"#).unwrap();
        let mut c = minimal.clone();
        c.normalize();
        let p = rdp_of(&c);
        assert_eq!((p.host.as_str(), p.port), ("win", DEFAULT_RDP_PORT));
        assert_eq!(p.resolution, RdpResolution::Fit);
        assert!(p.clipboard, "默认共享剪贴板");
        assert_eq!(c.name, "u@win");
        assert_eq!(c.kind(), ConnKind::Rdp);
        assert_eq!(c.ssh_params().unwrap_err().i18n, "err.conn.notSsh");

        let mut c = rdp("r", "s1");
        assert_eq!(c.secrets_mut().len(), 1);
        c.clear_secrets();
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["kind"], "rdp");
        assert_eq!(v["jumpId"], "s1");
        assert!(v.get("password").is_none());
        assert!(v.get("domain").is_none());
        assert_eq!(serde_json::from_value::<Connection>(v).unwrap(), c);
    }

    #[test]
    fn rdp_validate() {
        let mut c = rdp("r", "");
        assert!(c.validate().is_ok());
        if let ConnParams::Rdp(p) = &mut c.params {
            p.resolution = RdpResolution::Fixed;
            p.width = 100;
        }
        assert_eq!(c.validate().unwrap_err().i18n, "err.conn.rdpSizeInvalid");
        if let ConnParams::Rdp(p) = &mut c.params {
            p.width = 1280;
            p.user.clear();
        }
        assert_eq!(c.validate().unwrap_err().i18n, "err.conn.userRequired");
    }

    #[test]
    fn rdp_can_use_ssh_jump_but_not_be_one() {
        let all = vec![ssh("s1", ""), rdp("r1", "")];
        assert!(validate_refs(&rdp("r2", "s1"), &all).is_ok());
        assert_eq!(
            validate_refs(&ssh("s2", "r1"), &all).unwrap_err().i18n,
            "err.conn.jumpNotSsh"
        );
        assert_eq!(
            validate_refs(&rdp("r2", "r1"), &all).unwrap_err().i18n,
            "err.conn.jumpNotSsh"
        );
    }
}
