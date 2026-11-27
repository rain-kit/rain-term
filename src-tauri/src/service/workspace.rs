use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::error::{self, ApiResult};
use crate::util::write_file_atomic;

/// 最近打开的连接最多记录这么多个。
pub const MAX_RECENT: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TabKind {
    Term,
    Sftp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTab {
    pub conn_id: String,
    pub kind: TabKind,
}

/// workspace.json 的内容：上次打开的标签和最近打开的连接。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Workspace {
    pub tabs: Vec<WorkspaceTab>,
    /// 当前标签在 tabs 中的位置。
    pub active: Option<usize>,
    /// 最近打开的连接 ID，最近的在前。
    pub recent: Vec<String>,
}

pub struct WorkspaceService {
    path: PathBuf,
    lock: Mutex<()>,
}

impl WorkspaceService {
    pub fn new(path: PathBuf) -> Self {
        WorkspaceService {
            path,
            lock: Mutex::new(()),
        }
    }

    /// 文件不存在或内容损坏时返回空的工作区，损坏的文件会在下次保存时被覆盖。
    fn load(&self) -> Workspace {
        match std::fs::read(&self.path) {
            Ok(d) if d.iter().all(u8::is_ascii_whitespace) => Workspace::default(),
            Ok(d) => serde_json::from_slice(&d).unwrap_or_else(|e| {
                warn!(err = %e, "workspace.json 格式错误，忽略其中的内容");
                Workspace::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Workspace::default(),
            Err(e) => {
                warn!(err = %e, "读取 workspace.json 失败");
                Workspace::default()
            }
        }
    }

    fn modify(&self, f: impl FnOnce(&mut Workspace)) -> ApiResult<()> {
        let _g = self.lock.lock().unwrap();
        let mut ws = self.load();
        f(&mut ws);
        let data =
            serde_json::to_vec_pretty(&ws).map_err(|e| error::WORKSPACE_SAVE_FAILED.cause(e))?;
        write_file_atomic(&self.path, &data).map_err(|e| {
            warn!(err = %e, "保存 workspace.json 失败");
            error::WORKSPACE_SAVE_FAILED.cause(e)
        })
    }

    pub fn get(&self) -> Workspace {
        let _g = self.lock.lock().unwrap();
        self.load()
    }

    /// 保存打开的标签；active 超出范围时视为没有当前标签。
    pub fn save_tabs(&self, tabs: Vec<WorkspaceTab>, active: Option<usize>) -> ApiResult<()> {
        let active = active.filter(|&i| i < tabs.len());
        self.modify(|ws| {
            ws.tabs = tabs;
            ws.active = active;
        })
    }

    /// 记一次打开连接：移到最近列表的最前面。
    pub fn touch(&self, conn_id: &str) -> ApiResult<()> {
        if conn_id.is_empty() {
            return Err(error::PARAM_INVALID.err());
        }
        self.modify(|ws| {
            ws.recent.retain(|id| id != conn_id);
            ws.recent.insert(0, conn_id.to_string());
            ws.recent.truncate(MAX_RECENT);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> (WorkspaceService, PathBuf) {
        let dir = std::env::temp_dir().join(format!("rd-ws-{}", crate::util::new_id()));
        (WorkspaceService::new(dir.join("workspace.json")), dir)
    }

    fn tab(id: &str, kind: TabKind) -> WorkspaceTab {
        WorkspaceTab {
            conn_id: id.into(),
            kind,
        }
    }

    #[test]
    fn save_and_load_tabs() {
        let (svc, dir) = temp();
        assert_eq!(svc.get(), Workspace::default());

        svc.save_tabs(
            vec![tab("a", TabKind::Term), tab("a", TabKind::Sftp)],
            Some(1),
        )
        .unwrap();
        let raw = std::fs::read_to_string(dir.join("workspace.json")).unwrap();
        assert!(raw.contains("\"connId\": \"a\"") && raw.contains("\"kind\": \"sftp\""));
        let ws = svc.get();
        assert_eq!(ws.tabs, [tab("a", TabKind::Term), tab("a", TabKind::Sftp)]);
        assert_eq!(ws.active, Some(1));

        svc.save_tabs(vec![tab("b", TabKind::Term)], Some(5))
            .unwrap();
        assert_eq!(svc.get().active, None, "超出范围的 active 视为没有当前标签");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn touch_keeps_recent_unique_and_bounded() {
        let (svc, dir) = temp();
        for i in 0..MAX_RECENT + 5 {
            svc.touch(&format!("c{i}")).unwrap();
        }
        svc.touch("c10").unwrap();
        let recent = svc.get().recent;
        assert_eq!(recent.len(), MAX_RECENT);
        assert_eq!(recent[0], "c10");
        assert_eq!(recent[1], format!("c{}", MAX_RECENT + 4));
        assert_eq!(recent.iter().filter(|id| *id == "c10").count(), 1);
        assert!(svc.touch("").is_err());

        svc.save_tabs(vec![tab("x", TabKind::Term)], Some(0))
            .unwrap();
        assert_eq!(svc.get().recent.len(), MAX_RECENT, "保存标签不影响最近列表");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_file_is_ignored_and_overwritten() {
        let (svc, dir) = temp();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("workspace.json"), "{ not json").unwrap();
        assert_eq!(svc.get(), Workspace::default());
        svc.touch("a").unwrap();
        assert_eq!(svc.get().recent, ["a"]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
