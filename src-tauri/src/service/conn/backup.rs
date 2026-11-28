//! 导出和导入本应用的数据：连接、密钥、登录用户、工具配置和系统设置合在一个 JSON 文件里，按英文 key 分开存放。
//! 连接密码、用户密码、私钥和私钥密码用导出时设置的密码加密后放在 secrets 里，其余内容是明文。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::num::NonZeroU32;
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use chrono::{DateTime, Local};
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::pbkdf2;
use serde::{Deserialize, Serialize};
use tracing::info;

use super::{ConnService, ImportSource};
use crate::config::{AppConfig, TerminalConfig, ToolsConfig, UiConfig};
use crate::core::ssh::keyfile;
use crate::error::{self, ApiError, ApiResult};
use crate::model::{Account, AuthType, ConnParams, Connection, SshKey, validate_refs};
use crate::secrets::random;

const EXPORT_APP: &str = "rain-term";
/// 改名前导出的文件，导入时仍然接受。
const LEGACY_EXPORT_APP: &str = "rain-dial";
const EXPORT_VERSION: u32 = 2;
const KDF: &str = "pbkdf2-sha256";
/// debug 构建下 PBKDF2 很慢，测试用很小的次数。
const KDF_ITERATIONS: u32 = if cfg!(test) { 1_000 } else { 600_000 };
/// 导入时不接受更大的次数，免得改坏的文件让程序长时间卡住。
const MAX_KDF_ITERATIONS: u32 = 10_000_000;
const SALT_LEN: usize = 16;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportFile {
    app: String,
    version: u32,
    exported_at: DateTime<Local>,
    connections: Vec<Connection>,
    keys: Vec<SshKey>,
    accounts: Vec<Account>,
    /// 只在导出全部时写入，单独导出连接时没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tools: Option<ToolsConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    settings: Option<Settings>,
    secrets: Sealed,
}

/// 系统设置：界面和终端。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub ui: UiConfig,
    pub terminal: TerminalConfig,
}

/// 加密后的 [`SecretPayload`]：用 PBKDF2 从导出密码得到密钥，再做 AES-256-GCM 加密。
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sealed {
    kdf: String,
    iterations: u32,
    salt: String,
    nonce: String,
    data: String,
}

/// 按导出文件里的 ID 对应的密码和私钥。
#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct SecretPayload {
    /// 连接 ID → 密码
    connections: BTreeMap<String, String>,
    keys: BTreeMap<String, KeySecret>,
    /// 登录用户 ID → 密码
    accounts: BTreeMap<String, String>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct KeySecret {
    private_key: String,
    passphrase: String,
}

/// 导出或导入的数量。导入时密钥和用户只算新增的，已经存在的直接使用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub connections: usize,
    pub keys: usize,
    pub accounts: usize,
}

/// 导入的结果。文件里带的工具配置和系统设置已经校验过，由调用方保存。
#[derive(Debug)]
pub struct Imported {
    pub counts: Counts,
    pub tools: Option<ToolsConfig>,
    pub settings: Option<Settings>,
}

fn invalid_file() -> ApiError {
    error::IMPORT_FAILED.msg(msg!("err.import.invalidFile", "文件格式不正确"))
}

impl ConnService {
    /// 导出为一个 JSON 文件。ids 为空时导出全部连接、密钥、用户、工具配置和系统设置；
    /// 导出部分连接时只带上它们的跳板机，以及这些连接用到的密钥和用户。
    pub fn export(&self, ids: &[String], cfg: &AppConfig, password: &str) -> ApiResult<(String, Counts)> {
        if password.is_empty() {
            return Err(error::EXPORT_FAILED.msg(msg!("err.export.passwordRequired", "请设置导出密码")));
        }
        let conns = self.with_jumps(ids)?;
        let mut accounts = self.accounts.list()?;
        let mut keys = self.keys.list()?;
        if !ids.is_empty() {
            let used: HashSet<&str> = conns.iter().filter_map(|c| c.account_ref()).collect();
            accounts.retain(|a| used.contains(a.id.as_str()));
            let used: HashSet<&str> = conns
                .iter()
                .filter_map(|c| c.key_ref())
                .chain(accounts.iter().filter(|a| a.auth == AuthType::Key).map(|a| a.key_id.as_str()))
                .collect();
            keys.retain(|k| used.contains(k.id.as_str()));
        }

        let mut payload = SecretPayload::default();
        let mut connections = Vec::with_capacity(conns.len());
        for mut c in conns {
            self.secrets
                .fill(&mut c)
                .map_err(|e| error::SECRET_READ_FAILED.cause(e))?;
            let password = std::mem::take(c.password_mut());
            if !password.is_empty() {
                payload.connections.insert(c.id.clone(), password);
            }
            c.clear_secrets();
            connections.push(c);
        }
        for a in &mut accounts {
            a.password.clear();
            if a.auth == AuthType::Password {
                let password = self.accounts.password(&a.id)?;
                if !password.is_empty() {
                    payload.accounts.insert(a.id.clone(), password);
                }
            }
        }
        for k in &keys {
            let (private_key, passphrase) = self.keys.material(&k.id)?;
            payload.keys.insert(k.id.clone(), KeySecret { private_key, passphrase });
        }

        let counts = Counts {
            connections: connections.len(),
            keys: keys.len(),
            accounts: accounts.len(),
        };
        let all = ids.is_empty();
        let file = ExportFile {
            app: EXPORT_APP.into(),
            version: EXPORT_VERSION,
            exported_at: Local::now(),
            connections,
            keys,
            accounts,
            tools: all.then(|| cfg.tools.clone()),
            settings: all.then(|| Settings {
                ui: cfg.ui.clone(),
                terminal: cfg.terminal.clone(),
            }),
            secrets: seal(password, &payload)?,
        };
        let text = serde_json::to_string_pretty(&file).map_err(|e| error::EXPORT_FAILED.cause(e))?;
        Ok((text, counts))
    }

    /// ids 指定的连接和它们的整条跳板链，按列表顺序；ids 为空时返回全部。
    fn with_jumps(&self, ids: &[String]) -> ApiResult<Vec<Connection>> {
        let list = self.store.list()?;
        if ids.is_empty() {
            return Ok(list);
        }
        let by_id: HashMap<&str, &Connection> = list.iter().map(|c| (c.id.as_str(), c)).collect();
        let mut keep: HashSet<&str> = HashSet::new();
        for id in ids {
            let mut cur = Some(id.as_str());
            while let Some(id) = cur {
                let Some(c) = by_id.get(id) else { break };
                if !keep.insert(id) {
                    break;
                }
                cur = c.jump_ref();
            }
        }
        Ok(list
            .iter()
            .filter(|c| keep.contains(c.id.as_str()))
            .cloned()
            .collect())
    }

    /// 读取要导入的文件并检查是不是本应用导出的，不需要密码，用于选好文件后再询问密码。
    pub fn read_import_file(path: &Path) -> ApiResult<String> {
        let bytes = std::fs::read(path).map_err(|e| error::IMPORT_FAILED.cause(e))?;
        let text = String::from_utf8(bytes).map_err(|_| invalid_file())?;
        parse(&text)?;
        Ok(text)
    }

    /// 导入本应用导出的文件。密钥和用户先导入，连接里对它们的引用随之换成本机的 ID；
    /// 写入前先检查全部内容，有一项无效时什么都不导入。
    pub fn import(&self, text: &str, password: &str) -> ApiResult<Imported> {
        let mut file = parse(text)?;
        let mut cfg = AppConfig {
            tools: file.tools.clone().unwrap_or_default(),
            ..Default::default()
        };
        if let Some(s) = &file.settings {
            cfg.ui = s.ui.clone();
            cfg.terminal = s.terminal.clone();
        }
        cfg.validate().map_err(|reason| {
            error::IMPORT_FAILED.msg(msg!("err.import.invalidSettings", "系统设置无效：{reason}", reason))
        })?;
        let mut payload = open(password, &file.secrets)?;

        let mut key_secrets = Vec::with_capacity(file.keys.len());
        for k in &file.keys {
            let s = payload.keys.remove(&k.id).unwrap_or_default();
            keyfile::decode(&s.private_key, &s.passphrase).map_err(|reason| {
                let name = &k.name;
                error::IMPORT_FAILED.msg(msg!("err.import.invalidKey", "密钥“{name}”无效：{reason}", name, reason))
            })?;
            key_secrets.push(s);
        }
        let file_keys: HashSet<&str> = file.keys.iter().map(|k| k.id.as_str()).collect();
        for a in &mut file.accounts {
            a.normalize();
            let checked = a.validate().and_then(|_| {
                let known = a.auth != AuthType::Key
                    || file_keys.contains(a.key_id.as_str())
                    || self.keys.get(&a.key_id).is_ok();
                if known {
                    Ok(())
                } else {
                    Err(msg!("err.conn.keyNotFound", "所选密钥不存在，可能已被删除"))
                }
            });
            checked.map_err(|reason| {
                let name = &a.name;
                error::IMPORT_FAILED.msg(msg!("err.import.invalidAccount", "用户“{name}”无效：{reason}", name, reason))
            })?;
        }
        let snapshot = file.connections.clone();
        for c in &mut file.connections {
            c.clear_secrets();
            c.normalize();
            c.validate().and_then(|_| validate_refs(c, &snapshot)).map_err(|reason| {
                let name = &c.name;
                error::IMPORT_FAILED.msg(msg!("err.import.invalidConn", "连接“{name}”无效：{reason}", name, reason))
            })?;
        }

        let mut counts = Counts::default();
        let mut key_ids: HashMap<String, String> = HashMap::new();
        for (k, s) in file.keys.iter().zip(key_secrets) {
            let (saved, existed) = self.keys.add(&k.name, &s.private_key, &s.passphrase)?;
            if !existed {
                counts.keys += 1;
            }
            key_ids.insert(k.id.clone(), saved.id);
        }
        let accounts = file
            .accounts
            .into_iter()
            .map(|mut a| {
                if let Some(id) = key_ids.get(&a.key_id) {
                    a.key_id = id.clone();
                }
                let password = payload.accounts.remove(&a.id).unwrap_or_default();
                (a, password)
            })
            .collect();
        let (account_ids, added) = self.accounts.import(accounts)?;
        counts.accounts = added;

        let mut conns = file.connections;
        for c in &mut conns {
            let account = c.account_ref_mut();
            if let Some(id) = account_ids.get(account.as_str()) {
                *account = id.clone();
            }
            if let ConnParams::Ssh(p) = &mut c.params
                && let Some(id) = key_ids.get(&p.key_id)
            {
                p.key_id = id.clone();
            }
            if let Some(password) = payload.connections.remove(&c.id) {
                *c.password_mut() = password;
            }
        }
        if !conns.is_empty() {
            counts.connections = self.commit_import(conns, ImportSource::Rain)?;
        }
        info!(keys = counts.keys, accounts = counts.accounts, "导入完成");
        Ok(Imported {
            counts,
            tools: file.tools,
            settings: file.settings,
        })
    }
}

fn is_our_app(app: &str) -> bool {
    app == EXPORT_APP || app == LEGACY_EXPORT_APP
}

/// 只接受本应用当前的导出格式。改名前的 rain-dial 文件同样可以导入。
fn parse(text: &str) -> ApiResult<ExportFile> {
    let text = text.trim_start_matches('\u{feff}');
    let value: serde_json::Value = serde_json::from_str(text).map_err(|_| invalid_file())?;
    let ours = value
        .get("app")
        .and_then(|v| v.as_str())
        .is_some_and(is_our_app)
        && value.get("version").and_then(|v| v.as_u64()) == Some(EXPORT_VERSION as u64);
    if !ours {
        return Err(invalid_file());
    }
    serde_json::from_value(value).map_err(|e| {
        error::IMPORT_FAILED.msg(msg!("err.import.invalidFile", "文件格式不正确").with_cause(e))
    })
}

fn derive_key(password: &str, salt: &[u8], iterations: NonZeroU32) -> LessSafeKey {
    let mut key = [0u8; 32];
    pbkdf2::derive(pbkdf2::PBKDF2_HMAC_SHA256, iterations, salt, password.as_bytes(), &mut key);
    LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &key).expect("AES-256 密钥长度固定为 32"))
}

fn seal(password: &str, payload: &SecretPayload) -> ApiResult<Sealed> {
    let failed = |e: String| error::EXPORT_FAILED.cause(e);
    let salt = random::<SALT_LEN>().map_err(failed)?;
    let nonce = random::<NONCE_LEN>().map_err(failed)?;
    let iterations = NonZeroU32::new(KDF_ITERATIONS).expect("次数大于 0");
    let key = derive_key(password, &salt, iterations);
    let mut buf = serde_json::to_vec(payload).map_err(|e| error::EXPORT_FAILED.cause(e))?;
    key.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut buf)
        .map_err(|_| failed("加密失败".into()))?;
    Ok(Sealed {
        kdf: KDF.into(),
        iterations: KDF_ITERATIONS,
        salt: B64.encode(salt),
        nonce: B64.encode(nonce),
        data: B64.encode(buf),
    })
}

fn open(password: &str, sealed: &Sealed) -> ApiResult<SecretPayload> {
    let iterations = NonZeroU32::new(sealed.iterations)
        .filter(|n| sealed.kdf == KDF && n.get() <= MAX_KDF_ITERATIONS)
        .ok_or_else(invalid_file)?;
    let salt = B64.decode(&sealed.salt).map_err(|_| invalid_file())?;
    let nonce: [u8; NONCE_LEN] = B64
        .decode(&sealed.nonce)
        .ok()
        .and_then(|n| n.try_into().ok())
        .ok_or_else(invalid_file)?;
    let mut buf = B64.decode(&sealed.data).map_err(|_| invalid_file())?;
    let key = derive_key(password, &salt, iterations);
    let plain = key
        .open_in_place(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut buf)
        .map_err(|_| error::IMPORT_WRONG_PASSWORD.err())?;
    serde_json::from_slice(plain).map_err(|_| invalid_file())
}

#[cfg(test)]
mod tests {
    use super::super::tests::{Fixture, create, fixture, with_jump, write_file};
    use super::*;
    use crate::core::ssh::keyfile::KeyKind;
    use crate::model::tests::{rdp, rdp_of, ssh, ssh_of, ssh_of_mut};
    use crate::model::{AccountScope, KeyImportReq, SaveAccountReq};

    fn import_key(f: &Fixture, passphrase: &str) -> SshKey {
        f.svc
            .keys
            .import(KeyImportReq {
                name: "k".into(),
                content: keyfile::generate(KeyKind::Ed25519, "", passphrase).unwrap(),
                passphrase: passphrase.into(),
            })
            .unwrap()
    }

    fn save_account(f: &Fixture, a: Account) -> Account {
        f.svc
            .save_account(SaveAccountReq {
                account: a,
                ..Default::default()
            })
            .unwrap()
            .account
    }

    /// 准备一份数据：跳板机 a、用登录用户 ops（带密码）经 a 登录的 b、用密钥的 k，以及无关的 x。
    fn sample(f: &Fixture) -> (Connection, Connection, Connection, SshKey) {
        let key = import_key(f, "kp");
        let ops = save_account(
            f,
            Account {
                user: "ops".into(),
                password: "apw".into(),
                ..Default::default()
            },
        );
        save_account(
            f,
            Account {
                user: "unused".into(),
                ..Default::default()
            },
        );
        let a = create(&f.svc, ssh("a", "")).conn;
        let mut b = with_jump(ssh("b", ""), &a.id);
        ssh_of_mut(&mut b).account_id = ops.id.clone();
        let b = create(&f.svc, b).conn;
        let mut k = ssh("k", "");
        let p = ssh_of_mut(&mut k);
        p.auth = AuthType::Key;
        p.key_id = key.id.clone();
        let k = create(&f.svc, k).conn;
        create(&f.svc, rdp("x", ""));
        (a, b, k, key)
    }

    #[test]
    fn export_all_and_import_elsewhere() {
        let src = fixture();
        let (_, _, _, key) = sample(&src);
        let mut cfg = AppConfig::default();
        cfg.ui.theme = "dark".into();
        cfg.terminal.font_size = 16;
        cfg.tools.sftp.sudo = true;
        let (text, counts) = src.svc.export(&[], &cfg, "export-pw").unwrap();
        assert_eq!(counts, Counts { connections: 4, keys: 1, accounts: 2 });
        for secret in ["\"pw\"", "\"apw\"", "PRIVATE KEY", "\"kp\""] {
            assert!(!text.contains(secret), "导出文件里没有明文 {secret}");
        }
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        for k in ["connections", "keys", "accounts", "tools", "settings", "secrets"] {
            assert!(value.get(k).is_some(), "导出文件有 {k}");
        }

        let dst = fixture();
        let err = dst.svc.import(&text, "wrong").unwrap_err();
        assert_eq!(err.code, error::IMPORT_WRONG_PASSWORD.0);
        assert!(dst.svc.list().unwrap().is_empty(), "密码不对时什么都不导入");

        let r = dst.svc.import(&text, "export-pw").unwrap();
        assert_eq!(r.counts, counts);
        assert_eq!(r.tools, Some(cfg.tools.clone()));
        let settings = r.settings.unwrap();
        assert_eq!(settings.ui.theme, "dark");
        assert_eq!(settings.terminal.font_size, 16);

        let list = dst.svc.list().unwrap();
        let by_name = |n: &str| list.iter().find(|v| v.conn.name == n).unwrap().conn.clone();
        let (a2, b2, k2) = (by_name("a"), by_name("b"), by_name("k"));
        assert_eq!(b2.jump_ref(), Some(a2.id.as_str()));
        let got = dst.svc.resolve(&b2.id).unwrap();
        assert_eq!((ssh_of(&got).user.as_str(), ssh_of(&got).password.as_str()), ("ops", "apw"), "登录用户和密码一起导入");
        assert_eq!(ssh_of(&dst.svc.resolve(&a2.id).unwrap()).password, "pw", "连接密码一起导入");
        assert_eq!(rdp_of(&dst.svc.resolve(&by_name("x").id).unwrap()).password, "pw");
        let got = dst.svc.resolve(&k2.id).unwrap();
        let p = ssh_of(&got);
        assert_eq!(p.key_passphrase, "kp");
        assert_eq!(p.key_data, src.svc.keys.material(&key.id).unwrap().0, "私钥一起导入");
        assert_eq!(dst.svc.keys.get(&p.key_id).unwrap().fingerprint, key.fingerprint);

        // 再导入一次：连接照常新增，同一个密钥和相同的用户不重复添加。
        let r = dst.svc.import(&text, "export-pw").unwrap();
        assert_eq!(r.counts, Counts { connections: 4, keys: 0, accounts: 0 });
        assert_eq!(dst.svc.accounts.list().unwrap().len(), 2);
        assert_eq!(dst.svc.keys.list().unwrap().len(), 1);
    }

    #[test]
    fn export_selected_takes_what_it_uses() {
        let f = fixture();
        let (a, b, _, _) = sample(&f);
        let (text, counts) = f.svc.export(std::slice::from_ref(&b.id), &AppConfig::default(), "p").unwrap();
        assert_eq!(counts, Counts { connections: 2, keys: 0, accounts: 1 }, "带上跳板机和用到的用户");
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let names: Vec<&str> = value["connections"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, [a.name.as_str(), b.name.as_str()]);
        assert_eq!(value["accounts"][0]["user"], "ops");
        assert!(value.get("tools").is_none() && value.get("settings").is_none(), "单独导出时不带工具配置和系统设置");

        let dst = fixture();
        let r = dst.svc.import(&text, "p").unwrap();
        assert_eq!(r.counts, counts);
        assert_eq!((r.tools, r.settings), (None, None), "导入时不改设置");
        assert_eq!(
            ssh_of(&dst.svc.resolve(&dst.svc.list().unwrap()[1].conn.id).unwrap()).password,
            "apw",
            "用户密码一起导入"
        );

        assert_eq!(
            f.svc.export(&[], &AppConfig::default(), "").unwrap_err().msg.i18n,
            "err.export.passwordRequired"
        );
    }

    #[test]
    fn import_checks_everything_first() {
        let f = fixture();
        let a = create(&f.svc, ssh("a", "")).conn;
        let (text, _) = f.svc.export(&[], &AppConfig::default(), "p").unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();

        let mut bad = value.clone();
        bad["connections"][0]["jumpId"] = "nope".into();
        let err = f.svc.import(&bad.to_string(), "p").unwrap_err();
        assert_eq!(err.msg.i18n, "err.import.invalidConn");

        let mut bad = value.clone();
        bad["settings"]["ui"]["theme"] = "neon".into();
        assert_eq!(f.svc.import(&bad.to_string(), "p").unwrap_err().msg.i18n, "err.import.invalidSettings");

        bad = value.clone();
        bad["accounts"] = serde_json::json!([{ "id": "x", "name": "d", "user": "d", "auth": "key", "keyId": "missing", "scope": "ssh" }]);
        assert_eq!(f.svc.import(&bad.to_string(), "p").unwrap_err().msg.i18n, "err.import.invalidAccount");
        assert_eq!(f.svc.list().unwrap().len(), 1, "有无效内容时什么都不导入");
        assert!(f.svc.accounts.list().unwrap().is_empty());

        // 文件里的明文密码不生效，只用加密的内容。
        value["connections"][0]["password"] = "plain".into();
        f.svc.import(&value.to_string(), "p").unwrap();
        let copy = f.svc.list().unwrap().into_iter().find(|v| v.conn.id != a.id).unwrap();
        assert_eq!(ssh_of(&f.svc.resolve(&copy.conn.id).unwrap()).password, "pw");
    }

    #[test]
    fn import_rejects_other_formats() {
        let f = fixture();
        let old = r#"{"app":"rain-dial","version":1,"connections":[{"kind":"ssh","host":"h","user":"u"}]}"#;
        let wrong_version = r#"{"app":"rain-term","version":1,"connections":[{"kind":"ssh","host":"h","user":"u"}]}"#;
        let array = r#"[{"kind":"ssh","host":"h","user":"u"}]"#;
        let finalshell = r#"{"id":"h1","name":"db","host":"10.0.0.1","user_name":"root","authentication_type":1,"conection_type":100}"#;
        let moba = "[Bookmarks]\nSubRep=\nbastion=#109#0%10.0.0.1%22%ops%\n";
        for text in [old, wrong_version, array, finalshell, moba, "{}"] {
            let path = write_file(&f, "other.json", text);
            let err = ConnService::read_import_file(&path).unwrap_err();
            assert_eq!(err.msg.i18n, "err.import.invalidFile", "{text}");
            assert_eq!(f.svc.import(text, "p").unwrap_err().msg.i18n, "err.import.invalidFile");
        }
        assert!(f.svc.list().unwrap().is_empty());
    }

    #[test]
    fn import_accepts_legacy_app_name() {
        let src = fixture();
        create(&src.svc, ssh("a", ""));
        let (text, _) = src.svc.export(&[], &AppConfig::default(), "p").unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["app"], "rain-term");
        value["app"] = "rain-dial".into();

        let dst = fixture();
        let r = dst.svc.import(&value.to_string(), "p").unwrap();
        assert_eq!(r.counts.connections, 1);
        let id = dst.svc.list().unwrap().remove(0).conn.id;
        assert_eq!(ssh_of(&dst.svc.resolve(&id).unwrap()).password, "pw");
    }

    #[test]
    fn key_account_survives_without_connections() {
        let src = fixture();
        let key = import_key(&src, "");
        save_account(
            &src,
            Account {
                user: "deploy".into(),
                auth: AuthType::Key,
                key_id: key.id.clone(),
                ..Default::default()
            },
        );
        let (text, counts) = src.svc.export(&[], &AppConfig::default(), "p").unwrap();
        assert_eq!(counts, Counts { connections: 0, keys: 1, accounts: 1 });

        let dst = fixture();
        let r = dst.svc.import(&text, "p").unwrap();
        assert_eq!(r.counts, counts, "没有连接时也导入密钥和用户");
        let accounts = dst.svc.accounts.list().unwrap();
        let new_key = dst.svc.keys.list().unwrap().remove(0);
        assert_eq!(accounts[0].key_id, new_key.id, "用户引用的密钥换成本机的 ID");
        assert_eq!(accounts[0].scope, AccountScope::Ssh);
    }
}
