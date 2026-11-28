use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::model::Connection;
use crate::util::write_file_atomic;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("连接不存在")]
    NotFound,
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("connections.json 格式错误: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Serialize, Deserialize)]
struct ConnFile {
    #[serde(default = "file_version")]
    version: u32,
    #[serde(default)]
    connections: Vec<Connection>,
}

fn file_version() -> u32 {
    1
}

/// 把连接列表保存在数据目录下的 connections.json，列表顺序即界面上的排序；密码类字段不写进这个文件。
pub struct ConnStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl ConnStore {
    pub fn new(path: PathBuf) -> Self {
        ConnStore {
            path,
            lock: Mutex::new(()),
        }
    }

    /// connections.json 所在的数据目录。导入的私钥写在其中的 keys 目录。
    pub fn dir(&self) -> PathBuf {
        self.path.parent().unwrap_or(Path::new(".")).to_path_buf()
    }

    fn load(&self) -> Result<Vec<Connection>, StoreError> {
        let data = match std::fs::read(&self.path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        if data.iter().all(u8::is_ascii_whitespace) {
            return Ok(Vec::new());
        }
        let mut f: ConnFile = serde_json::from_slice(&data)?;
        for c in &mut f.connections {
            c.normalize();
        }
        Ok(f.connections)
    }

    fn save(&self, list: &[Connection]) -> Result<(), StoreError> {
        let connections = list
            .iter()
            .cloned()
            .map(|mut c| {
                c.clear_secrets();
                c
            })
            .collect();
        let data = serde_json::to_vec_pretty(&ConnFile {
            version: file_version(),
            connections,
        })?;
        write_file_atomic(&self.path, &data)?;
        Ok(())
    }

    /// 全部连接（不含密码），按保存顺序。
    pub fn list(&self) -> Result<Vec<Connection>, StoreError> {
        let _g = self.lock.lock().unwrap();
        self.load()
    }

    pub fn get(&self, id: &str) -> Result<Connection, StoreError> {
        self.list()?
            .into_iter()
            .find(|c| c.id == id)
            .ok_or(StoreError::NotFound)
    }

    /// 读出列表交给 `f` 修改，`f` 成功后写回。整个过程持有锁，用于需要对照全部连接做校验的修改。
    pub fn modify<T, E: From<StoreError>>(
        &self,
        f: impl FnOnce(&mut Vec<Connection>) -> Result<T, E>,
    ) -> Result<T, E> {
        let _g = self.lock.lock().unwrap();
        let mut list = self.load()?;
        let out = f(&mut list)?;
        self.save(&list)?;
        Ok(out)
    }

    /// 按 ID 覆盖，不存在时追加到末尾。
    pub fn save_one(&self, c: Connection) -> Result<(), StoreError> {
        self.modify(|list| {
            match list.iter_mut().find(|x| x.id == c.id) {
                Some(slot) => *slot = c,
                None => list.push(c),
            }
            Ok::<_, StoreError>(())
        })
    }

    pub fn delete(&self, id: &str) -> Result<(), StoreError> {
        self.modify(|list| {
            let n = list.len();
            list.retain(|x| x.id != id);
            if list.len() == n {
                Err(StoreError::NotFound)
            } else {
                Ok(())
            }
        })
    }

    /// 按给定 ID 顺序重排，未列出的连接保持原相对顺序排在最后。
    pub fn reorder(&self, ids: &[String]) -> Result<(), StoreError> {
        self.modify(|list| {
            sort_by_ids(list, ids);
            Ok::<_, StoreError>(())
        })
    }

    /// 把连接移到另一个分组（空字符串表示未分组），同时按给定 ID 顺序重排。
    pub fn move_to_group(&self, id: &str, group: &str, ids: &[String]) -> Result<(), StoreError> {
        self.modify(|list| {
            let c = list
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or(StoreError::NotFound)?;
            c.group = group.trim().to_string();
            sort_by_ids(list, ids);
            Ok::<_, StoreError>(())
        })
    }
}

fn sort_by_ids(list: &mut [Connection], ids: &[String]) {
    let rank: HashMap<&str, usize> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    list.sort_by_key(|c| rank.get(c.id.as_str()).copied().unwrap_or(usize::MAX));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tests::ssh;

    fn temp_store() -> (ConnStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!("rd-store-{}", crate::util::new_id()));
        (ConnStore::new(dir.join("connections.json")), dir)
    }

    #[test]
    fn crud_and_reorder() {
        let (store, dir) = temp_store();
        assert!(store.list().unwrap().is_empty());

        store.save_one(ssh("a", "")).unwrap();
        store.save_one(ssh("b", "a")).unwrap();
        store.save_one(ssh("c", "a")).unwrap();
        let raw = std::fs::read_to_string(dir.join("connections.json")).unwrap();
        assert!(!raw.contains("pw"), "密码不能写入 connections.json");
        assert!(raw.contains("\"kind\": \"ssh\""));

        let mut b = store.get("b").unwrap();
        b.name = "renamed".into();
        store.save_one(b).unwrap();
        assert_eq!(store.get("b").unwrap().name, "renamed");
        assert_eq!(store.get("c").unwrap().jump_ref(), Some("a"));

        store.reorder(&["c".into(), "a".into()]).unwrap();
        let ids: Vec<_> = store.list().unwrap().into_iter().map(|c| c.id).collect();
        assert_eq!(ids, ["c", "a", "b"]);

        store.delete("a").unwrap();
        assert!(matches!(store.delete("a"), Err(StoreError::NotFound)));
        assert!(matches!(store.get("zzz"), Err(StoreError::NotFound)));
        assert_eq!(store.list().unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn modify_error_leaves_file_unchanged() {
        let (store, dir) = temp_store();
        store.save_one(ssh("a", "")).unwrap();
        let r: Result<(), StoreError> = store.modify(|list| {
            list.clear();
            Err(StoreError::NotFound)
        });
        assert!(r.is_err());
        assert_eq!(store.list().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_file_is_reported() {
        let (store, dir) = temp_store();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("connections.json"), "{ not json").unwrap();
        assert!(matches!(store.list(), Err(StoreError::Json(_))));
        let _ = std::fs::remove_dir_all(dir);
    }
}
