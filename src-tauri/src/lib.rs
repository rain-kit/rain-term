#[macro_use]
pub mod i18n;
pub mod app;
pub mod commands;
pub mod config;
pub mod core;
pub mod error;
pub mod events;
pub mod logging;
pub mod model;
pub mod os_auth;
pub mod secrets;
pub mod service;
pub mod store;
pub mod tray;
pub mod update;
pub mod util;
pub mod window;

use tracing::{info, warn};

pub fn run() {
    let data_dir = config::default_data_dir();
    let _ = std::fs::create_dir_all(&data_dir);
    let (cfg, cfg_err) = match config::load(&data_dir) {
        Ok(c) => (c, None),
        Err(e) => (config::AppConfig::default(), Some(e)),
    };
    i18n::set(&cfg.ui.language);
    let level = logging::init(&data_dir.join("logs"), &cfg.system.log_level);
    if let Some(e) = cfg_err {
        warn!(err = %e, "读取配置失败，使用默认配置");
    }
    info!(version = env!("CARGO_PKG_VERSION"), data_dir = %data_dir.display(), "程序启动");

    tauri::Builder::default()
        // 必须最先注册：重复启动时把已运行实例的窗口显示出来。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            window::show_main(app)
        }))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            commands::conn::conn_list,
            commands::conn::conn_get,
            commands::conn::conn_save,
            commands::conn::conn_duplicate,
            commands::conn::conn_delete,
            commands::conn::conn_reorder,
            commands::conn::conn_test,
            commands::conn::conn_reveal_password,
            commands::conn::conn_export,
            commands::conn::conn_import,
            commands::conn::conn_import_dir,
            commands::conn::conn_import_moba,
            commands::conn::pick_key_file,
            commands::ssh::hostkey_pending,
            commands::ssh::hostkey_answer,
            commands::ssh::session_open,
            commands::ssh::session_write,
            commands::ssh::session_resize,
            commands::ssh::session_ack,
            commands::ssh::session_close,
            commands::ssh::session_close_all,
            commands::rdp::rdp_open,
            commands::rdp::rdp_input,
            commands::rdp::rdp_ctrl_alt_del,
            commands::rdp::rdp_release_all,
            commands::rdp::rdp_resize,
            commands::rdp::rdp_clipboard,
            commands::rdp::rdp_ack,
            commands::rdp::rdp_close,
            commands::sftp::sftp_open,
            commands::sftp::sftp_list,
            commands::sftp::sftp_mkdir,
            commands::sftp::sftp_rename,
            commands::sftp::sftp_remove,
            commands::sftp::sftp_upload,
            commands::sftp::sftp_download,
            commands::sftp::sftp_cancel,
            commands::sftp::sftp_close,
            commands::sftp::pick_upload_files,
            commands::sftp::pick_download_dir,
            commands::settings::settings_get,
            commands::settings::settings_save,
            commands::settings::open_data_dir,
            commands::settings::open_log_dir,
            commands::settings::log_tail,
            commands::settings::open_url,
            commands::workspace::workspace_get,
            commands::workspace::workspace_save_tabs,
            commands::workspace::workspace_touch,
            commands::update::update_status,
            commands::update::update_check,
            commands::update::update_take_open_request,
            commands::update::update_install,
            commands::update::open_release_page,
        ])
        .setup(move |app| app::setup(app, data_dir, cfg, level))
        .build(tauri::generate_context!())
        .expect("启动失败")
        .run(app::on_run_event);
}
