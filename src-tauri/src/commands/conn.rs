use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, State, Wry};
use tauri_plugin_dialog::{DialogExt, FileDialogBuilder};

use crate::app::AppState;
use crate::error::{self, ApiResult};
use crate::i18n::pick;
use crate::model::{ConnView, SaveConnReq, TestResult};
use crate::service::conn::{ConnService, Counts};

type S<'a> = State<'a, AppState>;

#[tauri::command]
pub async fn conn_list(s: S<'_>) -> ApiResult<Vec<ConnView>> {
    s.conns.list()
}

#[tauri::command]
pub async fn conn_get(s: S<'_>, id: String) -> ApiResult<ConnView> {
    s.conns.get(&id)
}

#[tauri::command]
pub async fn conn_save(s: S<'_>, req: SaveConnReq) -> ApiResult<ConnView> {
    s.conns.save(req)
}

#[tauri::command]
pub async fn conn_duplicate(s: S<'_>, id: String) -> ApiResult<ConnView> {
    s.conns.duplicate(&id)
}

#[tauri::command]
pub async fn conn_delete(s: S<'_>, id: String) -> ApiResult<()> {
    s.conns.delete(&id)
}

#[tauri::command]
pub async fn conn_reorder(s: S<'_>, ids: Vec<String>) -> ApiResult<()> {
    s.conns.reorder(&ids)
}

/// 把连接移到另一个分组（空字符串表示未分组），同时按 ids 调整顺序。
#[tauri::command]
pub async fn conn_move(s: S<'_>, id: String, group: String, ids: Vec<String>) -> ApiResult<()> {
    s.conns.move_to_group(&id, &group, &ids)
}

#[tauri::command]
pub async fn conn_test(s: S<'_>, req: SaveConnReq) -> ApiResult<TestResult> {
    s.conns.test(req).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    path: String,
    #[serde(flatten)]
    counts: Counts,
}

/// 选择保存位置，把连接、密钥、用户、工具配置和系统设置导出到一个文件，ids 为空时导出全部。
/// 私钥和密码用 password 加密。取消时返回 None。
#[tauri::command]
pub async fn conn_export(
    app: AppHandle,
    s: S<'_>,
    ids: Vec<String>,
    password: String,
) -> ApiResult<Option<ExportResult>> {
    if password.is_empty() {
        return Err(error::PARAM_INVALID.err());
    }
    let name = format!("rain-term-{}.json", chrono::Local::now().format("%Y%m%d"));
    let dlg = app
        .dialog()
        .file()
        .set_title(pick(["导出", "匯出", "Export"]))
        .add_filter("JSON", &["json"])
        .set_file_name(name);
    let Some(path) = choose(dlg, true).await else {
        return Ok(None);
    };
    let cfg = s.config.lock().unwrap().clone();
    let conns = s.conns.clone();
    let (text, counts) = tokio::task::spawn_blocking(move || conns.export(&ids, &cfg, &password))
        .await
        .map_err(|e| error::SERVICE_ERROR.cause(e))??;
    crate::util::write_file_atomic(&path, text.as_bytes())
        .map_err(|e| error::EXPORT_FAILED.cause(e))?;
    Ok(Some(ExportResult {
        path: path.display().to_string(),
        counts,
    }))
}

/// 选择本应用导出的文件并检查格式，返回文件路径，之后再询问导出密码；取消时返回 None。
#[tauri::command]
pub async fn conn_import_pick(app: AppHandle) -> ApiResult<Option<String>> {
    let dlg = app
        .dialog()
        .file()
        .set_title(pick(["导入", "匯入", "Import"]))
        .add_filter("JSON", &["json"]);
    let Some(path) = choose(dlg, false).await else {
        return Ok(None);
    };
    ConnService::read_import_file(&path)?;
    Ok(Some(path.display().to_string()))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    #[serde(flatten)]
    counts: Counts,
    /// 文件里带有工具配置或系统设置，已经替换。
    settings: bool,
}

/// 用导出密码导入 conn_import_pick 选好的文件；文件里带有工具配置和系统设置时一并替换。
#[tauri::command]
pub async fn conn_import(app: AppHandle, s: S<'_>, path: String, password: String) -> ApiResult<ImportResult> {
    let text = ConnService::read_import_file(Path::new(&path))?;
    let conns = s.conns.clone();
    let r = tokio::task::spawn_blocking(move || conns.import(&text, &password))
        .await
        .map_err(|e| error::SERVICE_ERROR.cause(e))??;
    let settings = r.tools.is_some() || r.settings.is_some();
    super::settings::import_config(&app, &s, r.tools, r.settings)?;
    Ok(ImportResult {
        counts: r.counts,
        settings,
    })
}

/// 选择 FinalShell 的数据目录（或其中的 conn 目录）并导入。取消时返回 None。
#[tauri::command]
pub async fn conn_import_dir(app: AppHandle, s: S<'_>) -> ApiResult<Option<usize>> {
    let dlg = app.dialog().file().set_title(pick([
        "选择 FinalShell 数据目录",
        "選擇 FinalShell 資料目錄",
        "Choose the FinalShell data folder",
    ]));
    let Some(path) = choose_folder(dlg).await else {
        return Ok(None);
    };
    s.conns.import_finalshell(&path).map(Some)
}

/// 选择 MobaXterm 会话文件并导入。取消时返回 None。
#[tauri::command]
pub async fn conn_import_moba(app: AppHandle, s: S<'_>) -> ApiResult<Option<usize>> {
    let dlg = app
        .dialog()
        .file()
        .set_title(pick([
            "导入 MobaXterm",
            "匯入 MobaXterm",
            "Import MobaXterm",
        ]))
        .add_filter("MobaXterm", &["mxtsessions", "moba", "mobaconf", "ini"]);
    let Some(path) = choose(dlg, false).await else {
        return Ok(None);
    };
    s.conns.import_mobaxterm(&path).map(Some)
}

async fn choose_folder(dlg: FileDialogBuilder<Wry>) -> Option<PathBuf> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    dlg.pick_folder(move |p| {
        let _ = tx.send(p);
    });
    rx.await.ok().flatten().and_then(|p| p.into_path().ok())
}

pub(super) async fn choose(dlg: FileDialogBuilder<Wry>, save: bool) -> Option<PathBuf> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let done = move |p| {
        let _ = tx.send(p);
    };
    if save {
        dlg.save_file(done);
    } else {
        dlg.pick_file(done);
    }
    rx.await.ok().flatten().and_then(|p| p.into_path().ok())
}
