//! 密钥管理：私钥导入后保存在程序的数据目录，连接按 ID 引用，不再直接读取私钥文件。
//! 列表保存在 keys.json；私钥内容和私钥密码加密存进 secrets.json。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::Local;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use crate::core::ssh::keyfile::{self, MAX_KEY_SIZE};
use crate::error::{self, ApiResult};
use crate::events::{self, EventSink};
use crate::model::{KeyImportReq, SshKey};
use crate::secrets::{self, Secrets};
use crate::util::{new_id, write_file_atomic};

const FILE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct KeyFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    keys: Vec<SshKey>,
}

pub struct KeyService {
    path: PathBuf,
    lock: Mutex<()>,
    secrets: Arc<Secrets>,
    events: Arc<dyn EventSink>,
}

impl KeyService {
    pub fn new(path: PathBuf, secrets: Arc<Secrets>, events: Arc<dyn EventSink>) -> Self {
        KeyService {
            path,
            lock: Mutex::new(()),
            secrets,
            events,
        }
    }

    fn changed(&self) {
        self.events
            .emit(events::KEYS_CHANGED, serde_json::Value::Null);
    }

    fn load(&self) -> ApiResult<Vec<SshKey>> {
        let data = match std::fs::read(&self.path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(error::KEY_FILE_IO.cause(e)),
        };
        if data.iter().all(u8::is_ascii_whitespace) {
            return Ok(Vec::new());
        }
        let f: KeyFile = serde_json::from_slice(&data).map_err(|e| error::KEY_FILE_IO.cause(e))?;
        Ok(f.keys)
    }

    /// 读出列表交给 `f` 修改，`f` 成功后写回，整个过程持有锁。
    fn modify<T>(&self, f: impl FnOnce(&mut Vec<SshKey>) -> ApiResult<T>) -> ApiResult<T> {
        let _g = self.lock.lock().unwrap();
        let mut keys = self.load()?;
        let out = f(&mut keys)?;
        let data = serde_json::to_vec_pretty(&KeyFile {
            version: FILE_VERSION,
            keys,
        })
        .map_err(|e| error::KEY_FILE_IO.cause(e))?;
        write_file_atomic(&self.path, &data).map_err(|e| error::KEY_FILE_IO.cause(e))?;
        Ok(out)
    }

    pub fn list(&self) -> ApiResult<Vec<SshKey>> {
        let _g = self.lock.lock().unwrap();
        self.load()
            .inspect_err(|e| error!(err = %e, "读取密钥列表失败"))
    }

    pub fn get(&self, id: &str) -> ApiResult<SshKey> {
        self.list()?
            .into_iter()
            .find(|k| k.id == id)
            .ok_or_else(|| error::KEY_NOT_FOUND.err())
    }

    /// 私钥内容和私钥密码，登录时使用。
    pub fn material(&self, id: &str) -> ApiResult<(String, String)> {
        self.get(id)?;
        let read = |kind| {
            self.secrets
                .get(id, kind)
                .map_err(|e| error::SECRET_READ_FAILED.cause(e))
        };
        let text = read(secrets::PRIVATE_KEY)?;
        if text.is_empty() {
            return Err(error::KEY_NOT_FOUND.err());
        }
        Ok((text, read(secrets::KEY_PASSPHRASE)?))
    }

    /// 导入用户选择或粘贴的私钥，已经导入过同一个私钥时报错。
    pub fn import(&self, req: KeyImportReq) -> ApiResult<SshKey> {
        let (key, existed) = self.add(&req.name, &req.content, &req.passphrase)?;
        if existed {
            let name = key.name;
            return Err(error::KEY_EXISTS.msg(msg!(
                "err.keyExists",
                "这个密钥已经导入过：{name}",
                name
            )));
        }
        Ok(key)
    }

    /// 加入一个私钥并返回它；指纹相同的密钥已经存在时不重复保存，返回已有的密钥和 true。
    /// name 为空时用私钥的注释，没有注释时用类型。私钥不需要密码时不保存填写的密码。
    pub fn add(&self, name: &str, text: &str, passphrase: &str) -> ApiResult<(SshKey, bool)> {
        let text = text.trim();
        if text.len() > MAX_KEY_SIZE {
            return Err(error::KEY_INVALID.msg(msg!("err.key.notKeyFile", "不是私钥文件")));
        }
        let (pk, encrypted) =
            keyfile::decode(text, passphrase).map_err(|m| error::KEY_INVALID.msg(m))?;
        let info = keyfile::info(&pk);
        let name = [name.trim(), info.comment.trim(), info.algorithm.as_str()]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or_default()
            .to_string();
        let id = new_id();
        let r = self.modify(|keys| {
            if let Some(k) = keys.iter().find(|k| k.fingerprint == info.fingerprint) {
                return Ok((k.clone(), true));
            }
            let key = SshKey {
                id: id.clone(),
                name,
                algorithm: info.algorithm,
                bits: info.bits,
                fingerprint: info.fingerprint,
                encrypted,
                created_at: Some(Local::now()),
            };
            self.store_secrets(&id, &format!("{text}\n"), if encrypted { passphrase } else { "" })?;
            keys.push(key.clone());
            Ok((key, false))
        });
        match &r {
            Ok((key, false)) => {
                info!(key = %key.name, algorithm = %key.algorithm, "密钥已导入");
                self.changed();
            }
            Ok(_) => {}
            Err(_) => self.drop_secrets(&id),
        }
        r
    }

    pub fn rename(&self, id: &str, name: &str) -> ApiResult<SshKey> {
        let name = name.trim();
        if name.is_empty() {
            return Err(error::KEY_INVALID.msg(msg!("err.key.nameRequired", "名称不能为空")));
        }
        let key = self.modify(|keys| {
            let k = keys
                .iter_mut()
                .find(|k| k.id == id)
                .ok_or_else(|| error::KEY_NOT_FOUND.err())?;
            k.name = name.to_string();
            Ok(k.clone())
        })?;
        self.changed();
        Ok(key)
    }

    /// 删除密钥和它的私钥。是否还被连接使用由调用方检查。
    pub fn remove(&self, id: &str) -> ApiResult<()> {
        let key = self.modify(|keys| {
            let idx = keys
                .iter()
                .position(|k| k.id == id)
                .ok_or_else(|| error::KEY_NOT_FOUND.err())?;
            Ok(keys.remove(idx))
        })?;
        self.drop_secrets(id);
        info!(key = %key.name, "密钥已删除");
        self.changed();
        Ok(())
    }

    fn store_secrets(&self, id: &str, text: &str, passphrase: &str) -> ApiResult<()> {
        for (kind, value) in [(secrets::PRIVATE_KEY, text), (secrets::KEY_PASSPHRASE, passphrase)] {
            self.secrets.set(id, kind, value).map_err(|e| {
                error!(err = %e, "保存私钥失败");
                error::SECRET_STORE_FAILED.cause(e)
            })?;
        }
        Ok(())
    }

    fn drop_secrets(&self, id: &str) {
        for kind in [secrets::PRIVATE_KEY, secrets::KEY_PASSPHRASE] {
            if let Err(e) = self.secrets.set(id, kind, "") {
                warn!(err = %e, "删除私钥失败");
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::core::ssh::keyfile::KeyKind;
    use crate::events::NoopSink;
    use crate::secrets::MemoryBackend;

    pub fn service(dir: &std::path::Path) -> KeyService {
        KeyService::new(
            dir.join("keys.json"),
            Arc::new(Secrets::new(Box::new(MemoryBackend::default()))),
            Arc::new(NoopSink),
        )
    }

    fn import(svc: &KeyService, name: &str, content: &str, passphrase: &str) -> ApiResult<SshKey> {
        svc.import(KeyImportReq {
            name: name.into(),
            content: content.into(),
            passphrase: passphrase.into(),
        })
    }

    #[test]
    fn import_rename_remove() {
        let dir = std::env::temp_dir().join(format!("rd-keys-{}", new_id()));
        let svc = service(&dir);
        let text = keyfile::generate(KeyKind::Ed25519, "me@pc", "pw").unwrap();

        assert_eq!(
            import(&svc, "", &text, "").unwrap_err().msg.i18n,
            "err.ssh.keyEncrypted"
        );
        let k = import(&svc, "", &text, "pw").unwrap();
        assert_eq!(k.name, "me@pc", "没填名称时用注释");
        assert!(k.encrypted);
        assert_eq!(svc.material(&k.id).unwrap(), (format!("{}\n", text.trim()), "pw".into()));
        let raw = std::fs::read_to_string(dir.join("keys.json")).unwrap();
        assert!(!raw.contains("PRIVATE KEY"), "keys.json 里没有私钥");

        let err = import(&svc, "again", &text, "pw").unwrap_err();
        assert_eq!(err.code, error::KEY_EXISTS.0);
        let (same, existed) = svc.add("again", &text, "pw").unwrap();
        assert!(existed);
        assert_eq!(same.id, k.id);
        assert_eq!(svc.list().unwrap().len(), 1);

        let plain = keyfile::generate(KeyKind::Ecdsa, "", "").unwrap();
        let p = import(&svc, " 部署 ", &plain, "unused").unwrap();
        assert_eq!(p.name, "部署");
        assert!(!p.encrypted);
        assert_eq!(svc.material(&p.id).unwrap().1, "", "不需要的密码不保存");

        assert_eq!(svc.rename(&p.id, " 新名字 ").unwrap().name, "新名字");
        assert_eq!(
            svc.rename(&p.id, " ").unwrap_err().msg.i18n,
            "err.key.nameRequired"
        );
        svc.remove(&k.id).unwrap();
        assert_eq!(svc.get(&k.id).unwrap_err().code, error::KEY_NOT_FOUND.0);
        assert_eq!(svc.material(&k.id).unwrap_err().code, error::KEY_NOT_FOUND.0);
        assert!(!svc.secrets.has(&k.id, secrets::PRIVATE_KEY), "删除密钥时删除私钥");
        let _ = std::fs::remove_dir_all(dir);
    }
}
