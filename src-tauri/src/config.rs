use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::i18n::{LANGUAGES, Msg};
use crate::util::write_file_atomic;

pub const APP_NAME: &str = "Rain Term";
pub const DATA_DIR_NAME: &str = "rain-dial";
/// 设置后用它作为数据目录（便携模式、测试用）。
pub const DATA_DIR_ENV: &str = "RAIN_DIAL_DATA_DIR";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SystemConfig {
    /// 日志级别 debug/info/warn/error
    pub log_level: String,
}

impl Default for SystemConfig {
    fn default() -> Self {
        SystemConfig {
            log_level: "info".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiConfig {
    /// 主题 system/light/dark
    pub theme: String,
    /// 界面语言 system/zh-CN/zh-TW/en
    pub language: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        UiConfig {
            theme: "system".into(),
            language: "system".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TerminalConfig {
    /// 终端字体，为空时使用界面的等宽字体
    pub font_family: String,
    pub font_size: u32,
    /// 回滚保留的行数
    pub scrollback: u32,
    /// 配色方案，取值见 COLOR_SCHEMES；auto 表示跟随界面明暗
    pub color_scheme: String,
    /// 光标样式 block/bar/underline
    pub cursor_style: String,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        TerminalConfig {
            font_family: String::new(),
            font_size: 14,
            scrollback: 10_000,
            color_scheme: "auto".into(),
            cursor_style: "block".into(),
        }
    }
}

/// 顶栏“工具”里的开关，按分组存放；导出连接时一起导出。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ToolsConfig {
    pub enhance: EnhanceConfig,
    pub sftp: SftpToolsConfig,
}

/// 工具窗口里的“终端”分组，默认全部关闭。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct EnhanceConfig {
    /// 在 vim 等全屏程序里单击时把光标移到点击的位置
    pub click_moves_cursor: bool,
}

/// “SFTP”分组。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SftpToolsConfig {
    /// 连接没有单独设置时，SFTP 是否用 sudo 以 root 身份打开
    pub sudo: bool,
    /// SFTP 跟随终端目录：打开终端时注入报告当前目录的命令
    pub follow_terminal: bool,
}

/// config.yaml 的内容。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AppConfig {
    pub system: SystemConfig,
    pub ui: UiConfig,
    pub terminal: TerminalConfig,
    pub tools: ToolsConfig,
}

pub const LOG_LEVELS: [&str; 4] = ["debug", "info", "warn", "error"];
pub const THEMES: [&str; 3] = ["system", "light", "dark"];
/// 与前端 src/lib/terminalThemes.ts 对应。
pub const COLOR_SCHEMES: [&str; 6] = [
    "auto",
    "oneDark",
    "dracula",
    "nord",
    "solarizedDark",
    "solarizedLight",
];
pub const CURSOR_STYLES: [&str; 3] = ["block", "bar", "underline"];
pub const FONT_SIZES: RangeInclusive<u32> = 8..=32;
pub const SCROLLBACK: RangeInclusive<u32> = 100..=100_000;

fn check_range(name: &'static str, value: u32, range: RangeInclusive<u32>) -> Result<(), Msg> {
    if range.contains(&value) {
        return Ok(());
    }
    let (min, max) = (*range.start(), *range.end());
    Err(msg!(
        "err.config.outOfRange",
        "{name} 应在 {min} 到 {max} 之间：{value}",
        name,
        min,
        max,
        value
    ))
}

impl AppConfig {
    pub fn validate(&self) -> Result<(), Msg> {
        if !LOG_LEVELS.contains(&self.system.log_level.as_str()) {
            return Err(msg!(
                "err.config.logLevelInvalid",
                "日志级别无效: {value}",
                value = self.system.log_level
            ));
        }
        if !THEMES.contains(&self.ui.theme.as_str()) {
            return Err(msg!(
                "err.config.themeInvalid",
                "主题无效: {value}",
                value = self.ui.theme
            ));
        }
        if !LANGUAGES.contains(&self.ui.language.as_str()) {
            return Err(msg!(
                "err.config.languageInvalid",
                "语言无效: {value}",
                value = self.ui.language
            ));
        }
        if !COLOR_SCHEMES.contains(&self.terminal.color_scheme.as_str()) {
            return Err(msg!(
                "err.config.colorSchemeInvalid",
                "终端配色无效: {value}",
                value = self.terminal.color_scheme
            ));
        }
        if !CURSOR_STYLES.contains(&self.terminal.cursor_style.as_str()) {
            return Err(msg!(
                "err.config.cursorStyleInvalid",
                "光标样式无效: {value}",
                value = self.terminal.cursor_style
            ));
        }
        check_range("terminal.fontSize", self.terminal.font_size, FONT_SIZES)?;
        check_range("terminal.scrollback", self.terminal.scrollback, SCROLLBACK)?;
        Ok(())
    }

    /// 把无效的值恢复为默认值，用于读取手动改坏的配置文件。
    fn sanitize(&mut self) {
        let def = AppConfig::default();
        if !LOG_LEVELS.contains(&self.system.log_level.as_str()) {
            self.system.log_level = def.system.log_level;
        }
        if !THEMES.contains(&self.ui.theme.as_str()) {
            self.ui.theme = def.ui.theme;
        }
        if !LANGUAGES.contains(&self.ui.language.as_str()) {
            self.ui.language = def.ui.language;
        }
        if !COLOR_SCHEMES.contains(&self.terminal.color_scheme.as_str()) {
            self.terminal.color_scheme = def.terminal.color_scheme;
        }
        if !CURSOR_STYLES.contains(&self.terminal.cursor_style.as_str()) {
            self.terminal.cursor_style = def.terminal.cursor_style;
        }
        let fix = |v: &mut u32, range: RangeInclusive<u32>, default: u32| {
            if !range.contains(v) {
                *v = default;
            }
        };
        fix(
            &mut self.terminal.font_size,
            FONT_SIZES,
            def.terminal.font_size,
        );
        fix(
            &mut self.terminal.scrollback,
            SCROLLBACK,
            def.terminal.scrollback,
        );
    }

    /// 环境变量可以覆盖配置：把 yaml 路径里的 . 换成 _ 并大写，例如 SYSTEM_LOGLEVEL=debug。
    fn apply_env(&mut self) {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        if let Some(v) = var("SYSTEM_LOGLEVEL") {
            self.system.log_level = v.to_ascii_lowercase();
        }
        if let Some(v) = var("UI_THEME") {
            self.ui.theme = v.to_ascii_lowercase();
        }
        if let Some(v) = var("UI_LANGUAGE") {
            self.ui.language = v;
        }
    }
}

pub fn config_file(data_dir: &Path) -> PathBuf {
    data_dir.join("config.yaml")
}

/// 读取数据目录下的 config.yaml，不存在时写入默认配置。
pub fn load(data_dir: &Path) -> Result<AppConfig, String> {
    let path = config_file(data_dir);
    let mut cfg = match std::fs::read_to_string(&path) {
        Ok(text) if text.trim().is_empty() => AppConfig::default(),
        Ok(text) => {
            serde_yaml::from_str(&text).map_err(|e| format!("解析 {} 失败: {e}", path.display()))?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let cfg = AppConfig::default();
            save(data_dir, &cfg).map_err(|e| format!("写入默认配置失败: {e}"))?;
            cfg
        }
        Err(e) => return Err(format!("读取 {} 失败: {e}", path.display())),
    };
    cfg.apply_env();
    cfg.sanitize();
    Ok(cfg)
}

pub fn save(data_dir: &Path, cfg: &AppConfig) -> std::io::Result<()> {
    let text = serde_yaml::to_string(cfg).map_err(std::io::Error::other)?;
    write_file_atomic(&config_file(data_dir), text.as_bytes())
}

/// 数据目录：环境变量 RAIN_DIAL_DATA_DIR 优先，
/// 否则为 Windows 的 %AppData%\rain-dial、macOS 的 ~/Library/Application Support/rain-dial、Linux 的 ~/.config/rain-dial。
pub fn default_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(DATA_DIR_ENV).filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(DATA_DIR_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!("rd-cfg-{}", crate::util::new_id()))
    }

    #[test]
    fn missing_fields_use_defaults() {
        let yaml =
            "system:\n    logLevel: debug\nui:\n    theme: dark\nterminal:\n    fontSize: 16\n";
        let cfg: AppConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.system.log_level, "debug");
        assert_eq!(cfg.ui.theme, "dark");
        assert_eq!(cfg.ui.language, "system");
        assert_eq!(cfg.terminal.font_size, 16);
        assert_eq!(cfg.terminal.scrollback, 10_000);
        assert!(!cfg.tools.enhance.click_moves_cursor, "增强功能默认关闭");
        assert!(!cfg.tools.sftp.sudo, "SFTP 默认不用 sudo");
        assert!(!cfg.tools.sftp.follow_terminal, "SFTP 默认不跟随终端目录");

        let cfg: AppConfig =
            serde_yaml::from_str("tools:\n    enhance:\n        clickMovesCursor: true\n")
                .unwrap();
        assert!(cfg.tools.enhance.click_moves_cursor);

        // 旧版本写入的 app.autoCheckUpdate 不影响读取。
        let cfg: AppConfig = serde_yaml::from_str("app:\n    autoCheckUpdate: false\n").unwrap();
        assert_eq!(cfg, AppConfig::default());
    }

    #[test]
    fn load_writes_default() {
        let dir = temp_dir();
        let cfg = load(&dir).unwrap();
        assert_eq!(cfg, AppConfig::default());
        assert!(config_file(&dir).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn load_resets_invalid_values() {
        let dir = temp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        // 旧版本写入的 editor 段会被忽略。
        let yaml = "ui:\n    theme: neon\nterminal:\n    fontSize: 99\n    colorScheme: neon\n    cursorStyle: bar\n    scrollback: 500\neditor:\n    maxRows: 0\n";
        std::fs::write(config_file(&dir), yaml).unwrap();
        let cfg = load(&dir).unwrap();
        assert_eq!(cfg.ui.theme, "system");
        assert_eq!(cfg.terminal.font_size, 14);
        assert_eq!(
            (
                cfg.terminal.color_scheme.as_str(),
                cfg.terminal.cursor_style.as_str()
            ),
            ("auto", "bar")
        );
        assert_eq!(cfg.terminal.scrollback, 500, "有效值保持不变");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn validate_rejects_out_of_range() {
        let mut cfg = AppConfig::default();
        assert!(cfg.validate().is_ok());
        cfg.terminal.scrollback = 0;
        let err = cfg.validate().unwrap_err();
        assert_eq!(err.i18n, "err.config.outOfRange");
        assert_eq!(err.text(), "terminal.scrollback 应在 100 到 100000 之间：0");
    }
}
