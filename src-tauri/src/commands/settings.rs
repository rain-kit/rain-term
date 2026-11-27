use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_opener::OpenerExt;
use tracing::info;

use crate::app::AppState;
use crate::config::{self, AppConfig, TerminalConfig, ToolsConfig};
use crate::error::{self, ApiResult};
use crate::events;
use crate::i18n;
use crate::logging::{self, LogEntry};
use crate::{tray, window};

type S<'a> = State<'a, AppState>;

const LOG_TAIL_MAX: usize = 5000;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    log_level: String,
    theme: String,
    language: String,
    system_language: &'static str,
    terminal: TerminalConfig,
    tools: ToolsConfig,
    data_dir: String,
    log_dir: String,
    version: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsInput {
    log_level: String,
    theme: String,
    language: String,
    /// 不传时保持不变。
    #[serde(default)]
    terminal: Option<TerminalConfig>,
    /// 不传时保持不变。
    #[serde(default)]
    tools: Option<ToolsConfig>,
}

fn settings_view(app: &AppHandle, s: &AppState) -> SettingsView {
    let cfg = s.config.lock().unwrap().clone();
    SettingsView {
        log_level: cfg.system.log_level,
        theme: cfg.ui.theme,
        language: cfg.ui.language,
        system_language: i18n::system().tag(),
        terminal: cfg.terminal,
        tools: cfg.tools,
        data_dir: s.data_dir.display().to_string(),
        log_dir: s.log_dir().display().to_string(),
        version: app.package_info().version.to_string(),
    }
}

#[tauri::command]
pub async fn settings_get(app: AppHandle, s: S<'_>) -> ApiResult<SettingsView> {
    Ok(settings_view(&app, &s))
}

#[tauri::command]
pub async fn settings_save(
    app: AppHandle,
    s: S<'_>,
    input: SettingsInput,
) -> ApiResult<SettingsView> {
    let mut cfg: AppConfig = s.config.lock().unwrap().clone();
    cfg.system.log_level = input.log_level;
    cfg.ui.theme = input.theme;
    cfg.ui.language = input.language;
    if let Some(t) = input.terminal {
        cfg.terminal = t;
    }
    if let Some(t) = input.tools {
        cfg.tools = t;
    }
    cfg.validate().map_err(|e| error::PARAM_INVALID.msg(e))?;
    config::save(&s.data_dir, &cfg).map_err(|e| error::SETTING_SAVE_FAILED.cause(e))?;

    s.level.set(&cfg.system.log_level);
    window::apply_theme(&app, &cfg.ui.theme);
    i18n::set(&cfg.ui.language);
    tray::refresh(&app);
    info!(level = %cfg.system.log_level, theme = %cfg.ui.theme, language = %cfg.ui.language, "设置已保存");
    *s.config.lock().unwrap() = cfg;
    Ok(settings_view(&app, &s))
}

/// 保存导入文件里带的工具配置，并通知前端重新读取设置。
pub(super) fn import_tools(app: &AppHandle, s: &AppState, tools: ToolsConfig) -> ApiResult<()> {
    let mut cfg: AppConfig = s.config.lock().unwrap().clone();
    if cfg.tools == tools {
        return Ok(());
    }
    cfg.tools = tools;
    config::save(&s.data_dir, &cfg).map_err(|e| error::SETTING_SAVE_FAILED.cause(e))?;
    *s.config.lock().unwrap() = cfg;
    info!("已导入工具配置");
    let _ = app.emit(events::SETTINGS_CHANGED, ());
    Ok(())
}

#[tauri::command]
pub async fn open_data_dir(app: AppHandle, s: S<'_>) -> ApiResult<()> {
    open_dir(&app, &s.data_dir)
}

#[tauri::command]
pub async fn open_log_dir(app: AppHandle, s: S<'_>) -> ApiResult<()> {
    let dir = s.log_dir();
    let _ = std::fs::create_dir_all(&dir);
    open_dir(&app, &dir)
}

/// 最近的日志，按时间先后排列。
#[tauri::command]
pub async fn log_tail(s: S<'_>, limit: usize) -> ApiResult<Vec<LogEntry>> {
    logging::tail(&s.log_dir(), limit.min(LOG_TAIL_MAX))
        .map_err(|e| error::LOG_READ_FAILED.cause(e))
}

/// 用系统浏览器打开链接（终端里点击的网址），只接受 http / https。
#[tauri::command]
pub async fn open_url(app: AppHandle, url: String) -> ApiResult<()> {
    let lower = url.to_ascii_lowercase();
    if !lower.starts_with("http://") && !lower.starts_with("https://") {
        return Err(error::PARAM_INVALID.err());
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| error::OPEN_URL_FAILED.cause(e))
}

fn open_dir(app: &AppHandle, dir: &std::path::Path) -> ApiResult<()> {
    app.opener()
        .open_path(dir.display().to_string(), None::<&str>)
        .map_err(|e| error::OPEN_DIR_FAILED.cause(e))
}
