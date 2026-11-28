use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use super::conn::choose;
use crate::app::AppState;
use crate::core::ssh::keyfile::MAX_KEY_SIZE;
use crate::error::{self, ApiResult};
use crate::i18n::pick;
use crate::model::{KeyImportReq, SshKey};

type S<'a> = State<'a, AppState>;

#[tauri::command]
pub async fn key_list(s: S<'_>) -> ApiResult<Vec<SshKey>> {
    s.keys.list()
}

#[tauri::command]
pub async fn key_import(s: S<'_>, req: KeyImportReq) -> ApiResult<SshKey> {
    s.keys.import(req)
}

#[tauri::command]
pub async fn key_rename(s: S<'_>, id: String, name: String) -> ApiResult<SshKey> {
    s.keys.rename(&id, &name)
}

/// 还有连接在用时拒绝删除。
#[tauri::command]
pub async fn key_delete(s: S<'_>, id: String) -> ApiResult<()> {
    s.conns.delete_key(&id)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyFileContent {
    /// 不含扩展名的文件名，用作默认的密钥名称。
    name: String,
    content: String,
}

/// 选择私钥文件并读出内容，导入前在界面上显示；取消时返回 None。
#[tauri::command]
pub async fn key_read_file(app: AppHandle) -> ApiResult<Option<KeyFileContent>> {
    let mut dlg = app.dialog().file().set_title(pick([
        "选择私钥文件",
        "選擇私鑰檔案",
        "Choose a private key file",
    ]));
    if let Some(home) = dirs::home_dir() {
        let ssh = home.join(".ssh");
        dlg = dlg.set_directory(if ssh.is_dir() { ssh } else { home });
    }
    let Some(path) = choose(dlg, false).await else {
        return Ok(None);
    };
    let not_key = || error::KEY_INVALID.msg(msg!("err.key.notKeyFile", "不是私钥文件"));
    let meta = std::fs::metadata(&path).map_err(|e| error::KEY_INVALID.cause(e))?;
    if meta.len() > MAX_KEY_SIZE as u64 {
        return Err(not_key());
    }
    let bytes = std::fs::read(&path).map_err(|e| error::KEY_INVALID.cause(e))?;
    let content = String::from_utf8(bytes).map_err(|_| not_key())?;
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok(Some(KeyFileContent { name, content }))
}
