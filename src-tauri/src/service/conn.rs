use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use chrono::Local;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use super::accounts::AccountService;
use super::hostkey::HostKeyService;
use super::keys::KeyService;
use crate::core::rdp::{self, RdpTarget};
use crate::core::ssh::jump::{self, Hop};
use crate::core::ssh::keyfile::MAX_KEY_SIZE;
use crate::core::ssh::{self, SshTarget};
use crate::error::{self, ApiError, ApiResult};
use crate::events::{self, EventSink};
use crate::i18n::{Msg, pick};
use crate::model::{
    Account, AccountView, AuthType, ConnParams, ConnView, Connection, DEFAULT_RDP_HEIGHT,
    DEFAULT_RDP_WIDTH, RdpParams, SaveAccountReq, SaveConnReq, SshParams, TestResult,
    validate_refs,
};
use crate::secrets::{self, Secrets};
use crate::store::{ConnStore, StoreError};
use crate::util::{expand_home, new_id};

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::NotFound => error::CONN_NOT_FOUND.err(),
            StoreError::Json(e) => error::CONN_FILE_INVALID.cause(e),
            StoreError::Io(e) => error::CONN_FILE_IO.cause(e),
        }
    }
}

mod backup;

pub use backup::{Counts, Imported, Settings};

/// 导入连接的来源，决定文件里的密码和对不上的跳板引用怎么处理。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportSource {
    /// 本应用导出的文件：密码已经从加密内容里取出，跳板引用对不上时整体不导入。
    Rain,
    /// 密码是解密后的；proxy_id 经常不是 SSH 跳板，对不上就留空。
    FinalShell,
    /// 会话文件不含密码。
    MobaXterm,
}

pub struct ConnService {
    store: ConnStore,
    secrets: Arc<Secrets>,
    keys: Arc<KeyService>,
    accounts: Arc<AccountService>,
    host_keys: Arc<HostKeyService>,
    events: Arc<dyn EventSink>,
}

impl ConnService {
    /// 密钥管理、登录用户和连接共用 secrets，列表保存在 connections.json 旁边的 keys.json、accounts.json。
    pub fn new(
        store: ConnStore,
        secrets: Secrets,
        host_keys: Arc<HostKeyService>,
        events: Arc<dyn EventSink>,
    ) -> Self {
        let secrets = Arc::new(secrets);
        let keys = Arc::new(KeyService::new(
            store.dir().join("keys.json"),
            secrets.clone(),
            events.clone(),
        ));
        let accounts = Arc::new(AccountService::new(
            store.dir().join("accounts.json"),
            secrets.clone(),
            keys.clone(),
            events.clone(),
        ));
        ConnService {
            store,
            secrets,
            keys,
            accounts,
            host_keys,
            events,
        }
    }

    pub fn keys(&self) -> &Arc<KeyService> {
        &self.keys
    }

    pub fn accounts(&self) -> &Arc<AccountService> {
        &self.accounts
    }

    fn changed(&self) {
        self.events
            .emit(events::CONN_CHANGED, serde_json::Value::Null);
    }

    pub fn list(&self) -> ApiResult<Vec<ConnView>> {
        let list = self
            .store
            .list()
            .inspect_err(|e| error!(err = %e, "读取连接列表失败"))?;
        let accounts = self.accounts.list().unwrap_or_default();
        Ok(list.into_iter().map(|c| self.view_with(c, &accounts)).collect())
    }

    pub fn get(&self, id: &str) -> ApiResult<ConnView> {
        Ok(self.view(self.store.get(id)?))
    }

    /// 读取连接并填入已保存的密码和私钥，供建立 SSH 连接时使用。
    pub fn resolve(&self, id: &str) -> ApiResult<Connection> {
        let mut c = self.store.get(id)?;
        self.fill_secrets(&mut c)?;
        Ok(c)
    }

    fn fill_secrets(&self, c: &mut Connection) -> ApiResult<()> {
        self.secrets
            .fill(c)
            .map_err(|e| error::SECRET_READ_FAILED.cause(e))?;
        self.fill_account(c, true)?;
        self.fill_key(c)
    }

    /// 选了登录用户时填入它的用户名和认证方式，with_password 时连同密码一起填入。
    fn fill_account(&self, c: &mut Connection, with_password: bool) -> ApiResult<()> {
        let Some(id) = c.account_ref().map(str::to_string) else {
            return Ok(());
        };
        let a = match self.accounts.get(&id) {
            Ok(a) => a,
            Err(e) if e.code == error::ACCOUNT_NOT_FOUND.0 => {
                return Err(error::CONN_INVALID.msg(msg!(
                    "err.conn.accountNotFound",
                    "所选用户不存在，可能已被删除"
                )));
            }
            Err(e) => return Err(e),
        };
        if !a.scope.allows(c.kind()) {
            return Err(error::CONN_INVALID.msg(msg!(
                "err.conn.accountKind",
                "所选用户不能用于这种连接"
            )));
        }
        c.apply_account(&a);
        if with_password {
            *c.password_mut() = self.accounts.password(&id)?;
        }
        Ok(())
    }

    /// 填入所选密钥的私钥内容和私钥密码。
    fn fill_key(&self, c: &mut Connection) -> ApiResult<()> {
        let Some(id) = c.key_ref().map(str::to_string) else {
            return Ok(());
        };
        let (text, passphrase) = self.keys.material(&id)?;
        if let ConnParams::Ssh(p) = &mut c.params {
            p.key_data = text;
            p.key_passphrase = passphrase;
        }
        Ok(())
    }

    /// 私钥认证要选择密钥管理里已有的密钥。选了登录用户时密钥由登录用户检查。
    fn check_key(&self, c: &Connection) -> ApiResult<()> {
        let ConnParams::Ssh(p) = &c.params else {
            return Ok(());
        };
        if p.auth != AuthType::Key || !p.account_id.is_empty() {
            return Ok(());
        }
        if p.key_id.is_empty() {
            return Err(error::CONN_INVALID.msg(msg!("err.conn.keyRequired", "私钥认证需要选择密钥")));
        }
        match self.keys.get(&p.key_id) {
            Ok(_) => Ok(()),
            Err(e) if e.code == error::KEY_NOT_FOUND.0 => Err(error::CONN_INVALID.msg(msg!(
                "err.conn.keyNotFound",
                "所选密钥不存在，可能已被删除"
            ))),
            Err(e) => Err(e),
        }
    }

    /// 删除密钥。还有连接或登录用户在用时拒绝删除，并返回它们的名称。
    pub fn delete_key(&self, id: &str) -> ApiResult<()> {
        let users: Vec<String> = self
            .store
            .list()?
            .into_iter()
            .filter(|c| c.key_ref() == Some(id))
            .map(|c| c.name)
            .collect();
        if !users.is_empty() {
            let names = join_names(&users);
            return Err(error::KEY_IN_USE.msg(msg!(
                "err.keyInUse",
                "以下连接正在使用这个密钥，请先修改这些连接：{names}",
                names
            )));
        }
        let accounts: Vec<String> = self
            .accounts
            .list()?
            .into_iter()
            .filter(|a| a.auth == AuthType::Key && a.key_id == id)
            .map(|a| a.name)
            .collect();
        if !accounts.is_empty() {
            let names = join_names(&accounts);
            return Err(error::KEY_IN_USE.msg(msg!(
                "err.keyInUseByAccount",
                "以下用户正在使用这个密钥，请先修改这些用户：{names}",
                names
            )));
        }
        self.keys.remove(id)
    }

    /// 保存登录用户。修改连接类型或认证方式后，正在使用它的连接不能再用时拒绝保存。
    pub fn save_account(&self, mut req: SaveAccountReq) -> ApiResult<AccountView> {
        self.accounts.prepare(&mut req.account)?;
        let a = &req.account;
        if !a.id.is_empty() {
            let users: Vec<String> = self
                .store
                .list()?
                .into_iter()
                .filter(|c| c.account_ref() == Some(a.id.as_str()) && !a.scope.allows(c.kind()))
                .map(|c| c.name)
                .collect();
            if !users.is_empty() {
                let names = join_names(&users);
                return Err(error::ACCOUNT_IN_USE.msg(msg!(
                    "err.accountKindInUse",
                    "以下连接正在使用这个用户，不能改为只用于其他类型的连接：{names}",
                    names
                )));
            }
        }
        self.accounts.save(req)
    }

    /// 删除登录用户。还有连接在用时拒绝删除，并返回这些连接的名称。
    pub fn delete_account(&self, id: &str) -> ApiResult<()> {
        let users: Vec<String> = self
            .store
            .list()?
            .into_iter()
            .filter(|c| c.account_ref() == Some(id))
            .map(|c| c.name)
            .collect();
        if !users.is_empty() {
            let names = join_names(&users);
            return Err(error::ACCOUNT_IN_USE.msg(msg!(
                "err.accountInUse",
                "以下连接正在使用这个用户，请先修改这些连接：{names}",
                names
            )));
        }
        self.accounts.remove(id)
    }

    /// 读取私钥文件导入密钥管理，返回密钥 ID；同一个私钥已经导入过时返回已有的。
    fn import_key_file(&self, path: &str, passphrase: &str) -> Result<String, Msg> {
        let full = expand_home(path);
        let read_failed = |e: std::io::Error| {
            let path = full.display();
            msg!("err.ssh.keyReadFailed", "读取私钥 {path} 失败", path).with_cause(e)
        };
        let size = std::fs::metadata(&full).map_err(read_failed)?.len();
        if size > MAX_KEY_SIZE as u64 {
            return Err(msg!("err.key.notKeyFile", "不是私钥文件"));
        }
        let text = std::fs::read_to_string(&full).map_err(read_failed)?;
        let name = full
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        self.keys
            .add(&name, &text, passphrase)
            .map(|(k, _)| k.id)
            .map_err(|e| e.msg)
    }

    /// 导入的连接引用了这台电脑上没有的密钥时去掉引用，之后编辑连接重新选择。
    fn unlink_missing_key(&self, c: &mut Connection) {
        let name = c.name.clone();
        if let ConnParams::Ssh(p) = &mut c.params
            && !p.key_id.is_empty()
            && self.keys.get(&p.key_id).is_err()
        {
            warn!(conn = %name, "导入的连接引用的密钥不在这台电脑上，需要重新选择");
            p.key_id.clear();
        }
    }

    /// 导入的连接引用了这台电脑上没有（或不能用于这种连接）的登录用户时去掉引用，保留用户名，之后编辑连接重新选择。
    fn unlink_missing_account(&self, c: &mut Connection) {
        let Some(id) = c.account_ref() else { return };
        let usable = self
            .accounts
            .get(id)
            .is_ok_and(|a| a.scope.allows(c.kind()));
        if !usable {
            warn!(conn = %c.name, "导入的连接引用的登录用户不在这台电脑上，需要重新选择");
            c.account_ref_mut().clear();
        }
    }

    fn view(&self, c: Connection) -> ConnView {
        let accounts = self.accounts.list().unwrap_or_default();
        self.view_with(c, &accounts)
    }

    /// 选了登录用户时显示登录用户当前的用户名。
    fn view_with(&self, mut c: Connection, accounts: &[Account]) -> ConnView {
        if let Some(a) = c
            .account_ref()
            .and_then(|id| accounts.iter().find(|a| a.id == id))
        {
            c.apply_account(a);
        }
        let has_password = self.secrets.has(&c.id, secrets::PASSWORD);
        c.clear_secrets();
        ConnView { conn: c, has_password }
    }

    /// 新建（ID 为空）或修改连接。
    pub fn save(&self, req: SaveConnReq) -> ApiResult<ConnView> {
        let mut c = req.conn;
        let is_new = c.id.is_empty();
        self.fill_account(&mut c, false)?;
        c.normalize();
        c.validate().map_err(|e| error::CONN_INVALID.msg(e))?;
        self.check_key(&c)?;
        let saved = self.store.modify(|list| {
            if is_new {
                c.id = new_id();
                c.created_at = Some(Local::now());
            } else {
                let old = list
                    .iter()
                    .find(|x| x.id == c.id)
                    .ok_or(error::CONN_NOT_FOUND)?;
                c.created_at = old.created_at;
            }
            validate_refs(&c, list).map_err(|e| error::CONN_INVALID.msg(e))?;
            self.store_secrets(&mut c, is_new, req.clear_password)?;
            match list.iter_mut().find(|x| x.id == c.id) {
                Some(slot) => *slot = c.clone(),
                None => list.push(c.clone()),
            }
            Ok::<_, ApiError>(c)
        })?;
        info!(conn = %saved.name, kind = ?saved.kind(), new = is_new, "连接已保存");
        self.changed();
        Ok(self.view(saved))
    }

    /// 写入当前类型用得到的密码：有值时保存，勾选清除时删除，否则保持原值；用不到的种类一律删除。
    fn store_secrets(&self, c: &mut Connection, is_new: bool, clear: bool) -> ApiResult<()> {
        let id = c.id.clone();
        let name = c.name.clone();
        let used: Vec<(&str, String)> = c
            .secrets_mut()
            .into_iter()
            .map(|(k, v)| (k, std::mem::take(v)))
            .collect();
        for kind in secrets::KINDS {
            let value = match used.iter().find(|(k, _)| *k == kind) {
                Some((_, v)) if !v.is_empty() => v.as_str(),
                Some(_) if !clear => continue,
                // 新连接没有旧凭据，不用删。
                _ if is_new => continue,
                _ => "",
            };
            self.secrets.set(&id, kind, value).map_err(|e| {
                error!(conn = %name, err = %e, "保存密码失败");
                error::SECRET_STORE_FAILED.cause(e)
            })?;
        }
        Ok(())
    }

    /// 复制一个连接（含密码），名称加“副本”，放在原连接后面。
    pub fn duplicate(&self, id: &str) -> ApiResult<ConnView> {
        let mut c = self.resolve(id)?;
        c.id = new_id();
        c.created_at = Some(Local::now());
        c.name.push_str(pick([" 副本", " 副本", " Copy"]));
        // 登录用户的密码和密钥不复制到连接上。
        c.normalize();
        self.store_secrets(&mut c, true, false)?;
        let copy = c.clone();
        self.store.modify(|list| {
            let at = list
                .iter()
                .position(|x| x.id == id)
                .map_or(list.len(), |i| i + 1);
            list.insert(at, c);
            Ok::<_, StoreError>(())
        })?;
        info!(conn = %copy.name, "连接已复制");
        self.changed();
        Ok(self.view(copy))
    }

    /// 删除连接和它的密码。被其他连接用作跳板机时拒绝删除，并返回这些连接的名称。
    pub fn delete(&self, id: &str) -> ApiResult<()> {
        let name = self.store.modify(|list| {
            let idx = list
                .iter()
                .position(|c| c.id == id)
                .ok_or(error::CONN_NOT_FOUND)?;
            let users: Vec<&str> = list
                .iter()
                .filter(|c| c.jump_ref() == Some(id))
                .map(|c| c.name.as_str())
                .collect();
            if !users.is_empty() {
                let names = join_names(&users);
                return Err(error::CONN_IN_USE.msg(msg!(
                    "err.connInUse",
                    "以下连接正在使用它，请先修改这些连接：{names}",
                    names
                )));
            }
            Ok::<_, ApiError>(list.remove(idx).name)
        })?;
        if let Err(e) = self.secrets.delete_all(id) {
            warn!(conn = %name, err = %e, "删除连接密码失败");
        }
        info!(conn = %name, "连接已删除");
        self.changed();
        Ok(())
    }

    pub fn reorder(&self, ids: &[String]) -> ApiResult<()> {
        self.store.reorder(ids)?;
        self.changed();
        Ok(())
    }

    /// 拖到其他分组：改分组并按 ids 重排，一次写入。
    pub fn move_to_group(&self, id: &str, group: &str, ids: &[String]) -> ApiResult<()> {
        self.store.move_to_group(id, group, ids)?;
        info!(conn = %id, "连接已移到其他分组");
        self.changed();
        Ok(())
    }

    /// 导入 FinalShell 的数据目录、`conn` 目录或其中某一层文件夹；FinalShell 的密码会解密后保存。
    /// 选中单个 `*_connect_config.json` 时只导入这一台，并向上查找分组名和私钥。
    pub fn import_finalshell(&self, path: &Path) -> ApiResult<usize> {
        let mut import_key = |name: &str, text: &str, passphrase: &str| {
            self.keys
                .add(name, text, passphrase)
                .inspect_err(|e| warn!(key = name, err = %e.msg.text(), "FinalShell 私钥没能导入密钥管理"))
                .ok()
                .map(|(k, _)| k.id)
        };
        let conns = if path.is_dir() {
            super::finalshell::load_dir(path, &mut import_key)
        } else {
            let Some(conns) = super::finalshell::load_file(path, &mut import_key) else {
                return Err(
                    error::IMPORT_FAILED.msg(msg!("err.import.invalidFile", "文件格式不正确"))
                );
            };
            conns
        };
        self.commit_import(conns, ImportSource::FinalShell)
    }

    /// 导入 MobaXterm 的会话文件。导出的会话不含密码；会话引用的私钥文件导入密钥管理，带密码的私钥导入不了，之后再选择密钥。
    pub fn import_mobaxterm(&self, path: &Path) -> ApiResult<usize> {
        let text = read_moba_text(path)?;
        let mut imported: HashMap<String, Option<String>> = HashMap::new();
        let mut import_key = |key_path: &str| {
            imported
                .entry(key_path.to_string())
                .or_insert_with(|| {
                    self.import_key_file(key_path, "")
                        .inspect_err(|e| warn!(path = key_path, err = %e.text(), "MobaXterm 私钥没能导入密钥管理"))
                        .ok()
                })
                .clone()
        };
        let Some(conns) = super::mobaxterm::parse(&text, &mut import_key) else {
            return Err(error::IMPORT_FAILED.msg(msg!("err.import.invalidFile", "文件格式不正确")));
        };
        self.commit_import(conns, ImportSource::MobaXterm)
    }

    /// 每个连接重新生成 ID，文件内的跳板机引用随之改为新 ID；任何一个连接无效时整体不导入。
    fn commit_import(&self, mut conns: Vec<Connection>, source: ImportSource) -> ApiResult<usize> {
        if conns.is_empty() {
            return Err(error::IMPORT_FAILED.msg(msg!("err.import.empty", "文件中没有连接")));
        }
        let keep_secrets = source != ImportSource::MobaXterm;
        let now = Local::now();
        let new_ids: Vec<String> = conns.iter().map(|_| new_id()).collect();
        let mut ids: HashMap<String, String> = HashMap::new();
        for (c, id) in conns.iter().zip(&new_ids) {
            if !c.id.is_empty() {
                ids.entry(c.id.clone()).or_insert_with(|| id.clone());
            }
        }
        for (c, id) in conns.iter_mut().zip(new_ids) {
            c.id = id;
            c.created_at = Some(now);
            let r = c.jump_ref_mut();
            if let Some(new) = ids.get(r.as_str()).cloned() {
                *r = new;
            } else if source == ImportSource::FinalShell {
                r.clear();
            }
            self.unlink_missing_key(c);
            self.unlink_missing_account(c);
            if !keep_secrets {
                c.clear_secrets();
            }
            c.normalize();
        }
        let count = conns.len();
        self.store.modify(|list| {
            let merged: Vec<Connection> = list.iter().chain(conns.iter()).cloned().collect();
            for c in &conns {
                c.validate()
                    .and_then(|_| validate_refs(c, &merged))
                    .map_err(|reason| {
                        let name = &c.name;
                        error::IMPORT_FAILED.msg(msg!(
                            "err.import.invalidConn",
                            "连接“{name}”无效：{reason}",
                            name,
                            reason
                        ))
                    })?;
            }
            if keep_secrets {
                for c in &mut conns {
                    self.store_secrets(c, true, false)?;
                }
            }
            list.extend(conns);
            Ok::<_, ApiError>(())
        })?;
        info!(count, "连接已导入");
        self.changed();
        Ok(count)
    }

    /// 用编辑器里尚未保存的内容测试连接；密码留空时使用已保存的密码。
    pub async fn test(&self, req: SaveConnReq) -> ApiResult<TestResult> {
        let mut c = req.conn;
        self.fill_account(&mut c, false)?;
        c.normalize();
        c.validate().map_err(|e| error::CONN_INVALID.msg(e))?;
        self.check_key(&c)?;
        self.fill_account(&mut c, true)?;
        if !c.id.is_empty() {
            let id = c.id.clone();
            for (kind, value) in c.secrets_mut() {
                if value.is_empty() && !req.clear_password {
                    *value = self
                        .secrets
                        .get(&id, kind)
                        .map_err(|e| error::SECRET_READ_FAILED.cause(e))?;
                }
            }
        }
        self.fill_key(&mut c)?;
        validate_refs(&c, &self.store.list()?).map_err(|e| error::CONN_INVALID.msg(e))?;
        match &c.params {
            ConnParams::Ssh(p) => self.test_ssh(&c, p).await,
            ConnParams::Rdp(p) => self.test_rdp(&c, p).await,
        }
    }

    /// 只验证网络、证书和密码，不登录桌面，避免挤掉这个用户正在使用的会话。
    async fn test_rdp(&self, c: &Connection, p: &RdpParams) -> ApiResult<TestResult> {
        let cancel = CancellationToken::new();
        let opts = self.host_keys.rdp_options(&c.id, &c.name);
        let port = p.port as u16;
        let (stream, jump, jump_elapsed) = match c.jump_ref() {
            None => {
                let begin = Instant::now();
                let s = rdp::dial_tcp(&p.host, port, opts.timeout, &cancel).await?;
                (s, None, begin.elapsed())
            }
            Some(id) => {
                let j = self.resolve(id)?;
                let jp = j.ssh_params().map_err(|m| error::CONN_INVALID.msg(m))?;
                let conn = self.dial_ssh(&j, jp, false).await?;
                let begin = Instant::now();
                let s = conn
                    .client
                    .open_tunnel(&j.name, &p.host, port, opts.timeout, &cancel)
                    .await
                    .inspect_err(|_| conn.client.close())
                    .map_err(|e| error::RDP_CONNECT_FAILED.msg(e.msg()))?;
                let elapsed = conn.elapsed + begin.elapsed();
                (Box::new(s) as rdp::BoxStream, Some(conn.client), elapsed)
            }
        };
        let target = RdpTarget::from_params(p, DEFAULT_RDP_WIDTH as u32, DEFAULT_RDP_HEIGHT as u32, 100);
        let r = rdp::probe(stream, &target, &opts, &cancel, Instant::now()).await;
        if let Some(j) = jump {
            j.close();
        }
        let elapsed = r.inspect_err(
            |e| warn!(conn = %c.name, err = %e, "测试远程桌面连接失败"),
        )? + jump_elapsed;
        let elapsed_ms = elapsed.as_millis() as u64;
        info!(conn = %c.name, elapsed_ms, "测试远程桌面连接成功");
        Ok(TestResult {
            elapsed_ms,
            server_version: None,
        })
    }

    async fn test_ssh(&self, c: &Connection, p: &SshParams) -> ApiResult<TestResult> {
        let conn = self
            .dial_ssh(c, p, false)
            .await
            .inspect_err(|e| warn!(conn = %c.name, err = %e.msg.text(), "测试 SSH 连接失败"))?;
        conn.client.close();
        let elapsed_ms = conn.elapsed.as_millis() as u64;
        info!(conn = %c.name, elapsed_ms, "测试 SSH 连接成功");
        Ok(TestResult {
            elapsed_ms,
            server_version: conn.client.server_version(),
        })
    }

    /// 建立到 SSH 连接 c 的连接；设置了跳板机时按引用链依次经过各级跳板机，每一级都校验指纹。
    /// keepalive 为 true 时按各级连接自己设置的间隔开启保活。c 可以是尚未保存的编辑内容。
    pub async fn dial_ssh(
        &self,
        c: &Connection,
        p: &SshParams,
        keepalive: bool,
    ) -> ApiResult<ssh::Connected> {
        let all = self.store.list()?;
        validate_refs(c, &all).map_err(|e| error::CONN_INVALID.msg(e))?;
        // 引用链已校验过不成环，长度不会超过连接总数。
        let mut jumps = Vec::new();
        let mut next = p.jump_id.clone();
        while !next.is_empty() && jumps.len() < all.len() {
            let mut j = all
                .iter()
                .find(|x| x.id == next)
                .cloned()
                .ok_or(error::CONN_NOT_FOUND)?;
            self.fill_secrets(&mut j)?;
            next = j.jump_ref().unwrap_or_default().to_string();
            jumps.push(j);
        }

        let hop = |conn: &Connection, p: &SshParams| Hop {
            name: conn.name.clone(),
            target: SshTarget::from_params(p),
            opts: self.host_keys.dial_options(
                &conn.id,
                &conn.name,
                (keepalive && p.keepalive_seconds > 0)
                    .then(|| Duration::from_secs(p.keepalive_seconds as u64)),
            ),
        };
        let mut hops: Vec<Hop> = jumps
            .iter()
            .rev()
            .map(|j| {
                j.ssh_params()
                    .map(|jp| hop(j, jp))
                    .map_err(|m| error::CONN_INVALID.msg(m))
            })
            .collect::<ApiResult<_>>()?;
        hops.push(hop(c, p));
        Ok(jump::connect_chain(&hops, &CancellationToken::new()).await?)
    }
}

fn join_names<S: AsRef<str>>(names: &[S]) -> String {
    names
        .iter()
        .map(AsRef::as_ref)
        .collect::<Vec<_>>()
        .join(pick(["、", "、", ", "]))
}

/// MobaXterm 的会话文件不是 UTF-8 时按 Windows-1252 读。
fn read_moba_text(path: &Path) -> ApiResult<String> {
    let bytes = std::fs::read(path).map_err(|e| error::IMPORT_FAILED.cause(e))?;
    if let Ok(text) = std::str::from_utf8(&bytes) {
        return Ok(text.trim_start_matches('\u{feff}').to_string());
    }
    let (text, _, _) = encoding_rs::WINDOWS_1252.decode(&bytes);
    Ok(text.into_owned())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use super::*;
    use crate::core::rdp::cert::CertStore;
    use crate::core::ssh::keyfile::{self, KeyKind};
    use crate::core::ssh::known_hosts::HostKeyStore;
    use crate::model::tests::{rdp, rdp_of, ssh, ssh_of, ssh_of_mut};
    use crate::model::{AccountScope, AuthType, ConnKind, KeyImportReq};
    use crate::secrets::MemoryBackend;
    use base64::Engine;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<String>>);

    impl EventSink for Recorder {
        fn emit(&self, event: &str, _payload: serde_json::Value) {
            self.0.lock().unwrap().push(event.to_string());
        }
        fn show_main_window(&self) {}
    }

    pub(super) struct Fixture {
        pub(super) svc: ConnService,
        events: Arc<Recorder>,
        pub(super) dir: PathBuf,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    pub(super) fn fixture() -> Fixture {
        let dir = std::env::temp_dir().join(format!("rd-svc-{}", new_id()));
        let events = Arc::new(Recorder::default());
        let svc = ConnService::new(
            ConnStore::new(dir.join("connections.json")),
            Secrets::new(Box::new(MemoryBackend::default())),
            HostKeyService::new(
                HostKeyStore::new(dir.join("known_hosts"), vec![]),
                CertStore::new(dir.join("rdp_known_hosts")),
                events.clone(),
            ),
            events.clone(),
        );
        Fixture { svc, events, dir }
    }

    /// 以新建方式保存（清空 ID）。
    pub(super) fn create(svc: &ConnService, mut c: Connection) -> ConnView {
        c.id.clear();
        svc.save(SaveConnReq {
            conn: c,
            ..Default::default()
        })
        .unwrap()
    }

    pub(super) fn with_jump(mut c: Connection, id: &str) -> Connection {
        *c.jump_ref_mut() = id.to_string();
        c
    }

    #[test]
    fn save_get_update_with_secrets() {
        let f = fixture();
        let v = create(&f.svc, ssh("x", ""));
        assert!(!v.conn.id.is_empty());
        assert!(v.conn.created_at.is_some());
        assert!(v.has_password);
        assert_eq!(
            f.svc.resolve(&v.conn.id).unwrap().params,
            ssh("x", "").params,
            "密码单独保存，读取时填回"
        );
        assert_eq!(
            f.events.0.lock().unwrap().as_slice(),
            [events::CONN_CHANGED]
        );

        // 密码留空表示不修改。
        let mut c = f.svc.get(&v.conn.id).unwrap().conn;
        c.name = "renamed".into();
        let v2 = f
            .svc
            .save(SaveConnReq {
                conn: c.clone(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(v2.conn.name, "renamed");
        assert_eq!(v2.conn.created_at, v.conn.created_at);
        assert!(v2.has_password);

        let v3 = f
            .svc
            .save(SaveConnReq {
                conn: c.clone(),
                clear_password: true,
                ..Default::default()
            })
            .unwrap();
        assert!(!v3.has_password);

        // 切换到 agent 认证时删除用不到的密码。
        let mut c = create(&f.svc, ssh("y", "")).conn;
        let p = ssh_of_mut(&mut c);
        p.auth = AuthType::Agent;
        assert!(
            !f.svc
                .save(SaveConnReq {
                    conn: c,
                    ..Default::default()
                })
                .unwrap()
                .has_password
        );
    }

    fn key_conn(name: &str, key_id: &str) -> Connection {
        let mut c = ssh(name, "");
        let p = ssh_of_mut(&mut c);
        p.auth = AuthType::Key;
        p.key_id = key_id.into();
        c.id.clear();
        c
    }

    #[test]
    fn key_auth_uses_managed_key() {
        let f = fixture();
        let save = |c: Connection| {
            f.svc.save(SaveConnReq {
                conn: c,
                ..Default::default()
            })
        };
        assert_eq!(
            save(key_conn("k", "")).unwrap_err().msg.i18n,
            "err.conn.keyRequired"
        );
        assert_eq!(
            save(key_conn("k", "missing")).unwrap_err().msg.i18n,
            "err.conn.keyNotFound"
        );

        let text = keyfile::generate(KeyKind::Ed25519, "", "pw").unwrap();
        let key = f
            .svc
            .keys
            .import(KeyImportReq {
                name: "k".into(),
                content: text.clone(),
                passphrase: "pw".into(),
            })
            .unwrap();
        let v = save(key_conn("k", &key.id)).unwrap();
        let got = f.svc.resolve(&v.conn.id).unwrap();
        let p = ssh_of(&got);
        assert_eq!(p.key_data.trim(), text.trim());
        assert_eq!(p.key_passphrase, "pw");
        assert!(
            !serde_json::to_string(&f.svc.list().unwrap()).unwrap().contains("PRIVATE KEY"),
            "连接列表不带私钥"
        );

        let err = f.svc.delete_key(&key.id).unwrap_err();
        assert_eq!(err.code, error::KEY_IN_USE.0);
        assert_eq!(
            err.msg.params.get("names").map(|p| p.to_string()),
            Some("k".into())
        );
        f.svc.delete(&v.conn.id).unwrap();
        f.svc.delete_key(&key.id).unwrap();
        assert!(f.svc.keys.list().unwrap().is_empty());
    }

    fn save_account(f: &Fixture, a: Account) -> AccountView {
        f.svc
            .save_account(SaveAccountReq {
                account: a,
                ..Default::default()
            })
            .unwrap()
    }

    #[test]
    fn conn_uses_account_credentials() {
        let f = fixture();
        let ops = save_account(
            &f,
            Account {
                user: "ops".into(),
                password: "apw".into(),
                scope: AccountScope::Ssh,
                ..Default::default()
            },
        )
        .account;
        let mut c = ssh("a", "");
        c.id.clear();
        let p = ssh_of_mut(&mut c);
        p.account_id = ops.id.clone();
        p.user.clear();
        p.password = "own".into();
        let v = f
            .svc
            .save(SaveConnReq {
                conn: c,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(ssh_of(&v.conn).user, "ops", "用户名来自登录用户");
        assert!(!v.has_password, "选了登录用户时连接不保存自己的密码");
        let got = f.svc.resolve(&v.conn.id).unwrap();
        assert_eq!((ssh_of(&got).user.as_str(), ssh_of(&got).password.as_str()), ("ops", "apw"));

        // 复制时不把登录用户的密码写进连接。
        f.svc.duplicate(&v.conn.id).unwrap();
        let raw = std::fs::read_to_string(f.dir.join("connections.json")).unwrap();
        assert!(!raw.contains("apw"));

        // 只能用于 SSH 的用户不能给远程桌面用，也不能把在用的用户改成只用于远程桌面。
        let mut r = rdp("r", "");
        r.id.clear();
        *r.account_ref_mut() = ops.id.clone();
        let err = f
            .svc
            .save(SaveConnReq {
                conn: r,
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(err.msg.i18n, "err.conn.accountKind");
        let mut changed = ops.clone();
        changed.scope = AccountScope::Rdp;
        let err = f
            .svc
            .save_account(SaveAccountReq {
                account: changed,
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!((err.code, err.msg.i18n), (error::ACCOUNT_IN_USE.0, "err.accountKindInUse"));

        // 改登录用户的用户名，连接跟着变。
        let mut renamed = ops.clone();
        renamed.user = "ops2".into();
        save_account(&f, renamed);
        assert_eq!(ssh_of(&f.svc.get(&v.conn.id).unwrap().conn).user, "ops2");
        assert_eq!(ssh_of(&f.svc.resolve(&v.conn.id).unwrap()).password, "apw", "密码留空表示不修改");

        let err = f.svc.delete_account(&ops.id).unwrap_err();
        assert_eq!(err.code, error::ACCOUNT_IN_USE.0);
        for v in f.svc.list().unwrap() {
            f.svc.delete(&v.conn.id).unwrap();
        }
        f.svc.delete_account(&ops.id).unwrap();
        assert!(f.svc.accounts.list().unwrap().is_empty());
    }

    #[test]
    fn account_with_key() {
        let f = fixture();
        let text = keyfile::generate(KeyKind::Ed25519, "", "").unwrap();
        let key = f
            .svc
            .keys
            .import(KeyImportReq {
                content: text.clone(),
                ..Default::default()
            })
            .unwrap();
        let deploy = save_account(
            &f,
            Account {
                user: "deploy".into(),
                auth: AuthType::Key,
                key_id: key.id.clone(),
                ..Default::default()
            },
        )
        .account;
        let mut c = ssh("k", "");
        c.id.clear();
        ssh_of_mut(&mut c).account_id = deploy.id.clone();
        let v = f
            .svc
            .save(SaveConnReq {
                conn: c,
                ..Default::default()
            })
            .unwrap();
        let got = f.svc.resolve(&v.conn.id).unwrap();
        let p = ssh_of(&got);
        assert_eq!(p.auth, AuthType::Key);
        assert_eq!(p.key_data.trim(), text.trim());
        assert_eq!(f.svc.store.get(&v.conn.id).unwrap().key_ref(), None, "密钥跟着登录用户保存");

        let err = f.svc.delete_key(&key.id).unwrap_err();
        assert_eq!((err.code, err.msg.i18n), (error::KEY_IN_USE.0, "err.keyInUseByAccount"));
    }

    #[test]
    fn save_rejects_invalid() {
        let f = fixture();
        let s = create(&f.svc, ssh("s", "")).conn;

        let mut x = with_jump(ssh("x", ""), "missing");
        x.id.clear();
        let err = f
            .svc
            .save(SaveConnReq {
                conn: x,
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(
            (err.code, err.msg.i18n),
            (error::CONN_INVALID.0, "err.conn.jumpNotFound")
        );

        // s → t，再让 s 以 t 为跳板就成环。
        let t = create(&f.svc, with_jump(ssh("t", ""), &s.id)).conn;
        let err = f
            .svc
            .save(SaveConnReq {
                conn: with_jump(s.clone(), &t.id),
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(err.msg.i18n, "err.conn.jumpCycle");
        assert!(
            f.svc.get(&s.id).unwrap().conn.jump_ref().is_none(),
            "校验失败时不写入"
        );
    }

    #[test]
    fn duplicate_and_delete() {
        let f = fixture();
        let a = create(&f.svc, ssh("a", "")).conn;
        let b = create(&f.svc, ssh("b", "")).conn;
        let copy = f.svc.duplicate(&a.id).unwrap();
        assert_ne!(copy.conn.id, a.id);
        assert!(copy.conn.name.starts_with("a "));
        assert!(copy.has_password, "复制时带上密码");
        let ids: Vec<_> = f
            .svc
            .list()
            .unwrap()
            .into_iter()
            .map(|v| v.conn.id)
            .collect();
        assert_eq!(
            ids,
            [a.id.clone(), copy.conn.id.clone(), b.id.clone()],
            "副本紧跟在原连接后面"
        );

        let m = create(&f.svc, with_jump(ssh("m", ""), &a.id)).conn;
        let err = f.svc.delete(&a.id).unwrap_err();
        assert_eq!(err.code, error::CONN_IN_USE.0);
        assert_eq!(
            err.msg.params.get("names").map(|p| p.to_string()),
            Some("m".into())
        );

        f.svc.delete(&m.id).unwrap();
        f.svc.delete(&a.id).unwrap();
        assert_eq!(
            f.svc.delete(&a.id).unwrap_err().code,
            error::CONN_NOT_FOUND.0
        );
        assert!(
            !f.svc.secrets.has(&a.id, secrets::PASSWORD),
            "删除连接时删除密码"
        );
        assert_eq!(f.svc.list().unwrap().len(), 2);
    }

    #[test]
    fn reorder_list() {
        let f = fixture();
        let ids: Vec<String> = ["a", "b", "c"]
            .iter()
            .map(|n| create(&f.svc, ssh(n, "")).conn.id)
            .collect();
        f.svc.reorder(&[ids[2].clone(), ids[0].clone()]).unwrap();
        let names: Vec<_> = f
            .svc
            .list()
            .unwrap()
            .into_iter()
            .map(|v| v.conn.name)
            .collect();
        assert_eq!(names, ["c", "a", "b"]);
    }

    #[test]
    fn move_to_group() {
        let f = fixture();
        let ids: Vec<String> = ["a", "b", "c"]
            .iter()
            .map(|n| create(&f.svc, ssh(n, "")).conn.id)
            .collect();
        let order = [ids[1].clone(), ids[0].clone(), ids[2].clone()];
        f.svc.move_to_group(&ids[0], " 生产 ", &order).unwrap();
        let list = f.svc.list().unwrap();
        let names: Vec<_> = list.iter().map(|v| v.conn.name.as_str()).collect();
        assert_eq!(names, ["b", "a", "c"]);
        assert_eq!(list[1].conn.group, "生产");
        assert_eq!(list[0].conn.group, "", "其他连接的分组不变");

        f.svc.move_to_group(&ids[0], "", &order).unwrap();
        assert_eq!(f.svc.get(&ids[0]).unwrap().conn.group, "", "可以移回未分组");
        assert!(f.svc.move_to_group("missing", "x", &order).is_err());
    }

    pub(super) fn write_file(f: &Fixture, name: &str, text: &str) -> PathBuf {
        std::fs::create_dir_all(&f.dir).unwrap();
        let path = f.dir.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn import_mobaxterm_sessions() {
        let f = fixture();
        let text = "\
[Bookmarks]
SubRep=生产
bastion=#109#0%10.0.0.1%22%ops%
db=#109#0%10.0.0.2%22%root%%-1%-1%%10.0.0.1%22%ops%
win=#91#4%10.0.0.3%3389%admin%
";
        let path = write_file(&f, "sessions.mxtsessions", text);
        assert_eq!(f.svc.import_mobaxterm(&path).unwrap(), 3);
        let list = f.svc.list().unwrap();
        let bastion = list.iter().find(|v| v.conn.name == "bastion").unwrap();
        let db = list.iter().find(|v| v.conn.name == "db").unwrap();
        let win = list.iter().find(|v| v.conn.name == "win").unwrap();
        assert_eq!(bastion.conn.group, "生产");
        assert_eq!(db.conn.jump_ref(), Some(bastion.conn.id.as_str()));
        assert_eq!(win.conn.kind(), ConnKind::Rdp);
        assert_eq!(rdp_of(&win.conn).user, "admin");
        assert!(!db.conn.id.starts_with("moba-"));
        assert!(!list.iter().any(|v| v.has_password));

        let path = f.dir.join("cafe.mxtsessions");
        let mut bytes = b"[Bookmarks]\r\nSubRep=\r\ncaf".to_vec();
        bytes.push(0x80);
        bytes.extend(br"=#109#0%10.0.0.5%22%root%");
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(f.svc.import_mobaxterm(&path).unwrap(), 1);
        assert!(f.svc.list().unwrap().iter().any(|v| v.conn.name == "caf€"));

        let path = write_file(&f, "rain.json", r#"[{"kind":"ssh","host":"h","user":"u"}]"#);
        assert_eq!(
            f.svc.import_mobaxterm(&path).unwrap_err().msg.i18n,
            "err.import.invalidFile"
        );
    }

    #[test]
    fn import_mobaxterm_keys() {
        let f = fixture();
        let key = write_file(&f, "id_moba", &keyfile::generate(KeyKind::Ed25519, "", "").unwrap());
        let missing = f.dir.join("missing").display().to_string();
        let session = |name: &str, host: &str, key: &str| format!("{name}=#109#0%{host}%22%root%%-1%-1%%%%%0%0%0%{key}\n");
        let text = format!(
            "[Bookmarks]\n{}{}{}",
            session("a", "10.0.0.1", &key.display().to_string()),
            session("b", "10.0.0.2", &key.display().to_string()),
            session("c", "10.0.0.3", &missing),
        );
        let path = write_file(&f, "keys.mxtsessions", &text);
        assert_eq!(f.svc.import_mobaxterm(&path).unwrap(), 3);
        let list = f.svc.list().unwrap();
        let key_of = |name: &str| ssh_of(&list.iter().find(|v| v.conn.name == name).unwrap().conn).key_id.clone();
        let keys = f.svc.keys.list().unwrap();
        assert_eq!(keys.len(), 1, "同一个私钥文件只导入一次");
        assert_eq!(keys[0].name, "id_moba");
        assert_eq!(key_of("a"), keys[0].id);
        assert_eq!(key_of("b"), keys[0].id);
        assert_eq!(key_of("c"), "", "读不到的私钥文件留空，之后再选择密钥");
    }

    #[test]
    fn import_finalshell_keeps_decrypted_password() {
        let f = fixture();
        let import = |text: &str| {
            let path = write_file(&f, "host_connect_config.json", text);
            f.svc.import_finalshell(&path)
        };
        let text = r#"{"id":"h1","name":"db","host":"10.0.0.1","port":2222,"user_name":"root","password":"UU8hWV51DmVNgmX/pUd0LlaEo53VTa6s","authentication_type":1,"conection_type":100,"parent_id":"root","description":"主库","proxy_id":"0"}"#;
        assert_eq!(import(text).unwrap(), 1);
        let view = f.svc.list().unwrap().into_iter().next().unwrap();
        assert!(view.has_password);
        assert_eq!(view.conn.name, "db");
        assert_eq!(view.conn.note, "主库");
        let got = f.svc.resolve(&view.conn.id).unwrap();
        let p = ssh_of(&got);
        assert_eq!(
            (
                p.host.as_str(),
                p.port,
                p.user.as_str(),
                p.password.as_str(),
                p.auth
            ),
            ("10.0.0.1", 2222, "root", "beac3d85988e", AuthType::Password)
        );

        // 解不开的 FinalShell 密码不挡住这台主机。
        let plain = import(r#"{"name":"broken","host":"10.0.0.9","user_name":"root","password":"@@@@","authentication_type":1,"conection_type":100}"#).unwrap();
        assert_eq!(plain, 1);
        assert!(
            !f.svc
                .list()
                .unwrap()
                .iter()
                .any(|v| v.conn.name == "broken" && v.has_password)
        );
        assert_eq!(
            import(r#"{"name":"win","host":"10.0.0.3","user_name":"admin","password":"UU8hWV51DmVNgmX/pUd0LlaEo53VTa6s","conection_type":101}"#)
                .unwrap(),
            1
        );
        let win = f
            .svc
            .list()
            .unwrap()
            .into_iter()
            .find(|v| v.conn.name == "win")
            .unwrap();
        assert!(win.has_password);
        let resolved = f.svc.resolve(&win.conn.id).unwrap();
        let p = rdp_of(&resolved);
        assert_eq!(
            (p.host.as_str(), p.port, p.password.as_str()),
            ("10.0.0.3", 3389, "beac3d85988e")
        );
        let agent = import(r#"{"name":"via-agent","host":"10.0.0.8","user_name":"root","authentication_type":2,"secret_key_id":"missing","conection_type":100}"#).unwrap();
        assert_eq!(agent, 1);
        let got = f
            .svc
            .list()
            .unwrap()
            .into_iter()
            .find(|v| v.conn.name == "via-agent")
            .unwrap();
        let resolved = f.svc.resolve(&got.conn.id).unwrap();
        let p = ssh_of(&resolved);
        assert_eq!(p.auth, AuthType::Agent);
    }

    #[test]
    fn import_finalshell_groups_and_directory() {
        let f = fixture();
        let text = r#"[
            {"id":"a","name":"生产","parent_id":"root"},
            {"id":"b","name":"数据库","parent_id":"a"},
            {"id":"h2","name":"jump","host":"10.0.0.2","user_name":"ops","authentication_type":1,"conection_type":100,"parent_id":"a","proxy_id":"0","order":0},
            {"id":"h1","name":"db","host":"10.0.0.1","user_name":"root","password":"UU8hWV51DmVNgmX/pUd0LlaEo53VTa6s","authentication_type":1,"conection_type":100,"parent_id":"b","proxy_id":"h2","order":1},
            {"id":"rdp","name":"win","host":"10.0.0.3","user_name":"admin","conection_type":101,"parent_id":"root","proxy_id":"h2","order":2},
            {"id":"vnc","name":"other","host":"10.0.0.4","user_name":"admin","conection_type":102,"parent_id":"root"}
        ]"#;
        let path = write_file(&f, "hosts.json", text);
        assert_eq!(f.svc.import_finalshell(&path).unwrap(), 3);
        let list = f.svc.list().unwrap();
        assert_eq!(list[0].conn.name, "jump");
        assert_eq!(list[0].conn.group, "生产");
        assert_eq!(list[1].conn.name, "db");
        assert_eq!(list[1].conn.group, "生产/数据库");
        assert_eq!(list[1].conn.jump_ref(), Some(list[0].conn.id.as_str()));
        assert!(list[1].has_password);
        assert_eq!(list[2].conn.kind(), ConnKind::Rdp);
        assert_eq!(list[2].conn.jump_ref(), Some(list[0].conn.id.as_str()));

        let root = f.dir.join("fs");
        let folder = root.join("conn").join("fold");
        let host_dir = folder.join("host1");
        let other_dir = root.join("conn").join("host2");
        std::fs::create_dir_all(&host_dir).unwrap();
        std::fs::create_dir_all(&other_dir).unwrap();
        let pem = keyfile::generate(KeyKind::Ed25519, "", "beac3d85988e").unwrap();
        let pem_b64 = base64::engine::general_purpose::STANDARD.encode(&pem);
        std::fs::write(
            root.join("config.json"),
            format!(r#"{{"secret_key_list":[{{"id":"k1","name":"mykey","key_data":"{pem_b64}","password":"UU8hWV51DmVNgmX/pUd0LlaEo53VTa6s"}}]}}"#),
        )
        .unwrap();
        std::fs::write(
            folder.join("folder.json"),
            r#"{"id":"fold","name":"生产","parent_id":"root"}"#,
        )
        .unwrap();
        std::fs::write(
            host_dir.join("host1_connect_config.json"),
            r#"{"id":"host1","name":"keyed","host":"10.1.1.1","user_name":"root","authentication_type":2,"secret_key_id":"k1","conection_type":100,"parent_id":"fold"}"#,
        )
        .unwrap();
        std::fs::write(
            other_dir.join("host2_connect_config.json"),
            r#"{"id":"host2","name":"other","host":"10.1.1.2","user_name":"root","authentication_type":1,"password":"UU8hWV51DmVNgmX/pUd0LlaEo53VTa6s","conection_type":100,"parent_id":"root"}"#,
        )
        .unwrap();

        assert_eq!(
            f.svc
                .import_finalshell(&host_dir.join("host1_connect_config.json"))
                .unwrap(),
            1
        );
        let keyed = f
            .svc
            .list()
            .unwrap()
            .into_iter()
            .find(|v| v.conn.name == "keyed")
            .unwrap();
        assert_eq!(keyed.conn.group, "生产");
        let got = f.svc.resolve(&keyed.conn.id).unwrap();
        let p = ssh_of(&got);
        assert_eq!(p.auth, AuthType::Key);
        assert_eq!(p.key_passphrase, "beac3d85988e");
        assert_eq!(p.key_data, pem);
        let key = f.svc.keys.get(&p.key_id).unwrap();
        assert_eq!(key.name, "mykey");
        assert!(key.encrypted);

        assert_eq!(f.svc.import_finalshell(&root).unwrap(), 2);
        assert_eq!(f.svc.keys.list().unwrap().len(), 1, "同一个私钥不重复导入");
        let names: Vec<_> = f
            .svc
            .list()
            .unwrap()
            .into_iter()
            .filter(|v| v.conn.name == "keyed" || v.conn.name == "other")
            .map(|v| v.conn.name)
            .collect();
        assert_eq!(names, ["keyed", "keyed", "other"]);
    }
}
