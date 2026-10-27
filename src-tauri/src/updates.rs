//! "Check for Updates…" in the app menu; becomes "Restart to Update" once a new version is installed.
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::menu::{Menu, MenuItem, MenuItemKind};
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_updater::UpdaterExt;

pub const MENU_ID: &str = "check-for-updates";

struct UpdateMenu {
    item: MenuItem<Wry>,
    ready: AtomicBool,
}

/// Default menu with the update item placed under the app's name menu (macOS).
pub fn menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::default(app)?;
    let item = MenuItem::with_id(app, MENU_ID, "Check for Updates…", true, None::<&str>)?;
    if let Some(MenuItemKind::Submenu(app_menu)) = menu.items()?.into_iter().next() {
        app_menu.insert(&item, 1)?; // right after "About Still"
    }
    app.manage(UpdateMenu { item, ready: AtomicBool::new(false) });
    Ok(menu)
}

pub fn on_click(app: &AppHandle) {
    if app.state::<UpdateMenu>().ready.load(Ordering::SeqCst) {
        app.restart();
    }
    check(app.clone(), true);
}

/// Downloads and installs any newer release; `manual` reports "up to date" and errors.
pub fn check(app: AppHandle, manual: bool) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<UpdateMenu>();
        let _ = state.item.set_enabled(false);
        let _ = state.item.set_text("Checking for Updates…");
        let result = async {
            let Some(update) = app.updater()?.check().await? else { return Ok(None) };
            let _ = state.item.set_text(format!("Downloading {}…", update.version));
            update.download_and_install(|_, _| {}, || {}).await?;
            Ok::<_, tauri_plugin_updater::Error>(Some(update.version))
        }
        .await;
        let _ = state.item.set_enabled(true);
        match result {
            Ok(Some(version)) => {
                state.ready.store(true, Ordering::SeqCst);
                let _ = state.item.set_text(format!("Restart to Update ({version})"));
            }
            Ok(None) => {
                let _ = state.item.set_text("Check for Updates…");
                if manual {
                    app.dialog().message("You're running the latest version of Still.").show(|_| {});
                }
            }
            Err(e) => {
                let _ = state.item.set_text("Check for Updates…");
                if manual {
                    app.dialog().message(format!("Couldn't check for updates: {e}")).show(|_| {});
                }
            }
        }
    });
}
