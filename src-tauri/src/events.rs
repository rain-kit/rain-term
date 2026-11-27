/// 推送给前端的事件名，前端在 src/lib/api.ts 中订阅。
/// 连接列表变化（新增、修改、删除、排序、导入），前端收到后重新拉取列表。
pub const CONN_CHANGED: &str = "conn:changed";
/// 需要用户确认主机指纹，载荷为 HostKeyPrompt。
pub const HOST_KEY: &str = "hostkey:prompt";
/// 指纹确认请求已结束（已回答、超时或连接取消），载荷为 requestId，前端据此关闭弹窗。
pub const HOST_KEY_DONE: &str = "hostkey:done";
/// 点击了托盘菜单里的最近连接，载荷为连接 ID，前端打开终端标签。
pub const TRAY_OPEN: &str = "tray:open";
/// 检查更新的结果，载荷为 UpdateInfo。
pub const UPDATE: &str = "update:status";
/// 下载更新的进度，载荷为 { downloaded, total }。
pub const UPDATE_PROGRESS: &str = "update:progress";
/// 点击了托盘菜单里的“发现新版本”，前端打开更新窗口。
pub const UPDATE_OPEN: &str = "update:open";

/// 业务层通过它通知界面，测试中可以替换成记录或空实现。
pub trait EventSink: Send + Sync {
    fn emit(&self, event: &str, payload: serde_json::Value);
    /// 显示主窗口（窗口已销毁时重新创建）。
    fn show_main_window(&self);
}

pub struct NoopSink;

impl EventSink for NoopSink {
    fn emit(&self, _event: &str, _payload: serde_json::Value) {}
    fn show_main_window(&self) {}
}
