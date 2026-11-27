use serde::Deserialize;
use tauri::State;
use tauri::ipc::{Channel, InvokeResponseBody};

use crate::app::AppState;
use crate::core::ssh::shell::PtySize;
use crate::error::ApiResult;
use crate::model::HostKeyPrompt;
use crate::service::session::{SessionEvent, SessionSink};

type S<'a> = State<'a, AppState>;

#[tauri::command]
pub async fn hostkey_pending(s: S<'_>) -> ApiResult<Vec<HostKeyPrompt>> {
    Ok(s.host_keys.pending())
}

#[tauri::command]
pub async fn hostkey_answer(s: S<'_>, request_id: String, accept: bool) -> ApiResult<()> {
    s.host_keys.answer(&request_id, accept)
}

/// 打开终端会话，返回会话 ID。输出以原始字节（前端收到 ArrayBuffer）推到 on_event，
/// 会话结束时最后推一条 JSON（Closed）；用户主动关闭时不推。
#[tauri::command]
pub async fn session_open(
    s: S<'_>,
    conn_id: String,
    cols: u32,
    rows: u32,
    on_event: Channel<InvokeResponseBody>,
) -> ApiResult<String> {
    let sink: SessionSink = Box::new(move |ev| {
        let body = match ev {
            SessionEvent::Output(data) => InvokeResponseBody::Raw(data),
            SessionEvent::Closed(c) => {
                InvokeResponseBody::Json(serde_json::to_string(&c).unwrap_or_default())
            }
        };
        let _ = on_event.send(body);
    });
    let report_cwd = s.config.lock().unwrap().tools.sftp.follow_terminal;
    s.sessions
        .open(&conn_id, PtySize { cols, rows }, report_cwd, sink)
        .await
}

/// 键盘输入是文本；xterm 的 onBinary（如鼠标上报）是字节数组。
#[derive(Deserialize)]
#[serde(untagged)]
pub enum SessionInput {
    Text(String),
    Bytes(Vec<u8>),
}

#[tauri::command]
pub async fn session_write(s: S<'_>, id: String, data: SessionInput) -> ApiResult<()> {
    let bytes = match &data {
        SessionInput::Text(t) => t.as_bytes(),
        SessionInput::Bytes(b) => b.as_slice(),
    };
    s.sessions.write(&id, bytes).await
}

#[tauri::command]
pub async fn session_resize(s: S<'_>, id: String, cols: u32, rows: u32) -> ApiResult<()> {
    s.sessions.resize(&id, PtySize { cols, rows }).await
}

/// 前端已处理完 bytes 字节输出，后端据此控制发送速度。
#[tauri::command]
pub async fn session_ack(s: S<'_>, id: String, bytes: usize) -> ApiResult<()> {
    s.sessions.ack(&id, bytes);
    Ok(())
}

#[tauri::command]
pub async fn session_close(s: S<'_>, id: String) -> ApiResult<()> {
    s.sessions.close(&id).await;
    Ok(())
}

/// 前端页面加载时调用：上一次页面留下的终端会话、SFTP 面板已经没有界面了。
#[tauri::command]
pub async fn session_close_all(s: S<'_>) -> ApiResult<()> {
    s.close_all();
    Ok(())
}
