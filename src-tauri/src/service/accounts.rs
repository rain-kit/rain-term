//! 登录用户：用户名和密码或私钥保存一份，多个连接按 ID 引用，改密码时只改一处。
//! 列表保存在 accounts.json；密码加密存进 secrets.json，私钥引用密钥管理里的密钥。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::Local;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use super::keys::KeyService;
use crate::error::{self, ApiResult};
use crate::events::{self, EventSink};
use crate::model::{Account, AccountView, AuthType, SaveAccountReq};
use crate::secrets::{self, Secrets};
use crate::util::{new_id, write_file_atomic};

const FILE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct AccountFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    accounts: Vec<Account>,
}

pub struct AccountService {
    path: PathBuf,
    lock: Mutex<()>,
    secrets: Arc<Secrets>,
    keys: Arc<KeyService>,
    events: Arc<dyn EventSink>,
}

impl AccountService {
    pub fn new(
        path: PathBuf,
        secrets: Arc<Secrets>,
        keys: Arc<KeyService>,
        events: Arc<dyn EventSink>,
    ) -> Self {
        AccountService {
            path,
            lock: Mutex::new(()),
            secrets,
            keys,
            events,
        }
    }

    /// 连接列表里显示的用户名跟着登录用户变，所以也通知连接列表刷新。
    fn changed(&self) {
        self.events
            .emit(events::ACCOUNTS_CHANGED, serde_json::Value::Null);
        self.events
            .emit(events::CONN_CHANGED, serde_json::Value::Null);
    }

    fn load(&self) -> ApiResult<Vec<Account>> {
        let data = match std::fs::read(&self.path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(error::ACCOUNT_FILE_IO.cause(e)),
        };
        if data.iter().all(u8::is_ascii_whitespace) {
            return Ok(Vec::new());
        }
        let f: AccountFile =
            serde_json::from_slice(&data).map_err(|e| error::ACCOUNT_FILE_IO.cause(e))?;
        Ok(f.accounts)
    }

    /// 读出列表交给 `f` 修改，`f` 成功后写回，整个过程持有锁。
    fn modify<T>(&self, f: impl FnOnce(&mut Vec<Account>) -> ApiResult<T>) -> ApiResult<T> {
        let _g = self.lock.lock().unwrap();
        let mut accounts = self.load()?;
        let out = f(&mut accounts)?;
        let data = serde_json::to_vec_pretty(&AccountFile {
            version: FILE_VERSION,
            accounts,
        })
        .map_err(|e| error::ACCOUNT_FILE_IO.cause(e))?;
        write_file_atomic(&self.path, &data).map_err(|e| error::ACCOUNT_FILE_IO.cause(e))?;
        Ok(out)
    }

    pub fn list(&self) -> ApiResult<Vec<Account>> {
        let _g = self.lock.lock().unwrap();
        self.load()
            .inspect_err(|e| error!(err = %e, "读取用户列表失败"))
    }

    pub fn views(&self) -> ApiResult<Vec<AccountView>> {
        Ok(self.list()?.into_iter().map(|a| self.view(a)).collect())
    }

    fn view(&self, mut account: Account) -> AccountView {
        let has_password = self.secrets.has(&account.id, secrets::PASSWORD);
        account.password.clear();
        AccountView {
            account,
            has_password,
        }
    }

    pub fn get(&self, id: &str) -> ApiResult<Account> {
        self.list()?
            .into_iter()
            .find(|a| a.id == id)
            .ok_or_else(|| error::ACCOUNT_NOT_FOUND.err())
    }

    /// 登录时使用的密码，没有保存时为空。
    pub fn password(&self, id: &str) -> ApiResult<String> {
        self.secrets
            .get(id, secrets::PASSWORD)
            .map_err(|e| error::SECRET_READ_FAILED.cause(e))
    }

    /// 校验并整理要保存的内容，不写入。修改已有用户时 ID 必须存在。
    pub fn prepare(&self, account: &mut Account) -> ApiResult<()> {
        account.normalize();
        account
            .validate()
            .map_err(|m| error::ACCOUNT_INVALID.msg(m))?;
        if account.auth == AuthType::Key {
            match self.keys.get(&account.key_id) {
                Ok(_) => {}
                Err(e) if e.code == error::KEY_NOT_FOUND.0 => {
                    return Err(error::ACCOUNT_INVALID.msg(msg!(
                        "err.conn.keyNotFound",
                        "所选密钥不存在，可能已被删除"
                    )));
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// 新建（ID 为空）或修改登录用户。内容要先经过 [`Self::prepare`]。
    /// 密码有值时保存，勾选清除时删除，否则保持原值；私钥认证不保存密码。
    pub fn save(&self, req: SaveAccountReq) -> ApiResult<AccountView> {
        let mut a = req.account;
        let is_new = a.id.is_empty();
        let password = std::mem::take(&mut a.password);
        let saved = self.modify(|list| {
            if is_new {
                a.id = new_id();
                a.created_at = Some(Local::now());
            } else {
                let old = list
                    .iter()
                    .find(|x| x.id == a.id)
                    .ok_or(error::ACCOUNT_NOT_FOUND)?;
                a.created_at = old.created_at;
            }
            let value = match a.auth {
                AuthType::Password if !password.is_empty() => Some(password.as_str()),
                AuthType::Password if !req.clear_password => None,
                _ if is_new => None,
                _ => Some(""),
            };
            if let Some(value) = value {
                self.secrets.set(&a.id, secrets::PASSWORD, value).map_err(|e| {
                    error!(account = %a.name, err = %e, "保存用户密码失败");
                    error::SECRET_STORE_FAILED.cause(e)
                })?;
            }
            match list.iter_mut().find(|x| x.id == a.id) {
                Some(slot) => *slot = a.clone(),
                None => list.push(a.clone()),
            }
            Ok(a)
        })?;
        info!(account = %saved.name, new = is_new, "登录用户已保存");
        self.changed();
        Ok(self.view(saved))
    }

    /// 导入登录用户和各自的密码，密钥 ID 要先换成本机的。返回文件里的 ID 到本机 ID 的对应和新增的数量。
    /// 名称、用户名、认证方式、连接类型和密钥都相同的用户已经存在时直接使用，不重复添加。
    pub fn import(&self, list: Vec<(Account, String)>) -> ApiResult<(HashMap<String, String>, usize)> {
        if list.is_empty() {
            return Ok((HashMap::new(), 0));
        }
        let now = Local::now();
        let (ids, added) = self.modify(|existing| {
            let mut ids = HashMap::new();
            let mut added = 0;
            for (mut a, password) in list {
                a.normalize();
                a.password.clear();
                let same = existing.iter().find(|x| {
                    (&x.name, &x.user, x.auth, x.scope, &x.key_id)
                        == (&a.name, &a.user, a.auth, a.scope, &a.key_id)
                });
                if let Some(x) = same {
                    ids.insert(a.id, x.id.clone());
                    continue;
                }
                let old = std::mem::replace(&mut a.id, new_id());
                a.created_at = Some(now);
                if a.auth == AuthType::Password && !password.is_empty() {
                    self.secrets.set(&a.id, secrets::PASSWORD, &password).map_err(|e| {
                        error!(account = %a.name, err = %e, "保存用户密码失败");
                        error::SECRET_STORE_FAILED.cause(e)
                    })?;
                }
                ids.insert(old, a.id.clone());
                existing.push(a);
                added += 1;
            }
            Ok((ids, added))
        })?;
        if added > 0 {
            info!(count = added, "登录用户已导入");
            self.changed();
        }
        Ok((ids, added))
    }

    /// 删除登录用户和它的密码。是否还被连接使用由调用方检查。
    pub fn remove(&self, id: &str) -> ApiResult<()> {
        let account = self.modify(|list| {
            let idx = list
                .iter()
                .position(|a| a.id == id)
                .ok_or_else(|| error::ACCOUNT_NOT_FOUND.err())?;
            Ok(list.remove(idx))
        })?;
        if let Err(e) = self.secrets.delete_all(id) {
            warn!(account = %account.name, err = %e, "删除用户密码失败");
        }
        info!(account = %account.name, "登录用户已删除");
        self.changed();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ssh::keyfile::{self, KeyKind};
    use crate::events::NoopSink;
    use crate::model::{AccountScope, KeyImportReq};
    use crate::secrets::MemoryBackend;

    fn service(dir: &std::path::Path) -> AccountService {
        let secrets = Arc::new(Secrets::new(Box::new(MemoryBackend::default())));
        let keys = Arc::new(KeyService::new(
            dir.join("keys.json"),
            secrets.clone(),
            Arc::new(NoopSink),
        ));
        AccountService::new(dir.join("accounts.json"), secrets, keys, Arc::new(NoopSink))
    }

    fn save(svc: &AccountService, mut a: Account, clear_password: bool) -> ApiResult<AccountView> {
        svc.prepare(&mut a)?;
        svc.save(SaveAccountReq {
            account: a,
            clear_password,
        })
    }

    #[test]
    fn save_update_remove() {
        let dir = std::env::temp_dir().join(format!("rd-accounts-{}", new_id()));
        let svc = service(&dir);
        let v = save(
            &svc,
            Account {
                user: " root ".into(),
                password: "pw".into(),
                ..Default::default()
            },
            false,
        )
        .unwrap();
        assert_eq!((v.account.name.as_str(), v.account.user.as_str()), ("root", "root"));
        assert_eq!(v.account.scope, AccountScope::All);
        assert!(v.has_password && v.account.password.is_empty());
        assert_eq!(svc.password(&v.account.id).unwrap(), "pw");
        let raw = std::fs::read_to_string(dir.join("accounts.json")).unwrap();
        assert!(!raw.contains("\"pw\""), "accounts.json 里没有密码");

        // 密码留空表示不修改，勾选清除时删除。
        let mut a = v.account.clone();
        a.name = "运维".into();
        a.scope = AccountScope::Rdp;
        let v2 = save(&svc, a.clone(), false).unwrap();
        assert_eq!(v2.account.name, "运维");
        assert_eq!(v2.account.created_at, v.account.created_at);
        assert!(v2.has_password);
        assert!(!save(&svc, a, true).unwrap().has_password);

        assert_eq!(
            save(&svc, Account::default(), false).unwrap_err().msg.i18n,
            "err.conn.userRequired"
        );
        let agent = Account {
            user: "u".into(),
            auth: AuthType::Agent,
            ..Default::default()
        };
        assert_eq!(save(&svc, agent, false).unwrap_err().msg.i18n, "err.account.authInvalid");

        svc.remove(&v.account.id).unwrap();
        assert_eq!(svc.get(&v.account.id).unwrap_err().code, error::ACCOUNT_NOT_FOUND.0);
        assert!(!svc.secrets.has(&v.account.id, secrets::PASSWORD), "删除用户时删除密码");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn key_account_is_ssh_only() {
        let dir = std::env::temp_dir().join(format!("rd-accounts-{}", new_id()));
        let svc = service(&dir);
        let key_account = |key_id: &str| Account {
            user: "deploy".into(),
            auth: AuthType::Key,
            key_id: key_id.into(),
            password: "unused".into(),
            ..Default::default()
        };
        assert_eq!(save(&svc, key_account(""), false).unwrap_err().msg.i18n, "err.conn.keyRequired");
        assert_eq!(
            save(&svc, key_account("missing"), false).unwrap_err().msg.i18n,
            "err.conn.keyNotFound"
        );
        let key = svc
            .keys
            .import(KeyImportReq {
                content: keyfile::generate(KeyKind::Ed25519, "", "").unwrap(),
                ..Default::default()
            })
            .unwrap();
        let v = save(&svc, key_account(&key.id), false).unwrap();
        assert_eq!(v.account.scope, AccountScope::Ssh, "私钥只能用于 SSH");
        assert!(!v.has_password, "私钥认证不保存密码");
        assert_eq!(v.account.key_id, key.id);
        let _ = std::fs::remove_dir_all(dir);
    }
}
