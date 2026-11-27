use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use crate::i18n::Msg;

pub const DEFAULT_SSH_PORT: i32 = 22;
pub const DEFAULT_KEEPALIVE: i32 = 30;
pub const MAX_KEEPALIVE: i32 = 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnKind {
    Ssh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AuthType {
    #[default]
    Password,
    Key,
    Agent,
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
    /// 无痕模式：禁止本次会话把命令写入历史文件，并定期清除服务器上的历史。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub incognito: bool,
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
            password: String::new(),
            key_passphrase: String::new(),
        }
    }
}

/// 连接参数，JSON 中用 kind 字段区分类型，和公共字段平铺在一起。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ConnParams {
    Ssh(SshParams),
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
        }
    }

    /// 跳板机的连接 ID，没有设置时返回 None。
    pub fn jump_ref(&self) -> Option<&str> {
        let ConnParams::Ssh(p) = &self.params;
        (!p.jump_id.is_empty()).then_some(p.jump_id.as_str())
    }

    pub fn jump_ref_mut(&mut self) -> &mut String {
        let ConnParams::Ssh(p) = &mut self.params;
        &mut p.jump_id
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
        }
        if self.name.is_empty() {
            self.name = self.default_name();
        }
    }

    fn default_name(&self) -> String {
        match &self.params {
            ConnParams::Ssh(p) => format!("{}@{}", p.user, p.host),
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
        }
        Ok(())
    }

    pub fn clear_secrets(&mut self) {
        match &mut self.params {
            ConnParams::Ssh(p) => {
                p.password.clear();
                p.key_passphrase.clear();
            }
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
    /// 测试尚未保存的连接时为空。
    pub conn_id: String,
    pub conn_name: String,
    pub host: String,
    pub key_type: String,
    pub fingerprint: String,
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
        let ConnParams::Ssh(p) = &c.params;
        assert_eq!(p.host, "srv");
        assert_eq!(p.port, DEFAULT_SSH_PORT);
        assert_eq!(p.key_path, "", "密码认证用不到私钥路径");
        assert_eq!(c.name, "u@srv");
    }

    #[test]
    fn validate_fields() {
        let mut c = ssh("a", "");
        assert!(c.validate().is_ok());
        let ConnParams::Ssh(p) = &mut c.params;
        p.port = 70000;
        assert_eq!(c.validate().unwrap_err().i18n, "err.conn.portInvalid");
        let ConnParams::Ssh(p) = &mut c.params;
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
        let back: Connection = serde_json::from_value(v).unwrap();
        assert_eq!(back, c);

        c.production = true;
        let ConnParams::Ssh(p) = &mut c.params;
        p.incognito = true;
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["production"], true);
        assert_eq!(v["incognito"], true);
        assert_eq!(serde_json::from_value::<Connection>(v).unwrap(), c);

        let minimal: Connection =
            serde_json::from_str(r#"{"id":"x","kind":"ssh","host":"h"}"#).unwrap();
        let ConnParams::Ssh(p) = &minimal.params;
        assert_eq!(
            (p.host.as_str(), p.port, p.keepalive_seconds),
            ("h", DEFAULT_SSH_PORT, DEFAULT_KEEPALIVE)
        );
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
        let ConnParams::Ssh(p) = &req.conn.params;
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
}
