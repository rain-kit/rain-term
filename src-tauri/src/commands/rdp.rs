use tauri::State;
use tauri::ipc::{Channel, InvokeResponseBody};

use crate::app::AppState;
use crate::core::rdp::input::InputEvent;
use crate::error::ApiResult;
use crate::service::rdp::{RdpEvent, RdpSink};

type S<'a> = State<'a, AppState>;

/// 打开远程桌面，返回会话 ID。画面和鼠标指针以二进制消息（前端收到 ArrayBuffer）推到 on_event，
/// 分辨率、剪贴板和结束通知是 JSON；用户主动关闭时不推结束通知。
#[tauri::command]
pub async fn rdp_open(
    s: S<'_>,
    conn_id: String,
    width: u32,
    height: u32,
    scale: u32,
    on_event: Channel<InvokeResponseBody>,
) -> ApiResult<String> {
    let sink: RdpSink = Box::new(move |ev| {
        let body = match ev {
            RdpEvent::Data(data) => InvokeResponseBody::Raw(data),
            RdpEvent::Notice(n) => {
                InvokeResponseBody::Json(serde_json::to_string(&n).unwrap_or_default())
            }
        };
        let _ = on_event.send(body);
    });
    s.rdp.open(&conn_id, width, height, scale, sink).await
}

#[tauri::command]
pub async fn rdp_input(s: S<'_>, id: String, events: Vec<InputEvent>) -> ApiResult<()> {
    s.rdp.input(&id, events)
}

#[tauri::command]
pub async fn rdp_ctrl_alt_del(s: S<'_>, id: String) -> ApiResult<()> {
    s.rdp.ctrl_alt_del(&id)
}

/// 窗口失去焦点时松开所有按键。
#[tauri::command]
pub async fn rdp_release_all(s: S<'_>, id: String) -> ApiResult<()> {
    s.rdp.release_all(&id)
}

#[tauri::command]
pub async fn rdp_resize(
    s: S<'_>,
    id: String,
    width: u32,
    height: u32,
    scale: u32,
) -> ApiResult<()> {
    s.rdp.resize(&id, width, height, scale)
}

/// 本机剪贴板的文本，远程粘贴时使用。
#[tauri::command]
pub async fn rdp_clipboard(s: S<'_>, id: String, text: String) -> ApiResult<()> {
    s.rdp.clipboard(&id, text)
}

/// 前端画完了一条画面消息。
#[tauri::command]
pub async fn rdp_ack(s: S<'_>, id: String) -> ApiResult<()> {
    s.rdp.ack(&id);
    Ok(())
}

#[tauri::command]
pub async fn rdp_close(s: S<'_>, id: String) -> ApiResult<()> {
    s.rdp.close(&id);
    Ok(())
}
