use tauri::State;
use tauri::ipc::Channel;

use crate::app::AppState;
use crate::error::ApiResult;
use crate::service::monitor::{MonitorEvent, MonitorSink};

type S<'a> = State<'a, AppState>;

/// 在连接已建立的 SSH 连接上定时采集服务器状态，返回监控 ID；数据推到 on_event。
#[tauri::command]
pub async fn monitor_open(
    s: S<'_>,
    conn_id: String,
    on_event: Channel<MonitorEvent>,
) -> ApiResult<String> {
    let sink: MonitorSink = Box::new(move |ev| {
        let _ = on_event.send(ev);
    });
    s.monitor.open(&conn_id, sink)
}

#[tauri::command]
pub async fn monitor_close(s: S<'_>, id: String) -> ApiResult<()> {
    s.monitor.close(&id);
    Ok(())
}
