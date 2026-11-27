use std::io::{self, Write};
use std::path::PathBuf;

use base64::Engine;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, MutexGuard};

use crate::util::{join_host_port, restrict_permissions};

pub enum Check {
    Known,
    /// 本机没有记录过这台主机。
    Unknown,
    /// 证书和上次信任的不同，附带上次的指纹。
    Changed(String),
}

/// 记录信任过的远程桌面证书指纹，每行一条 `host:port 指纹`。
/// Windows 的自签名证书会定期更新，证书变化时由用户决定是否改为信任新证书。
pub struct CertStore {
    path: PathBuf,
    /// 校验和询问用户期间持有，避免同时打开同一台新主机时重复询问。
    lock: Mutex<()>,
}

impl CertStore {
    pub fn new(path: PathBuf) -> Self {
        CertStore {
            path,
            lock: Mutex::new(()),
        }
    }

    pub async fn lock(&self) -> MutexGuard<'_, ()> {
        self.lock.lock().await
    }

    fn read(&self) -> io::Result<String> {
        match std::fs::read_to_string(&self.path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
            r => r,
        }
    }

    pub fn check(&self, host: &str, port: u16, fingerprint: &str) -> io::Result<Check> {
        let name = key(host, port);
        let text = self.read()?;
        let found = text.lines().find_map(|line| {
            let (h, fp) = line.trim().split_once(' ')?;
            (h == name).then(|| fp.trim().to_string())
        });
        Ok(match found {
            None => Check::Unknown,
            Some(fp) if fp == fingerprint => Check::Known,
            Some(fp) => Check::Changed(fp),
        })
    }

    /// 记录主机的证书指纹，替换已有的记录。
    pub fn set(&self, host: &str, port: u16, fingerprint: &str) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let name = key(host, port);
        let text = self.read()?;
        let mut out = String::new();
        for line in text.lines() {
            if line.trim().split_once(' ').is_some_and(|(h, _)| h == name) {
                continue;
            }
            if !line.trim().is_empty() {
                out.push_str(line);
                out.push('\n');
            }
        }
        out.push_str(&format!("{name} {fingerprint}\n"));
        let mut f = std::fs::File::create(&self.path)?;
        f.write_all(out.as_bytes())?;
        restrict_permissions(&self.path);
        Ok(())
    }
}

fn key(host: &str, port: u16) -> String {
    join_host_port(&host.to_ascii_lowercase(), port)
}

/// 证书 DER 编码的 SHA-256 指纹，写法和 SSH 指纹一致。
pub fn fingerprint(der: &[u8]) -> String {
    let hash = Sha256::digest(der);
    format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(hash)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_and_replace() {
        let dir = std::env::temp_dir().join(format!("st-rdp-{}", crate::util::new_id()));
        let store = CertStore::new(dir.join("rdp_known_hosts"));
        let a = fingerprint(b"a");
        let b = fingerprint(b"b");
        assert!(a.starts_with("SHA256:") && a != b);
        assert!(matches!(store.check("Win", 3389, &a).unwrap(), Check::Unknown));
        store.set("Win", 3389, &a).unwrap();
        store.set("other", 3390, &b).unwrap();
        assert!(matches!(store.check("win", 3389, &a).unwrap(), Check::Known));
        assert!(matches!(store.check("win", 3390, &a).unwrap(), Check::Unknown));
        match store.check("win", 3389, &b).unwrap() {
            Check::Changed(old) => assert_eq!(old, a),
            _ => panic!("应识别为证书变化"),
        }
        store.set("win", 3389, &b).unwrap();
        assert!(matches!(store.check("win", 3389, &b).unwrap(), Check::Known));
        let text = std::fs::read_to_string(dir.join("rdp_known_hosts")).unwrap();
        assert_eq!(text.lines().count(), 2, "替换而不是追加");
        let _ = std::fs::remove_dir_all(dir);
    }
}
