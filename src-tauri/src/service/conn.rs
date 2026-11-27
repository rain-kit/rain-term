use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use super::hostkey::HostKeyService;
use crate::config::ToolsConfig;
use crate::core::ssh::jump::{self, Hop};
use crate::core::ssh::{self, SshTarget};
use crate::error::{self, ApiError, ApiResult};
use crate::events::{self, EventSink};
use crate::i18n::pick;
use crate::model::{
    ConnParams, ConnView, Connection, SaveConnReq, SshParams, TestResult, validate_refs,
};
use crate::secrets::{self, Secrets};
use crate::store::{ConnStore, StoreError};
use crate::util::new_id;

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::NotFound => error::CONN_NOT_FOUND.err(),
            StoreError::Json(e) => error::CONN_FILE_INVALID.cause(e),
            StoreError::Io(e) => error::CONN_FILE_IO.cause(e),
        }
    }
}

/// 导出文件的格式；导入时也接受只有连接数组的 JSON。
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportFile {
    app: String,
    version: u32,
    exported_at: DateTime<Local>,
    connections: Vec<Connection>,
    /// 旧版本导出的文件没有这一项。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tools: Option<ToolsConfig>,
}

/// 导入的结果：连接数量和文件里带的工具配置。
#[derive(Debug)]
pub struct Imported {
    pub count: usize,
    pub tools: Option<ToolsConfig>,
}

const EXPORT_APP: &str = "rain-dial";
const EXPORT_VERSION: u32 = 1;

pub struct ConnService {
    store: ConnStore,
    secrets: Secrets,
    host_keys: Arc<HostKeyService>,
    events: Arc<dyn EventSink>,
}

impl ConnService {
    pub fn new(
        store: ConnStore,
        secrets: Secrets,
        host_keys: Arc<HostKeyService>,
        events: Arc<dyn EventSink>,
    ) -> Self {
        ConnService {
            store,
            secrets,
            host_keys,
            events,
        }
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
        Ok(list.into_iter().map(|c| self.view(c)).collect())
    }

    pub fn get(&self, id: &str) -> ApiResult<ConnView> {
        Ok(self.view(self.store.get(id)?))
    }

    /// 读取连接并填入已保存的密码，供建立 SSH 连接时使用。
    pub fn resolve(&self, id: &str) -> ApiResult<Connection> {
        let mut c = self.store.get(id)?;
        self.secrets
            .fill(&mut c)
            .map_err(|e| error::SECRET_READ_FAILED.cause(e))?;
        Ok(c)
    }

    fn view(&self, mut c: Connection) -> ConnView {
        let has_password = self.secrets.has(&c.id, secrets::PASSWORD);
        let has_key_passphrase = self.secrets.has(&c.id, secrets::KEY_PASSPHRASE);
        c.clear_secrets();
        ConnView {
            conn: c,
            has_password,
            has_key_passphrase,
        }
    }

    /// 新建（ID 为空）或修改连接。
    pub fn save(&self, req: SaveConnReq) -> ApiResult<ConnView> {
        let mut c = req.conn;
        let is_new = c.id.is_empty();
        c.normalize();
        c.validate().map_err(|e| error::CONN_INVALID.msg(e))?;
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
            self.store_secrets(&mut c, is_new, req.clear_password, req.clear_key_passphrase)?;
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
    fn store_secrets(
        &self,
        c: &mut Connection,
        is_new: bool,
        clear_password: bool,
        clear_passphrase: bool,
    ) -> ApiResult<()> {
        let id = c.id.clone();
        let name = c.name.clone();
        let used: Vec<(&str, String)> = c
            .secrets_mut()
            .into_iter()
            .map(|(k, v)| (k, std::mem::take(v)))
            .collect();
        for kind in secrets::KINDS {
            let clear = if kind == secrets::PASSWORD {
                clear_password
            } else {
                clear_passphrase
            };
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
        self.store_secrets(&mut c, true, false, false)?;
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
                let names = users.join(pick(["、", "、", ", "]));
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

    /// 导出为 JSON，不含密码，附带工具配置。ids 为空时导出全部；导出部分连接时自动带上它们的跳板机。
    pub fn export(&self, ids: &[String], tools: &ToolsConfig) -> ApiResult<(String, usize)> {
        let list = self.store.list()?;
        let selected: Vec<Connection> = if ids.is_empty() {
            list
        } else {
            let by_id: HashMap<&str, &Connection> =
                list.iter().map(|c| (c.id.as_str(), c)).collect();
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
            list.iter()
                .filter(|c| keep.contains(c.id.as_str()))
                .cloned()
                .collect()
        };
        let count = selected.len();
        let file = ExportFile {
            app: EXPORT_APP.into(),
            version: EXPORT_VERSION,
            exported_at: Local::now(),
            connections: selected
                .into_iter()
                .map(|mut c| {
                    c.clear_secrets();
                    c
                })
                .collect(),
            tools: Some(tools.clone()),
        };
        let text =
            serde_json::to_string_pretty(&file).map_err(|e| error::EXPORT_FAILED.cause(e))?;
        Ok((text, count))
    }

    /// 导入本应用导出的连接，返回导入的数量。其他终端的格式走各自的导入函数。
    /// 每个连接重新生成 ID，文件内的跳板机引用随之改为新 ID；任何一个连接无效时整体不导入。
    /// 文件里即使写了密码也会被忽略。工具配置原样返回，由调用方保存。
    pub fn import(&self, text: &str) -> ApiResult<Imported> {
        let (conns, tools) = parse_import(text)?;
        let count = self.commit_import(conns, false)?;
        Ok(Imported { count, tools })
    }

    /// 从本应用导出的文件导入。
    pub fn import_file(&self, path: &Path) -> ApiResult<Imported> {
        let bytes = std::fs::read(path).map_err(|e| error::IMPORT_FAILED.cause(e))?;
        let Ok(text) = std::str::from_utf8(&bytes) else {
            return Err(error::IMPORT_FAILED.msg(msg!("err.import.invalidFile", "文件格式不正确")));
        };
        self.import(text)
    }

    /// 导入 FinalShell 的数据目录、`conn` 目录或其中某一层文件夹；FinalShell 的密码会解密后写入凭据库。
    /// 选中单个 `*_connect_config.json` 时只导入这一台，并向上查找分组名和私钥。
    pub fn import_finalshell(&self, path: &Path) -> ApiResult<usize> {
        let keys_dir = self.store.dir();
        let conns = if path.is_dir() {
            super::finalshell::load_dir(path, &keys_dir)
        } else {
            let Some(conns) = super::finalshell::load_file(path, &keys_dir) else {
                return Err(
                    error::IMPORT_FAILED.msg(msg!("err.import.invalidFile", "文件格式不正确"))
                );
            };
            conns
        };
        self.commit_import(conns, true)
    }

    /// 导入 MobaXterm 的会话文件。导出的会话不含密码。
    pub fn import_mobaxterm(&self, path: &Path) -> ApiResult<usize> {
        let text = read_moba_text(path)?;
        let Some(conns) = super::mobaxterm::parse(&text) else {
            return Err(error::IMPORT_FAILED.msg(msg!("err.import.invalidFile", "文件格式不正确")));
        };
        self.commit_import(conns, false)
    }

    /// keep_secrets 为 true 时（FinalShell）保留解密后的密码，并清掉对不上的跳板引用。
    fn commit_import(&self, mut conns: Vec<Connection>, keep_secrets: bool) -> ApiResult<usize> {
        if conns.is_empty() {
            return Err(error::IMPORT_FAILED.msg(msg!("err.import.empty", "文件中没有连接")));
        }
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
            } else if keep_secrets {
                // FinalShell 的 proxy_id 经常不是 SSH 跳板，对不上就留空，不让整批失败。
                r.clear();
            }
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
                    self.store_secrets(c, true, false, false)?;
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
        c.normalize();
        c.validate().map_err(|e| error::CONN_INVALID.msg(e))?;
        if !c.id.is_empty() {
            let id = c.id.clone();
            for (kind, value) in c.secrets_mut() {
                let clear = if kind == secrets::PASSWORD {
                    req.clear_password
                } else {
                    req.clear_key_passphrase
                };
                if value.is_empty() && !clear {
                    *value = self
                        .secrets
                        .get(&id, kind)
                        .map_err(|e| error::SECRET_READ_FAILED.cause(e))?;
                }
            }
        }
        validate_refs(&c, &self.store.list()?).map_err(|e| error::CONN_INVALID.msg(e))?;
        match &c.params {
            ConnParams::Ssh(p) => self.test_ssh(&c, p).await,
        }
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
            self.secrets
                .fill(&mut j)
                .map_err(|e| error::SECRET_READ_FAILED.cause(e))?;
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
            .map(|j| match &j.params {
                ConnParams::Ssh(jp) => hop(j, jp),
            })
            .collect();
        hops.push(hop(c, p));
        Ok(jump::connect_chain(&hops, &CancellationToken::new()).await?)
    }
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

fn parse_import(text: &str) -> ApiResult<(Vec<Connection>, Option<ToolsConfig>)> {
    let invalid = |e: serde_json::Error| {
        error::IMPORT_FAILED.msg(msg!("err.import.invalidFile", "文件格式不正确").with_cause(e))
    };
    let text = text.trim_start_matches('\u{feff}');
    let value: serde_json::Value = serde_json::from_str(text).map_err(invalid)?;
    let (list, tools) = match value {
        serde_json::Value::Array(_) => (value, None),
        serde_json::Value::Object(mut obj) => match obj.remove("connections") {
            Some(v) => (v, obj.remove("tools")),
            None => {
                return Err(
                    error::IMPORT_FAILED.msg(msg!("err.import.invalidFile", "文件格式不正确"))
                );
            }
        },
        _ => return Err(error::IMPORT_FAILED.msg(msg!("err.import.invalidFile", "文件格式不正确"))),
    };
    let tools = tools
        .map(serde_json::from_value)
        .transpose()
        .map_err(invalid)?;
    Ok((serde_json::from_value(list).map_err(invalid)?, tools))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use super::*;
    use crate::core::ssh::known_hosts::HostKeyStore;
    use crate::model::tests::ssh;
    use crate::model::{AuthType, ConnParams};
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

    struct Fixture {
        svc: ConnService,
        events: Arc<Recorder>,
        dir: PathBuf,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn fixture() -> Fixture {
        let dir = std::env::temp_dir().join(format!("rd-svc-{}", new_id()));
        let events = Arc::new(Recorder::default());
        let svc = ConnService::new(
            ConnStore::new(dir.join("connections.json")),
            Secrets::new(Box::new(MemoryBackend::default())),
            HostKeyService::new(
                HostKeyStore::new(dir.join("known_hosts"), vec![]),
                events.clone(),
            ),
            events.clone(),
        );
        Fixture { svc, events, dir }
    }

    /// 以新建方式保存（清空 ID）。
    fn create(svc: &ConnService, mut c: Connection) -> ConnView {
        c.id.clear();
        svc.save(SaveConnReq {
            conn: c,
            ..Default::default()
        })
        .unwrap()
    }

    fn with_jump(mut c: Connection, id: &str) -> Connection {
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
            "密码保存在凭据库，读取时填回"
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
        let ConnParams::Ssh(p) = &mut c.params;
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
    fn export_import_keeps_references() {
        let f = fixture();
        let a = create(&f.svc, ssh("a", "")).conn;
        let b = create(&f.svc, with_jump(ssh("b", ""), &a.id)).conn;
        let m = create(&f.svc, with_jump(ssh("m", ""), &b.id)).conn;
        create(&f.svc, ssh("x", ""));

        // 只导出 m 时带上它的跳板机 b 和 b 的跳板机 a。
        let mut tools = ToolsConfig::default();
        tools.enhance.click_moves_cursor = true;
        let (text, count) = f.svc.export(std::slice::from_ref(&m.id), &tools).unwrap();
        assert_eq!(count, 3);
        assert!(!text.contains("pw"), "导出文件不含密码");

        let r = f.svc.import(&text).unwrap();
        assert_eq!(r.count, 3);
        assert_eq!(r.tools, Some(tools), "工具配置随文件导入");
        let list = f.svc.list().unwrap();
        assert_eq!(list.len(), 7);
        let imported = &list[4..];
        let by_name = |n: &str| {
            imported
                .iter()
                .find(|v| v.conn.name == n)
                .unwrap()
                .conn
                .clone()
        };
        let (a2, b2, m2) = (by_name("a"), by_name("b"), by_name("m"));
        assert!(
            [&a2.id, &b2.id, &m2.id]
                .iter()
                .all(|id| ![&a.id, &b.id, &m.id].contains(id)),
            "导入时重新生成 ID"
        );
        assert_eq!(b2.jump_ref(), Some(a2.id.as_str()));
        assert_eq!(m2.jump_ref(), Some(b2.id.as_str()));
        assert!(!imported.iter().any(|v| v.has_password));

        // 只有数组也能导入；引用了不存在的连接时整体失败。
        let bad = r#"[{"id":"1","name":"ok","kind":"ssh","host":"h","user":"u"},{"id":"2","name":"bad","kind":"ssh","host":"h","user":"u","jumpId":"nope"}]"#;
        let err = f.svc.import(bad).unwrap_err();
        assert_eq!(err.msg.i18n, "err.import.invalidConn");
        assert_eq!(f.svc.list().unwrap().len(), 7);
        let r = f
            .svc
            .import(r#"[{"kind":"ssh","host":"h","user":"u"}]"#)
            .unwrap();
        assert_eq!((r.count, r.tools), (1, None));
        // 旧版本导出的文件没有 tools。
        let old = r#"{"app":"rain-dial","version":1,"connections":[{"kind":"ssh","host":"h","user":"u"}]}"#;
        assert_eq!(f.svc.import(old).unwrap().tools, None);
        let bad_tools = r#"{"connections":[{"kind":"ssh","host":"h","user":"u"}],"tools":{"enhance":{"clickMovesCursor":"yes"}}}"#;
        assert_eq!(
            f.svc.import(bad_tools).unwrap_err().msg.i18n,
            "err.import.invalidFile"
        );
        assert_eq!(
            f.svc.import("{}").unwrap_err().msg.i18n,
            "err.import.invalidFile"
        );
        assert_eq!(
            f.svc.import(r#"{"connections":[]}"#).unwrap_err().msg.i18n,
            "err.import.empty"
        );
    }

    #[test]
    fn import_rejects_other_terminals() {
        let f = fixture();
        let finalshell = r#"{"id":"h1","name":"db","host":"10.0.0.1","user_name":"root","authentication_type":1,"conection_type":100}"#;
        let moba = "[Bookmarks]\nSubRep=\nbastion=#109#0%10.0.0.1%22%ops%\n";
        for text in [finalshell, moba] {
            let path = write_file(&f, "other.json", text);
            assert_eq!(
                f.svc.import_file(&path).unwrap_err().msg.i18n,
                "err.import.invalidFile"
            );
        }
        assert!(f.svc.list().unwrap().is_empty());
    }

    fn write_file(f: &Fixture, name: &str, text: &str) -> PathBuf {
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
        assert_eq!(f.svc.import_mobaxterm(&path).unwrap(), 2);
        let list = f.svc.list().unwrap();
        let bastion = list.iter().find(|v| v.conn.name == "bastion").unwrap();
        let db = list.iter().find(|v| v.conn.name == "db").unwrap();
        assert_eq!(bastion.conn.group, "生产");
        assert_eq!(db.conn.jump_ref(), Some(bastion.conn.id.as_str()));
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
        let ConnParams::Ssh(p) = &got.params;
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

        // 本应用自己的文件仍然丢掉密码。解不开的 FinalShell 密码不挡住这台主机。远程桌面不导入。
        let rain = f
            .svc
            .import(r#"[{"kind":"ssh","host":"h","user":"u","password":"secret"}]"#)
            .unwrap();
        assert_eq!(rain.count, 1);
        assert!(
            !f.svc
                .list()
                .unwrap()
                .iter()
                .any(|v| v.conn.name == "u@h" && v.has_password)
        );
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
            import(r#"{"name":"win","host":"10.0.0.3","user_name":"admin","conection_type":101}"#)
                .unwrap_err()
                .msg
                .i18n,
            "err.import.empty"
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
        let ConnParams::Ssh(p) = &resolved.params;
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
            {"id":"rdp","name":"win","host":"10.0.0.3","user_name":"admin","conection_type":101,"parent_id":"root"}
        ]"#;
        let path = write_file(&f, "hosts.json", text);
        assert_eq!(f.svc.import_finalshell(&path).unwrap(), 2);
        let list = f.svc.list().unwrap();
        assert_eq!(list[0].conn.name, "jump");
        assert_eq!(list[0].conn.group, "生产");
        assert_eq!(list[1].conn.name, "db");
        assert_eq!(list[1].conn.group, "生产/数据库");
        assert_eq!(list[1].conn.jump_ref(), Some(list[0].conn.id.as_str()));
        assert!(list[1].has_password);

        let root = f.dir.join("fs");
        let folder = root.join("conn").join("fold");
        let host_dir = folder.join("host1");
        let other_dir = root.join("conn").join("host2");
        std::fs::create_dir_all(&host_dir).unwrap();
        std::fs::create_dir_all(&other_dir).unwrap();
        let pem = "-----BEGIN OPENSSH PRIVATE KEY-----\nnot-a-real-key\n-----END OPENSSH PRIVATE KEY-----\n";
        let pem_b64 = base64::engine::general_purpose::STANDARD.encode(pem);
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
        assert!(keyed.has_key_passphrase);
        let got = f.svc.resolve(&keyed.conn.id).unwrap();
        let ConnParams::Ssh(p) = &got.params;
        assert_eq!(p.auth, AuthType::Key);
        assert_eq!(p.key_passphrase, "beac3d85988e");
        assert_eq!(std::fs::read_to_string(&p.key_path).unwrap(), pem);

        assert_eq!(f.svc.import_finalshell(&root).unwrap(), 2);
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
