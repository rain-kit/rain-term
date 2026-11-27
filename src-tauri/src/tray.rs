use tauri::image::Image;
use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};
use tracing::error;

use crate::app::AppState;
use crate::config::APP_NAME;
use crate::events;
use crate::i18n::pick;
use crate::window;

const TRAY_ID: &str = "main";
/// 托盘菜单最多列出的最近连接数。
const TRAY_RECENT: usize = 8;
const OPEN_PREFIX: &str = "open:";

fn icon() -> Image<'static> {
    #[cfg(target_os = "macos")]
    let bytes = include_bytes!("../icons/tray-template.png");
    #[cfg(not(target_os = "macos"))]
    let bytes = include_bytes!("../icons/tray.png");
    Image::from_bytes(bytes).expect("托盘图标无效")
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon())
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip(APP_NAME)
        .menu(&build_menu(app)?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, ev| on_menu(app, ev.id().as_ref()))
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = ev
            {
                window::show_main(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// 重建菜单：切换界面语言、连接列表变化、打开连接、检查更新后调用。
pub fn refresh(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    match build_menu(app) {
        Ok(menu) => {
            let _ = tray.set_menu(Some(menu));
        }
        Err(e) => error!(err = %e, "刷新托盘菜单失败"),
    }
}

/// 最近打开且仍然存在的连接（ID、名称），最近的在前。
fn recent_conns(app: &AppHandle) -> Vec<(String, String)> {
    let Some(s) = app.try_state::<AppState>() else {
        return Vec::new();
    };
    let Ok(conns) = s.conns.list() else {
        return Vec::new();
    };
    s.workspace
        .get()
        .recent
        .into_iter()
        .filter_map(|id| {
            conns
                .iter()
                .find(|c| c.conn.id == id)
                .map(|c| (id, c.conn.name.clone()))
        })
        .take(TRAY_RECENT)
        .collect()
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let recent = recent_conns(app)
        .into_iter()
        // Windows 菜单把 & 当作快捷键标记。
        .map(|(id, name)| {
            MenuItem::with_id(
                app,
                format!("{OPEN_PREFIX}{id}"),
                name.replace('&', "&&"),
                true,
                None::<&str>,
            )
        })
        .collect::<tauri::Result<Vec<_>>>()?;
    let header = MenuItem::with_id(
        app,
        "recent",
        pick(["最近连接", "最近連線", "Recent Connections"]),
        false,
        None::<&str>,
    )?;
    let show = MenuItem::with_id(
        app,
        "show",
        pick(["显示主界面", "顯示主視窗", "Show Window"]),
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(
        app,
        "quit",
        pick(["退出", "結束", "Quit"]),
        true,
        None::<&str>,
    )?;
    let update = match app.try_state::<AppState>().map(|s| s.updater.info()) {
        Some(crate::update::UpdateInfo {
            has_update: true,
            latest: Some(v),
            ..
        }) => {
            let text = tr!(
                "发现新版本 v{v}",
                "發現新版本 v{v}",
                "Update available: v{v}"
            );
            Some(MenuItem::with_id(app, "update", text, true, None::<&str>)?)
        }
        _ => None,
    };
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;

    let mut items: Vec<&dyn IsMenuItem<Wry>> = Vec::new();
    if !recent.is_empty() {
        items.push(&header);
        items.extend(recent.iter().map(|m| m as &dyn IsMenuItem<Wry>));
        items.push(&sep1);
    }
    items.push(&show);
    if let Some(u) = &update {
        items.push(u);
    }
    items.extend([&sep2 as &dyn IsMenuItem<Wry>, &quit]);
    Menu::with_items(app, &items)
}

fn on_menu(app: &AppHandle, id: &str) {
    if let Some(conn_id) = id.strip_prefix(OPEN_PREFIX) {
        window::show_main(app);
        let _ = app.emit(events::TRAY_OPEN, conn_id);
        return;
    }
    match id {
        "show" => window::show_main(app),
        "update" => crate::update::request_open(app),
        "quit" => app.exit(0),
        _ => {}
    }
}
