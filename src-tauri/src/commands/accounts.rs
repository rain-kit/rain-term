use tauri::State;

use crate::app::AppState;
use crate::error::ApiResult;
use crate::model::{AccountView, SaveAccountReq};

type S<'a> = State<'a, AppState>;

#[tauri::command]
pub async fn account_list(s: S<'_>) -> ApiResult<Vec<AccountView>> {
    s.accounts.views()
}

/// 正在使用它的连接用不了修改后的用户时拒绝保存。
#[tauri::command]
pub async fn account_save(s: S<'_>, req: SaveAccountReq) -> ApiResult<AccountView> {
    s.conns.save_account(req)
}

/// 还有连接在用时拒绝删除。
#[tauri::command]
pub async fn account_delete(s: S<'_>, id: String) -> ApiResult<()> {
    s.conns.delete_account(&id)
}
