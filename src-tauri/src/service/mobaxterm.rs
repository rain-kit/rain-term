//! MobaXterm 会话导入。
//!
//! `.mxtsessions`、单个 `.moba`、以及配置备份里的 `[Bookmarks]` 都是同一套记录：
//! 一段以 `#` 分段、连接参数再以 `%` 分隔的文本。导入 SSH（类型 0）、SFTP（类型 7）和远程桌面（类型 4）。
//! 导出的会话文件不含密码；`<default>` 用户名用本机登录名补上。

use std::collections::HashMap;

use tracing::warn;

use crate::model::{AuthType, ConnParams, Connection, RdpParams, SshParams};

const SSH: &str = "0";
const SFTP: &str = "7";
const RDP: &str = "4";

struct Raw {
    id: String,
    name: String,
    group: String,
    note: String,
    rdp: bool,
    host: String,
    port: i32,
    user: String,
    key: String,
    init_dir: String,
    /// 从本机到目标的跳板顺序。
    gateways: Vec<Gateway>,
}

struct Gateway {
    host: String,
    port: i32,
    user: String,
    key: String,
}

/// 文本像 MobaXterm 的会话导出时返回连接；否则返回 None。
/// 文件里只有 Telnet 等不支持的会话时返回空列表。
pub(crate) fn parse(text: &str) -> Option<Vec<Connection>> {
    let text = text.trim_start_matches('\u{feff}');
    if !looks_like(text) {
        return None;
    }
    Some(to_connections(collect(text)))
}

fn looks_like(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim();
        line.starts_with("[Bookmarks")
            || connection_fields(line.split_once('=').map(|(_, v)| v).unwrap_or("")).is_some()
    })
}

fn collect(text: &str) -> Vec<Raw> {
    let mut group = String::new();
    let mut in_bookmarks = false;
    let mut saw_header = false;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') && !line.contains('=') {
            continue;
        }
        if let Some(header) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            saw_header = true;
            in_bookmarks = header == "Bookmarks" || header.starts_with("Bookmarks_");
            group.clear();
            continue;
        }
        if saw_header && !in_bookmarks {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.eq_ignore_ascii_case("SubRep") {
            group = folder_name(value);
            continue;
        }
        if key.eq_ignore_ascii_case("ImgNum") {
            continue;
        }
        let Some(raw) = raw_session(key, value, &group) else {
            continue;
        };
        out.push(raw);
    }
    out
}

fn folder_name(value: &str) -> String {
    value
        .trim()
        .split(['\\', '/'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

fn raw_session(name: &str, value: &str, group: &str) -> Option<Raw> {
    let fields = connection_fields(value)?;
    let kind = fields.first().map(String::as_str)?;
    if kind != SSH && kind != SFTP && kind != RDP {
        return None;
    }
    let host = field(&fields, 1);
    if host.is_empty() {
        return None;
    }
    let user = resolve_user(&field(&fields, 3));
    let (key, init_dir, gateways) = if kind == RDP {
        (String::new(), String::new(), Vec::new())
    } else if kind == SFTP {
        (moba_path(&field(&fields, 9)), field(&fields, 6), Vec::new())
    } else {
        (
            moba_path(&field(&fields, 14)),
            String::new(),
            gateways(&fields),
        )
    };
    Some(Raw {
        id: String::new(),
        name: name.trim().to_string(),
        group: group.to_string(),
        note: unescape(&comment_of(value)),
        rdp: kind == RDP,
        host,
        port: field(&fields, 2).parse().unwrap_or(0),
        user,
        key,
        init_dir,
        gateways,
    })
}

fn connection_fields(value: &str) -> Option<Vec<String>> {
    let value = value
        .trim()
        .trim_start_matches("; logout")
        .trim()
        .trim_start_matches(";logout")
        .trim();
    let conn = value.split('#').find(|part| part.contains('%'))?;
    let fields: Vec<String> = conn.split('%').map(|s| s.to_string()).collect();
    let kind = fields.first()?.as_str();
    matches!(kind, SSH | SFTP | "4" | "5" | "11").then_some(fields)
}

fn comment_of(value: &str) -> String {
    let value = value.trim().trim_start_matches("; logout").trim();
    let mut parts = value.split('#');
    // 开头的 # 会分出空段，图标、连接参数、终端参数之后才是备注。
    let offset = usize::from(value.starts_with('#'));
    parts.nth(4 + offset).unwrap_or("").trim().to_string()
}

fn field(fields: &[String], index: usize) -> String {
    fields
        .get(index)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn resolve_user(user: &str) -> String {
    let user = user.trim();
    if user.is_empty() || user.eq_ignore_ascii_case("<default>") {
        default_user()
    } else {
        user.to_string()
    }
}

fn default_user() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default()
}

fn moba_path(path: &str) -> String {
    let path = path.trim();
    if path.is_empty() || path == "-1" || path == "0" {
        return String::new();
    }
    path.replace("_CurrentDrive_", "C")
}

fn unescape(text: &str) -> String {
    text.replace("__DIEZE__", "#").replace("__PERCENT__", "%")
}

fn gateways(fields: &[String]) -> Vec<Gateway> {
    let hosts = split_list(&field(fields, 8));
    if hosts.is_empty() {
        return Vec::new();
    }
    let ports = split_list(&field(fields, 9));
    let users = split_list(&field(fields, 10));
    let keys = split_list(&field(fields, 15));
    hosts
        .into_iter()
        .enumerate()
        .filter_map(|(i, host)| {
            let host = host.trim();
            if host.is_empty() {
                return None;
            }
            Some(Gateway {
                host: host.to_string(),
                port: ports.get(i).and_then(|p| p.parse().ok()).unwrap_or(0),
                user: resolve_user(users.get(i).map(String::as_str).unwrap_or("")),
                key: moba_path(keys.get(i).map(String::as_str).unwrap_or("")),
            })
        })
        .collect()
}

fn split_list(value: &str) -> Vec<String> {
    if value.is_empty() {
        Vec::new()
    } else {
        value.split("__PIPE__").map(str::to_string).collect()
    }
}

fn to_connections(mut raws: Vec<Raw>) -> Vec<Connection> {
    for (i, raw) in raws.iter_mut().enumerate() {
        raw.id = format!("moba-{i}");
    }
    let mut by_endpoint: HashMap<(String, i32, String), String> = HashMap::new();
    for raw in raws.iter().filter(|r| !r.rdp) {
        let port = if raw.port == 0 { 22 } else { raw.port };
        by_endpoint
            .entry((raw.host.clone(), port, raw.user.clone()))
            .or_insert_with(|| raw.id.clone());
    }
    let mut extra = Vec::new();
    let mut next_extra = 0usize;
    let mut jumps: Vec<(String, String)> = Vec::new();
    for raw in &raws {
        let mut previous = String::new();
        for gw in &raw.gateways {
            let port = if gw.port == 0 { 22 } else { gw.port };
            let key = (gw.host.clone(), port, gw.user.clone());
            let id = if let Some(id) = by_endpoint.get(&key) {
                id.clone()
            } else {
                let id = format!("moba-gw-{next_extra}");
                next_extra += 1;
                by_endpoint.insert(key, id.clone());
                extra.push(Raw {
                    id: id.clone(),
                    name: if gw.user.is_empty() {
                        gw.host.clone()
                    } else {
                        format!("{}@{}", gw.user, gw.host)
                    },
                    group: String::new(),
                    note: String::new(),
                    rdp: false,
                    host: gw.host.clone(),
                    port: gw.port,
                    user: gw.user.clone(),
                    key: gw.key.clone(),
                    init_dir: String::new(),
                    gateways: Vec::new(),
                });
                id
            };
            if !previous.is_empty() && previous != id {
                jumps.push((id.clone(), previous));
            }
            previous = id;
        }
        if !previous.is_empty() && previous != raw.id {
            jumps.push((raw.id.clone(), previous));
        }
    }
    raws.extend(extra);
    let mut out = Vec::new();
    for raw in &raws {
        let params = if raw.rdp {
            ConnParams::Rdp(RdpParams {
                host: raw.host.clone(),
                port: raw.port,
                user: raw.user.clone(),
                ..RdpParams::default()
            })
        } else {
            ConnParams::Ssh(SshParams {
                host: raw.host.clone(),
                port: raw.port,
                user: raw.user.clone(),
                auth: if raw.key.is_empty() {
                    AuthType::Password
                } else {
                    AuthType::Key
                },
                key_path: raw.key.clone(),
                init_dir: raw.init_dir.clone(),
                jump_id: jumps
                    .iter()
                    .find(|(id, _)| id == &raw.id)
                    .map(|(_, jump)| jump.clone())
                    .unwrap_or_default(),
                ..SshParams::default()
            })
        };
        let mut conn = Connection {
            id: raw.id.clone(),
            name: raw.name.clone(),
            group: raw.group.clone(),
            note: raw.note.clone(),
            params,
            ..Connection::default()
        };
        conn.normalize();
        if let Err(err) = conn.validate() {
            warn!(conn = %raw.name, reason = err.text(), "跳过无效的 MobaXterm 会话");
            continue;
        }
        out.push(conn);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh_of(conns: &[Connection], name: &str) -> SshParams {
        let conn = conns.iter().find(|c| c.name == name).unwrap();
        crate::model::tests::ssh_of(conn).clone()
    }

    #[test]
    fn parses_sessions_groups_and_jumps() {
        let text = "\
[Bookmarks]
SubRep=生产
ImgNum=41
bastion=#109#0%10.0.0.1%22%ops%%-1%-1%%%%%0%-1%0%%%-1%0%0%0%%1080%%0%0%1#MobaFont%10%0%0%-1%15%236,236,236%30,30,30%180,180,192%0%-1%0%%xterm%-1%0%_Std_Colors_0_%80%24%0%1%-1%<none>%%0%0%-1%-1#0# #-1
db=#109#0%10.0.0.2%22%root%%-1%-1%%10.0.0.1%22%ops%0%0%0%_CurrentDrive_:\\keys\\id_rsa#MobaFont#0#主库#-1
win=#91#4%10.0.0.3%3389%admin%0
files=#140#7%10.0.0.4%22%root%%-1%/var/www%0%0%C:\\keys\\a#MobaFont#0# #-1
[Bookmarks_1]
SubRep=生产\\数据库
ImgNum=41
nested=#109#0%10.0.0.9%2222%dbuser%%-1%-1%%10.1.0.1__PIPE__10.1.0.2%22__PIPE__2222%a__PIPE__b%#MobaFont#0##-1
";
        let conns = parse(text).unwrap();
        let names: Vec<_> = conns.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"bastion"));
        assert!(names.contains(&"db"));
        assert!(names.contains(&"files"));
        assert!(names.contains(&"nested"));
        let win = conns.iter().find(|c| c.name == "win").unwrap();
        let rdp = crate::model::tests::rdp_of(win);
        assert_eq!(
            (rdp.host.as_str(), rdp.port, rdp.user.as_str()),
            ("10.0.0.3", 3389, "admin")
        );
        assert!(win.jump_ref().is_none());

        let db = conns.iter().find(|c| c.name == "db").unwrap();
        assert_eq!(db.group, "生产");
        assert_eq!(db.note, "主库");
        let db_ssh = ssh_of(&conns, "db");
        assert_eq!(db_ssh.host, "10.0.0.2");
        assert_eq!(db_ssh.user, "root");
        assert_eq!(db_ssh.auth, AuthType::Key);
        assert_eq!(db_ssh.key_path, r"C:\keys\id_rsa");
        let bastion = conns.iter().find(|c| c.name == "bastion").unwrap();
        assert_eq!(db.jump_ref(), Some(bastion.id.as_str()));

        let files = ssh_of(&conns, "files");
        assert_eq!(files.init_dir, "/var/www");
        assert_eq!(files.key_path, r"C:\keys\a");

        let nested = conns.iter().find(|c| c.name == "nested").unwrap();
        assert_eq!(nested.group, "生产/数据库");
        assert_eq!(ssh_of(&conns, "nested").port, 2222);
        let hop2 = conns
            .iter()
            .find(|c| c.id == nested.jump_ref().unwrap())
            .unwrap();
        assert_eq!(hop2.name, "b@10.1.0.2");
        let hop1 = conns
            .iter()
            .find(|c| c.id == hop2.jump_ref().unwrap())
            .unwrap();
        assert_eq!(hop1.name, "a@10.1.0.1");
        assert!(hop1.jump_ref().is_none());
    }

    #[test]
    fn rejects_other_files() {
        assert!(parse(r#"{"connections":[]}"#).is_none());
        assert!(parse("just a note").is_none());
    }
}
