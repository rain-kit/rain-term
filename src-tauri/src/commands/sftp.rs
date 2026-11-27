use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::app::AppState;
use crate::error::ApiResult;
use crate::i18n::pick;
use crate::service::sftp::{Listing, OpenOptions, Opened, Preview, SftpEvent, SftpSink, Transfer};

type S<'a> = State<'a, AppState>;

/// 打开 SFTP 面板。传输进度和断开通知推到 on_event。
#[tauri::command]
pub async fn sftp_open(
    s: S<'_>,
    conn_id: String,
    opts: Option<OpenOptions>,
    on_event: Channel<SftpEvent>,
) -> ApiResult<Opened> {
    let sink: SftpSink = Box::new(move |ev| {
        let _ = on_event.send(ev);
    });
    let global_sudo = s.config.lock().unwrap().tools.sftp.sudo;
    s.sftp
        .open(&conn_id, opts.unwrap_or_default(), global_sudo, sink)
        .await
}

#[tauri::command]
pub async fn sftp_list(s: S<'_>, id: String, path: String) -> ApiResult<Listing> {
    s.sftp.list(&id, &path).await
}

#[tauri::command]
pub async fn sftp_mkdir(s: S<'_>, id: String, path: String) -> ApiResult<()> {
    s.sftp.mkdir(&id, &path).await
}

#[tauri::command]
pub async fn sftp_rename(s: S<'_>, id: String, from: String, to: String) -> ApiResult<()> {
    s.sftp.rename(&id, &from, &to).await
}

#[tauri::command]
pub async fn sftp_remove(s: S<'_>, id: String, paths: Vec<String>) -> ApiResult<()> {
    s.sftp.remove(&id, &paths).await
}

#[tauri::command]
pub async fn sftp_preview(s: S<'_>, id: String, path: String) -> ApiResult<Preview> {
    s.sftp.preview(&id, &path).await
}

#[tauri::command]
pub async fn sftp_upload(
    s: S<'_>,
    id: String,
    local_paths: Vec<String>,
    remote_dir: String,
) -> ApiResult<Vec<Transfer>> {
    s.sftp.upload(&id, &local_paths, &remote_dir)
}

#[tauri::command]
pub async fn sftp_download(
    s: S<'_>,
    id: String,
    remote_paths: Vec<String>,
    local_dir: String,
) -> ApiResult<Vec<Transfer>> {
    s.sftp.download(&id, &remote_paths, &local_dir)
}

#[tauri::command]
pub async fn sftp_cancel(s: S<'_>, id: String, task_id: String) -> ApiResult<()> {
    s.sftp.cancel(&id, &task_id)
}

#[tauri::command]
pub async fn sftp_close(s: S<'_>, id: String) -> ApiResult<()> {
    s.sftp.close(&id);
    Ok(())
}

/// 选择要上传的文件（可多选），取消时返回空数组。
#[tauri::command]
pub async fn pick_upload_files(app: AppHandle) -> ApiResult<Vec<String>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title(pick([
            "选择要上传的文件",
            "選擇要上傳的檔案",
            "Choose files to upload",
        ]))
        .pick_files(move |p| {
            let _ = tx.send(p);
        });
    let paths = rx.await.ok().flatten().unwrap_or_default();
    Ok(paths
        .into_iter()
        .filter_map(|p| p.into_path().ok())
        .map(|p| p.display().to_string())
        .collect())
}

/// 选择下载到的目录，默认是系统的下载目录；取消时返回空字符串。
#[tauri::command]
pub async fn pick_download_dir(app: AppHandle, default_dir: Option<String>) -> ApiResult<String> {
    let mut dlg = app.dialog().file().set_title(pick([
        "选择下载到的文件夹",
        "選擇下載到的資料夾",
        "Choose download folder",
    ]));
    if let Some(dir) = default_dir
        .filter(|d| !d.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(dirs::download_dir)
    {
        dlg = dlg.set_directory(dir);
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    dlg.pick_folder(move |p| {
        let _ = tx.send(p);
    });
    Ok(rx
        .await
        .ok()
        .flatten()
        .and_then(|p| p.into_path().ok())
        .map(|p| p.display().to_string())
        .unwrap_or_default())
}
