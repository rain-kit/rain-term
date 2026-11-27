use tauri::{AppHandle, State};

use crate::app::AppState;
use crate::error::ApiResult;
use crate::update::{self, Route, UpdateInfo};

type S<'a> = State<'a, AppState>;

#[tauri::command]
pub async fn update_status(s: S<'_>) -> ApiResult<UpdateInfo> {
    Ok(s.updater.info())
}

/// via 为跳板机的连接 ID 时经它访问 GitHub，为空时直接访问。
#[tauri::command]
pub async fn update_check(app: AppHandle, via: Option<String>) -> ApiResult<UpdateInfo> {
    let route = Route::open(&app, via.as_deref()).await?;
    update::check_now(&app, &route).await
}

/// 是否有待打开的更新窗口请求（来自托盘），取出后清除。
#[tauri::command]
pub async fn update_take_open_request(s: S<'_>) -> ApiResult<bool> {
    Ok(s.updater.take_open_request())
}

/// 下载并安装，成功后程序会重启，不会正常返回。via 同 [`update_check`]。
#[tauri::command]
pub async fn update_install(app: AppHandle, via: Option<String>) -> ApiResult<()> {
    let route = Route::open(&app, via.as_deref()).await?;
    update::install(&app, &route).await
}

#[tauri::command]
pub async fn open_release_page(app: AppHandle) -> ApiResult<()> {
    update::open_release_page(&app)
}
