use std::collections::HashMap;
use std::sync::Mutex;

use crate::config::KEYRING_PREFIX;
use crate::model::Connection;

pub const PASSWORD: &str = "password";
pub const KEY_PASSPHRASE: &str = "keyPassphrase";
pub const KINDS: [&str; 2] = [PASSWORD, KEY_PASSPHRASE];

/// 密码存储后端：正式运行用系统凭据库，测试用内存实现。
pub trait SecretBackend: Send + Sync {
    /// 不存在时返回空字符串。
    fn get(&self, conn_id: &str, kind: &str) -> Result<String, String>;
    fn set(&self, conn_id: &str, kind: &str, value: &str) -> Result<(), String>;
    fn delete(&self, conn_id: &str, kind: &str) -> Result<(), String>;
}

/// 按连接 ID 保存密码类信息：SSH 密码、私钥密码。
pub struct Secrets {
    backend: Box<dyn SecretBackend>,
}

impl Secrets {
    pub fn new(backend: Box<dyn SecretBackend>) -> Self {
        Secrets { backend }
    }

    pub fn get(&self, conn_id: &str, kind: &str) -> Result<String, String> {
        self.backend.get(conn_id, kind)
    }

    pub fn has(&self, conn_id: &str, kind: &str) -> bool {
        self.get(conn_id, kind).is_ok_and(|v| !v.is_empty())
    }

    /// value 为空表示删除。
    pub fn set(&self, conn_id: &str, kind: &str, value: &str) -> Result<(), String> {
        if value.is_empty() {
            self.backend.delete(conn_id, kind)
        } else {
            self.backend.set(conn_id, kind, value)
        }
    }

    /// 把已保存的密码填入连接中为空的密码类字段。
    pub fn fill(&self, conn: &mut Connection) -> Result<(), String> {
        let id = conn.id.clone();
        for (kind, value) in conn.secrets_mut() {
            if value.is_empty() {
                *value = self.get(&id, kind)?;
            }
        }
        Ok(())
    }

    pub fn delete_all(&self, conn_id: &str) -> Result<(), String> {
        let errs: Vec<String> = KINDS
            .iter()
            .filter_map(|k| self.backend.delete(conn_id, k).err())
            .collect();
        if errs.is_empty() {
            Ok(())
        } else {
            Err(errs.join("; "))
        }
    }
}

/// 系统凭据库（Windows 凭据管理器 / macOS 钥匙串 / Linux Secret Service）。
/// 服务名 "rain-dial:<连接ID>"，账户名为密码类型；Windows 上凭据名为 "服务名:账户名"，内容为 UTF-8。
pub struct KeyringBackend;

impl KeyringBackend {
    fn entry(conn_id: &str, kind: &str) -> keyring::Result<keyring::Entry> {
        let service = format!("{KEYRING_PREFIX}:{conn_id}");
        if cfg!(windows) {
            keyring::Entry::new_with_target(&format!("{service}:{kind}"), &service, kind)
        } else {
            keyring::Entry::new(&service, kind)
        }
    }
}

impl SecretBackend for KeyringBackend {
    fn get(&self, conn_id: &str, kind: &str) -> Result<String, String> {
        let entry = Self::entry(conn_id, kind).map_err(|e| e.to_string())?;
        match entry.get_secret() {
            Ok(bytes) => Ok(String::from_utf8(bytes)
                .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())),
            Err(keyring::Error::NoEntry) => Ok(String::new()),
            Err(e) => Err(e.to_string()),
        }
    }

    fn set(&self, conn_id: &str, kind: &str, value: &str) -> Result<(), String> {
        let entry = Self::entry(conn_id, kind).map_err(|e| e.to_string())?;
        entry
            .set_secret(value.as_bytes())
            .map_err(|e| e.to_string())
    }

    fn delete(&self, conn_id: &str, kind: &str) -> Result<(), String> {
        let entry = Self::entry(conn_id, kind).map_err(|e| e.to_string())?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[derive(Default)]
pub struct MemoryBackend {
    map: Mutex<HashMap<(String, String), String>>,
}

impl SecretBackend for MemoryBackend {
    fn get(&self, conn_id: &str, kind: &str) -> Result<String, String> {
        let map = self.map.lock().unwrap();
        Ok(map
            .get(&(conn_id.to_string(), kind.to_string()))
            .cloned()
            .unwrap_or_default())
    }

    fn set(&self, conn_id: &str, kind: &str, value: &str) -> Result<(), String> {
        self.map
            .lock()
            .unwrap()
            .insert((conn_id.into(), kind.into()), value.into());
        Ok(())
    }

    fn delete(&self, conn_id: &str, kind: &str) -> Result<(), String> {
        self.map
            .lock()
            .unwrap()
            .remove(&(conn_id.to_string(), kind.to_string()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_empty_deletes() {
        let s = Secrets::new(Box::new(MemoryBackend::default()));
        s.set("c1", PASSWORD, "pw").unwrap();
        assert!(s.has("c1", PASSWORD));
        s.set("c1", PASSWORD, "").unwrap();
        assert!(!s.has("c1", PASSWORD));
    }

    #[test]
    fn delete_all_only_touches_one_connection() {
        let s = Secrets::new(Box::new(MemoryBackend::default()));
        s.set("c1", PASSWORD, "a").unwrap();
        s.set("c1", KEY_PASSPHRASE, "b").unwrap();
        s.set("c2", PASSWORD, "c").unwrap();
        s.delete_all("c1").unwrap();
        assert!(!s.has("c1", PASSWORD));
        assert!(!s.has("c1", KEY_PASSPHRASE));
        assert_eq!(s.get("c2", PASSWORD).unwrap(), "c");
    }
}
