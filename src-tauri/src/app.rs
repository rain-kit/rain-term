use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tauri::{App, AppHandle, Emitter, Manager, RunEvent};
use tracing::{error, info};

use crate::config::AppConfig;
use crate::core::ssh::known_hosts::HostKeyStore;
use crate::events::{self, EventSink};
use crate::logging::LevelHandle;
use crate::secrets::{KeyringBackend, Secrets};
use crate::service::conn::ConnService;
use crate::service::hostkey::HostKeyService;
use crate::service::pool::ClientPool;
use crate::service::session::SessionService;
use crate::service::sftp::SftpService;
use crate::service::workspace::WorkspaceService;
use crate::store::ConnStore;
use crate::update::{self, Updater};
use crate::{tray, window};

pub struct AppState {
    pub data_dir: PathBuf,
    pub config: Mutex<AppConfig>,
    pub level: LevelHandle,
    pub conns: Arc<ConnService>,
    pub host_keys: Arc<HostKeyService>,
    pub pool: Arc<ClientPool>,
    pub sessions: Arc<SessionService>,
    pub sftp: Arc<SftpService>,
    pub workspace: WorkspaceService,
    pub updater: Updater,
}

impl AppState {
    pub fn log_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    /// 关闭全部终端会话、SFTP 面板和 SSH 连接。
    pub fn close_all(&self) {
        self.sessions.close_all();
        self.sftp.close_all();
        self.pool.close_all();
    }
}

struct TauriSink(AppHandle);

impl EventSink for TauriSink {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        let _ = self.0.emit(event, payload);
        // 托盘菜单里列出的最近连接要跟着改名、删除更新。
        if event == events::CONN_CHANGED {
            tray::refresh(&self.0);
        }
    }

    fn show_main_window(&self) {
        let app = self.0.clone();
        let _ = self.0.run_on_main_thread(move || window::show_main(&app));
    }
}

pub fn setup(
    app: &mut App,
    data_dir: PathBuf,
    cfg: AppConfig,
    level: LevelHandle,
) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    let sink: Arc<dyn EventSink> = Arc::new(TauriSink(handle.clone()));
    let host_keys = HostKeyService::new(
        HostKeyStore::new(
            data_dir.join("known_hosts"),
            HostKeyStore::user_known_hosts(),
        ),
        sink.clone(),
    );
    let conns = Arc::new(ConnService::new(
        ConnStore::new(data_dir.join("connections.json")),
        Secrets::new(Box::new(KeyringBackend)),
        host_keys.clone(),
        sink,
    ));
    let pool = ClientPool::new(conns.clone());
    let sessions = SessionService::new(conns.clone(), pool.clone());
    let sftp = SftpService::new(conns.clone(), pool.clone());
    let workspace = WorkspaceService::new(data_dir.join("workspace.json"));
    let updater = Updater::new(app.package_info().version.to_string());
    app.manage(AppState {
        data_dir,
        config: Mutex::new(cfg),
        level,
        conns,
        host_keys,
        pool,
        sessions,
        sftp,
        workspace,
        updater,
    });

    if let Err(e) = tray::create(&handle) {
        error!(err = %e, "创建托盘图标失败");
    }
    update::spawn(handle.clone());

    window::show_main(&handle);
    Ok(())
}

pub fn on_run_event(app: &AppHandle, ev: RunEvent) {
    match ev {
        RunEvent::Exit => {
            if let Some(s) = app.try_state::<AppState>() {
                s.close_all();
            }
            info!("程序退出");
        }
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => window::show_main(app),
        _ => {}
    }
}
