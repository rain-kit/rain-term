use tauri::{AppHandle, State};

use crate::app::AppState;
use crate::error::ApiResult;
use crate::service::workspace::{Workspace, WorkspaceTab};
use crate::tray;

type S<'a> = State<'a, AppState>;

#[tauri::command]
pub async fn workspace_get(s: S<'_>) -> ApiResult<Workspace> {
    Ok(s.workspace.get())
}

#[tauri::command]
pub async fn workspace_save_tabs(
    s: S<'_>,
    tabs: Vec<WorkspaceTab>,
    active: Option<usize>,
) -> ApiResult<()> {
    s.workspace.save_tabs(tabs, active)
}

#[tauri::command]
pub async fn workspace_touch(app: AppHandle, s: S<'_>, conn_id: String) -> ApiResult<()> {
    s.workspace.touch(&conn_id)?;
    tray::refresh(&app);
    Ok(())
}
