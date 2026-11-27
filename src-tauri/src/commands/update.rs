use tauri::{AppHandle, State};

use crate::app::AppState;
use crate::error::ApiResult;
use crate::update::{self, UpdateInfo};

type S<'a> = State<'a, AppState>;

#[tauri::command]
pub async fn update_status(s: S<'_>) -> ApiResult<UpdateInfo> {
    Ok(s.updater.info())
}

#[tauri::command]
pub async fn update_check(app: AppHandle) -> ApiResult<UpdateInfo> {
    update::check_now(&app).await
}

/// 是否有待打开的更新窗口请求（来自托盘），取出后清除。
#[tauri::command]
pub async fn update_take_open_request(s: S<'_>) -> ApiResult<bool> {
    Ok(s.updater.take_open_request())
}

/// 下载并安装，成功后程序会重启，不会正常返回。
#[tauri::command]
pub async fn update_install(app: AppHandle) -> ApiResult<()> {
    update::install(&app).await
}

#[tauri::command]
pub async fn open_release_page(app: AppHandle) -> ApiResult<()> {
    update::open_release_page(&app)
}
