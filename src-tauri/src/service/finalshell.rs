//! FinalShell 连接导入。
//!
//! 单份导出是一个 JSON 对象；数据目录则是 `conn` 下的 `*_connect_config.json`、`folder.json`，
//! 加上根上的 `config.json`（私钥）。密码用 DES 加密后做 Base64，密钥由密文前 8 字节经
//! `java.util.Random` 派生，再取 MD5 的前 8 字节。SSH 是 `conection_type` 100，远程桌面是 101。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;
use md5::{Digest, Md5};
use serde_json::{Map, Value};
use tracing::warn;

use crate::model::{AuthType, ConnParams, Connection, RdpParams, SshParams};
use crate::util::write_file_atomic;

const FS_SSH: i32 = 100;
const FS_RDP: i32 = 101;
const FS_AUTH_KEY: i32 = 2;
const IGNORED_DIRS: &[&str] = &["backup", "deleted", "temp", "blur_image_cache"];

struct Host {
    id: String,
    name: String,
    host: String,
    port: i32,
    user: String,
    password: String,
    auth_type: i32,
    secret_key_id: String,
    conn_type: i32,
    parent_id: String,
    description: String,
    order: i64,
    delete_time: i64,
    proxy_id: String,
}

struct Folder {
    id: String,
    name: String,
    parent_id: String,
    delete_time: i64,
}

struct SecretKey {
    id: String,
    name: String,
    key_data: String,
    password: String,
}

#[derive(Default)]
struct Bundle {
    hosts: Vec<Host>,
    folders: Vec<Folder>,
    keys: Vec<SecretKey>,
}

impl Bundle {
    fn merge(&mut self, other: Bundle) {
        for f in other.folders {
            if !self.folders.iter().any(|e| e.id == f.id) {
                self.folders.push(f);
            }
        }
        for h in other.hosts {
            if h.id.is_empty() || !self.hosts.iter().any(|e| e.id == h.id) {
                self.hosts.push(h);
            }
        }
        for k in other.keys {
            if !self.keys.iter().any(|e| e.id == k.id) {
                self.keys.push(k);
            }
        }
    }
}

/// 读取选中的一个文件。主机文件只导入这一台（分组名和私钥会向父目录找）；
/// `folder.json` 或 `config.json` 则导入旁边的整棵目录。
pub(crate) fn load_file(path: &Path, keys_dir: &Path) -> Option<Vec<Connection>> {
    let text = read_text(path)?;
    let value = parse_json(&text)?;
    let bundle = bundle_from_value(&value)?;
    if bundle.hosts.is_empty() {
        let root = path.parent()?;
        return Some(load_dir(root, keys_dir));
    }
    let mut bundle = bundle;
    if let Some(dir) = path.parent() {
        bundle.merge(meta_upward(dir));
    }
    Some(to_connections(bundle, keys_dir))
}

/// 导入 FinalShell 数据目录、`conn` 目录，或其中某一层文件夹。
pub(crate) fn load_dir(root: &Path, keys_dir: &Path) -> Vec<Connection> {
    let conn = root.join("conn");
    let walk = if conn.is_dir() {
        conn.clone()
    } else {
        root.to_path_buf()
    };
    let mut bundle = Bundle::default();
    walk_into(&walk, &mut bundle, 0);
    if conn.is_dir() {
        merge_file(&mut bundle, &root.join("config.json"));
    } else if let Some(parent) = root.parent() {
        merge_file(&mut bundle, &parent.join("config.json"));
        merge_file(&mut bundle, &root.join("config.json"));
    }
    if let Some(parent) = walk.parent() {
        bundle.merge(meta_upward(parent));
    }
    to_connections(bundle, keys_dir)
}

fn parse_json(text: &str) -> Option<Value> {
    let text = text.trim_start_matches('\u{feff}').trim();
    serde_json::from_str(text).ok()
}

fn read_text(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    if bytes.len() > 32 * 1024 * 1024 {
        return None;
    }
    if let Ok(text) = std::str::from_utf8(&bytes) {
        return Some(text.to_string());
    }
    let (text, _, err) = encoding_rs::GBK.decode(&bytes);
    if err { None } else { Some(text.into_owned()) }
}

fn is_finalshell(value: &Value) -> bool {
    match value {
        Value::Array(items) if !items.is_empty() => {
            items.iter().all(|item| {
                item.as_object().is_some_and(|o| {
                    !o.contains_key("kind") && (is_host_obj(o) || is_folder_obj(o))
                })
            }) && items
                .iter()
                .any(|item| item.as_object().is_some_and(is_host_obj))
        }
        Value::Object(obj) => {
            !obj.contains_key("connections")
                && !obj.contains_key("kind")
                && (is_host_obj(obj) || is_folder_obj(obj) || is_config_obj(obj))
        }
        _ => false,
    }
}

fn is_host_obj(obj: &Map<String, Value>) -> bool {
    obj.contains_key("user_name")
        || obj.contains_key("userName")
        || obj.contains_key("conection_type")
        || obj.contains_key("conectionType")
        || obj.contains_key("connection_type")
        || obj.contains_key("authentication_type")
        || obj.contains_key("authenticationType")
        || obj.contains_key("secret_key_id")
        || obj.contains_key("secretKeyId")
}

fn is_folder_obj(obj: &Map<String, Value>) -> bool {
    !is_host_obj(obj)
        && !obj.contains_key("kind")
        && !obj.contains_key("connections")
        && !is_config_obj(obj)
        && has_any(obj, &["parent_id", "parentId"])
        && has_any(obj, &["name"])
        && obj
            .get("id")
            .is_some_and(|v| matches!(v, Value::String(s) if !s.is_empty()) || v.is_number())
}

fn is_config_obj(obj: &Map<String, Value>) -> bool {
    has_any(obj, &["secret_key_list", "secretKeyList"])
}

fn has_any(obj: &Map<String, Value>, keys: &[&str]) -> bool {
    keys.iter().any(|k| obj.contains_key(*k))
}

fn bundle_from_value(value: &Value) -> Option<Bundle> {
    if !is_finalshell(value) {
        return None;
    }
    let mut bundle = Bundle::default();
    collect(&mut bundle, value);
    Some(bundle)
}

fn collect(bundle: &mut Bundle, value: &Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect(bundle, item);
            }
        }
        Value::Object(obj) if is_config_obj(obj) => {
            let list = obj
                .get("secret_key_list")
                .or_else(|| obj.get("secretKeyList"))
                .and_then(Value::as_array);
            if let Some(list) = list {
                for item in list {
                    if let Some(key) = item.as_object().and_then(parse_key) {
                        bundle.keys.push(key);
                    }
                }
            }
        }
        Value::Object(obj) if is_host_obj(obj) => {
            if let Some(host) = parse_host(obj) {
                bundle.hosts.push(host);
            }
        }
        Value::Object(obj) if is_folder_obj(obj) => {
            if let Some(folder) = parse_folder(obj) {
                bundle.folders.push(folder);
            }
        }
        _ => {}
    }
}

fn parse_host(obj: &Map<String, Value>) -> Option<Host> {
    Some(Host {
        id: json_str(obj, &["id"]),
        name: json_str(obj, &["name"]),
        host: json_str(obj, &["host"]),
        port: json_i64(obj, &["port"]) as i32,
        user: json_str(obj, &["user_name", "userName"]),
        password: json_str(obj, &["password"]),
        auth_type: json_i64_default(obj, &["authentication_type", "authenticationType"], 1) as i32,
        secret_key_id: json_str(obj, &["secret_key_id", "secretKeyId"]),
        conn_type: json_i64_default(
            obj,
            &["conection_type", "conectionType", "connection_type"],
            i64::from(FS_SSH),
        ) as i32,
        parent_id: {
            let id = json_str(obj, &["parent_id", "parentId"]);
            if id.is_empty() { "root".into() } else { id }
        },
        description: json_str(obj, &["description"]),
        order: json_i64(obj, &["order"]),
        delete_time: json_i64(obj, &["delete_time", "deleteTime"]),
        proxy_id: {
            let id = json_str(obj, &["proxy_id", "proxyId"]);
            if id.is_empty() { "0".into() } else { id }
        },
    })
}

fn parse_folder(obj: &Map<String, Value>) -> Option<Folder> {
    let id = json_str(obj, &["id"]);
    if id.is_empty() {
        return None;
    }
    let name = json_str(obj, &["name"]);
    Some(Folder {
        id: id.clone(),
        name: if name.is_empty() { id } else { name },
        parent_id: {
            let id = json_str(obj, &["parent_id", "parentId"]);
            if id.is_empty() { "root".into() } else { id }
        },
        delete_time: json_i64(obj, &["delete_time", "deleteTime"]),
    })
}

fn parse_key(obj: &Map<String, Value>) -> Option<SecretKey> {
    let id = json_str(obj, &["id"]);
    if id.is_empty() {
        return None;
    }
    let name = json_str(obj, &["name"]);
    Some(SecretKey {
        id: id.clone(),
        name: if name.is_empty() { id } else { name },
        key_data: json_str(obj, &["key_data", "keyData"]),
        password: json_str(obj, &["password"]),
    })
}

fn json_str(obj: &Map<String, Value>, keys: &[&str]) -> String {
    for key in keys {
        match obj.get(*key) {
            Some(Value::String(s)) => return s.clone(),
            Some(Value::Number(n)) => return n.to_string(),
            _ => {}
        }
    }
    String::new()
}

fn json_i64(obj: &Map<String, Value>, keys: &[&str]) -> i64 {
    json_i64_default(obj, keys, 0)
}

fn json_i64_default(obj: &Map<String, Value>, keys: &[&str], default: i64) -> i64 {
    for key in keys {
        match obj.get(*key) {
            Some(Value::Number(n)) => return n.as_i64().unwrap_or(default),
            Some(Value::String(s)) => return s.trim().parse().unwrap_or(default),
            Some(_) => return default,
            None => {}
        }
    }
    default
}

fn walk_into(dir: &Path, bundle: &mut Bundle, depth: usize) {
    if depth > 12 {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            if IGNORED_DIRS.contains(&name.as_ref()) {
                continue;
            }
            walk_into(&entry.path(), bundle, depth + 1);
            continue;
        }
        if kind.is_file() && is_fs_filename(&name) {
            merge_file(bundle, &entry.path());
        }
    }
}

fn is_fs_filename(name: &str) -> bool {
    name == "config.json" || name == "folder.json" || name.ends_with("_connect_config.json")
}

fn merge_file(bundle: &mut Bundle, path: &Path) {
    let Some(text) = read_text(path) else { return };
    let Some(value) = parse_json(&text) else {
        return;
    };
    let Some(part) = bundle_from_value(&value) else {
        return;
    };
    bundle.merge(part);
}

/// 只收集沿途的文件夹和 config.json，不把旁边的主机也收进来。
fn meta_upward(start: &Path) -> Bundle {
    let mut bundle = Bundle::default();
    let mut dir = start.to_path_buf();
    for _ in 0..8 {
        merge_file(&mut bundle, &dir.join("folder.json"));
        merge_file(&mut bundle, &dir.join("config.json"));
        if !dir.pop() {
            break;
        }
    }
    bundle
}

fn to_connections(mut bundle: Bundle, keys_dir: &Path) -> Vec<Connection> {
    bundle
        .folders
        .retain(|f| f.delete_time == 0 && !f.id.is_empty());
    bundle.hosts.retain(|h| h.delete_time == 0);
    bundle.hosts.sort_by(|a, b| {
        a.order
            .cmp(&b.order)
            .then(a.name.cmp(&b.name))
            .then(a.id.cmp(&b.id))
    });
    let mut seen = HashSet::new();
    bundle
        .hosts
        .retain(|h| h.id.is_empty() || seen.insert(h.id.clone()));

    let folders: HashMap<&str, &Folder> =
        bundle.folders.iter().map(|f| (f.id.as_str(), f)).collect();
    let included: HashSet<&str> = bundle
        .hosts
        .iter()
        .filter(|h| {
            (h.conn_type == FS_SSH || h.conn_type == 0)
                && !h.host.trim().is_empty()
                && !h.user.trim().is_empty()
        })
        .map(|h| h.id.as_str())
        .filter(|id| !id.is_empty())
        .collect();

    let mut written: HashMap<String, (String, String)> = HashMap::new();
    let mut out = Vec::new();
    for host in &bundle.hosts {
        let rdp = host.conn_type == FS_RDP;
        if !rdp && host.conn_type != FS_SSH && host.conn_type != 0 {
            continue;
        }
        let name = host.name.trim();
        let label = if name.is_empty() {
            host.host.trim()
        } else {
            name
        };
        let params = if rdp {
            ConnParams::Rdp(RdpParams {
                host: host.host.clone(),
                port: host.port,
                user: host.user.clone(),
                ..RdpParams::default()
            })
        } else {
            ConnParams::Ssh(SshParams {
                host: host.host.clone(),
                port: host.port,
                user: host.user.clone(),
                ..SshParams::default()
            })
        };
        let mut conn = Connection {
            id: host.id.clone(),
            name: name.to_string(),
            group: group_of(&host.parent_id, &folders),
            note: host.description.trim().to_string(),
            params,
            ..Connection::default()
        };
        let jump = host.proxy_id.as_str();
        if !jump.is_empty()
            && jump != "0"
            && jump != "root"
            && jump != host.id
            && included.contains(jump)
        {
            *conn.jump_ref_mut() = jump.to_string();
        }
        let params = match &mut conn.params {
            ConnParams::Ssh(p) => p,
            ConnParams::Rdp(p) => {
                if !host.password.trim().is_empty() {
                    match decode_password(&host.password) {
                        Some(password) => p.password = password,
                        None => warn!(
                            conn = label,
                            "FinalShell 密码解密失败，连接会导入但不带密码"
                        ),
                    }
                }
                push_valid(&mut out, conn, label);
                continue;
            }
        };
        if host.auth_type == FS_AUTH_KEY {
            match key_material(&host.secret_key_id, &bundle.keys, keys_dir, &mut written) {
                Some((path, passphrase)) => {
                    params.auth = AuthType::Key;
                    params.key_path = path;
                    params.key_passphrase = passphrase;
                }
                None => {
                    if !host.secret_key_id.is_empty() {
                        warn!(
                            conn = label,
                            "FinalShell 私钥不在本次导入的文件里，改为使用本机密钥代理"
                        );
                    }
                    params.auth = AuthType::Agent;
                }
            }
        } else if host.password.trim().is_empty() {
            params.auth = AuthType::Password;
        } else if let Some(password) = decode_password(&host.password) {
            params.auth = AuthType::Password;
            params.password = password;
        } else {
            warn!(
                conn = label,
                "FinalShell 密码解密失败，连接会导入但不带密码"
            );
            params.auth = AuthType::Password;
        }
        push_valid(&mut out, conn, label);
    }
    out
}

fn push_valid(out: &mut Vec<Connection>, mut conn: Connection, label: &str) {
    conn.normalize();
    if let Err(err) = conn.validate() {
        warn!(
            conn = label,
            reason = err.text(),
            "跳过无效的 FinalShell 连接"
        );
        return;
    }
    out.push(conn);
}

fn group_of(parent_id: &str, folders: &HashMap<&str, &Folder>) -> String {
    let mut names = Vec::new();
    let mut cur = parent_id;
    let mut guard = HashSet::new();
    while !cur.is_empty() && cur != "root" && cur != "0" && guard.insert(cur) {
        let Some(folder) = folders.get(cur) else {
            break;
        };
        let name = folder.name.trim();
        if !name.is_empty() {
            names.push(name.to_string());
        }
        cur = folder.parent_id.as_str();
    }
    names.reverse();
    names.join("/")
}

fn key_material(
    id: &str,
    keys: &[SecretKey],
    keys_dir: &Path,
    written: &mut HashMap<String, (String, String)>,
) -> Option<(String, String)> {
    let id = id.trim();
    if id.is_empty() {
        return None;
    }
    if let Some(found) = written.get(id) {
        return Some(found.clone());
    }
    let key = keys.iter().find(|k| k.id == id)?;
    let pem = key_pem(&key.key_data)?;
    let path = key_path(keys_dir, id, &key.name);
    write_file_atomic(&path, pem.as_bytes()).ok()?;
    let passphrase = if key.password.trim().is_empty() {
        String::new()
    } else {
        decode_password(&key.password).unwrap_or_default()
    };
    let stored = (path.display().to_string(), passphrase);
    written.insert(id.to_string(), stored.clone());
    Some(stored)
}

fn key_pem(data: &str) -> Option<String> {
    let data = data.trim();
    if data.is_empty() {
        return None;
    }
    if data.contains("PRIVATE KEY") {
        return Some(normalize_pem(data));
    }
    let bytes = b64_decode(data)?;
    let text = String::from_utf8(bytes).ok()?;
    text.contains("PRIVATE KEY").then(|| normalize_pem(&text))
}

fn normalize_pem(text: &str) -> String {
    let text = text.replace("\\n", "\n");
    if text.ends_with('\n') {
        text
    } else {
        format!("{text}\n")
    }
}

fn key_path(dir: &Path, id: &str, name: &str) -> PathBuf {
    let slug = |s: &str, n: usize| -> String {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(n)
            .collect()
    };
    let id_part = slug(id, 64);
    let id_part = if id_part.is_empty() {
        "key".to_string()
    } else {
        id_part
    };
    let name_part = slug(name, 32);
    let file = if name_part.is_empty() {
        format!("finalshell-{id_part}.pem")
    } else {
        format!("finalshell-{name_part}-{id_part}.pem")
    };
    dir.join("keys").join(file)
}

/// 解密 FinalShell 的密码字段。解不开时返回 None，调用方仍导入这台主机。
pub(crate) fn decode_password(data: &str) -> Option<String> {
    let buf = b64_decode(data.trim())?;
    if buf.len() < 16 || (buf.len() - 8) % 8 != 0 {
        return None;
    }
    let (head, body) = buf.split_at(8);
    let key = derive_des_key(head)?;
    let plain = des_decrypt(&key, body)?;
    decode_text(&plain)
}

fn b64_decode(data: &str) -> Option<Vec<u8>> {
    let s: String = data.chars().filter(|c| !c.is_whitespace()).collect();
    if s.is_empty() {
        return None;
    }
    let standard = base64::engine::general_purpose::STANDARD;
    standard.decode(&s).ok().or_else(|| {
        base64::engine::general_purpose::STANDARD_NO_PAD
            .decode(&s)
            .ok()
    })
}

fn decode_text(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.contains(&0) {
        return None;
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Some(text.to_string());
    }
    let (text, _, err) = encoding_rs::GBK.decode(bytes);
    if err { None } else { Some(text.into_owned()) }
}

fn derive_des_key(head: &[u8]) -> Option<[u8; 8]> {
    if head.len() < 8 {
        return None;
    }
    let divisor = JavaRandom::new(signed_byte(head[5])).next_int(127);
    if divisor == 0 {
        return None;
    }
    let ks = 3680984568597093857i64 / i64::from(divisor);
    let mut random = JavaRandom::new(ks);
    let skip = signed_byte(head[0]);
    let mut i = 0i64;
    while i < skip {
        random.next_long();
        i += 1;
    }
    let n = random.next_long();
    let mut r2 = JavaRandom::new(n);
    let longs = [
        signed_byte(head[4]),
        r2.next_long(),
        signed_byte(head[7]),
        signed_byte(head[3]),
        r2.next_long(),
        signed_byte(head[1]),
        random.next_long(),
        signed_byte(head[2]),
    ];
    let mut material = [0u8; 64];
    for (i, value) in longs.iter().enumerate() {
        material[i * 8..(i + 1) * 8].copy_from_slice(&value.to_be_bytes());
    }
    let digest = Md5::digest(material);
    let mut key = [0u8; 8];
    key.copy_from_slice(&digest[..8]);
    Some(key)
}

fn signed_byte(b: u8) -> i64 {
    i64::from(b as i8)
}

fn des_decrypt(key: &[u8; 8], data: &[u8]) -> Option<Vec<u8>> {
    use des::Des;
    use ecb::Decryptor;
    use ecb::cipher::{BlockDecryptMut, KeyInit, block_padding::Pkcs7};

    let mut buf = data.to_vec();
    let cipher = Decryptor::<Des>::new_from_slice(key).ok()?;
    let plain = cipher.decrypt_padded_mut::<Pkcs7>(&mut buf).ok()?;
    Some(plain.to_vec())
}

/// 复刻 `java.util.Random` 的 48 位线性同余。
struct JavaRandom {
    seed: u64,
}

impl JavaRandom {
    fn new(seed: i64) -> Self {
        const MULTIPLIER: u64 = 0x5DEECE66D;
        const MASK: u64 = (1 << 48) - 1;
        Self {
            seed: (seed as u64 ^ MULTIPLIER) & MASK,
        }
    }

    fn next(&mut self, bits: u32) -> i32 {
        const MULTIPLIER: u64 = 0x5DEECE66D;
        const ADDEND: u64 = 0xB;
        const MASK: u64 = (1 << 48) - 1;
        self.seed = self.seed.wrapping_mul(MULTIPLIER).wrapping_add(ADDEND) & MASK;
        (self.seed >> (48 - bits)) as i32
    }

    fn next_int(&mut self, bound: i32) -> i32 {
        if bound & bound.wrapping_neg() == bound {
            return ((i64::from(bound).wrapping_mul(i64::from(self.next(31)))) >> 31) as i32;
        }
        loop {
            let bits = self.next(31);
            let val = bits % bound;
            if bits.wrapping_sub(val).wrapping_add(bound - 1) >= 0 {
                return val;
            }
        }
    }

    fn next_long(&mut self) -> i64 {
        let hi = i64::from(self.next(32));
        let lo = i64::from(self.next(32));
        (hi << 32).wrapping_add(lo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CIPHER: &str = "UU8hWV51DmVNgmX/pUd0LlaEo53VTa6s";
    const PLAIN: &str = "beac3d85988e";

    #[test]
    fn decodes_known_password() {
        assert_eq!(decode_password(CIPHER).as_deref(), Some(PLAIN));
        assert!(decode_password("").is_none());
        assert!(decode_password("@@@").is_none());
    }
}
