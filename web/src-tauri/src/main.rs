// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    Emitter,
};

const KEYRING_SERVICE: &str = "com.baize.desktop";
const KEYRING_USER: &str = "session-token";

/// Writes a file the user chose in a save dialog.
#[tauri::command]
fn save_file(path: String, bytes: Vec<u8>) -> Result<(), String> {
    std::fs::write(&path, bytes).map_err(|err| err.to_string())
}

/// Keeps the session token in the operating system's keychain instead of the web storage.
#[tauri::command]
fn store_token(token: String) -> Result<(), String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .and_then(|entry| entry.set_password(&token))
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn read_token() -> Result<Option<String>, String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|err| err.to_string())?;
    match entry.get_password() {
        Ok(token) => Ok(Some(token)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(err.to_string()),
    }
}

#[tauri::command]
fn clear_token() -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|err| err.to_string())?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err.to_string()),
    }
}

/// The application menu. Its items send an event to the web app, which does the work.
fn build_menu(app: &tauri::App) -> tauri::Result<Menu<tauri::Wry>> {
    let new_chat = MenuItem::with_id(app, "new-chat", "New chat", true, Some("CmdOrCtrl+N"))?;
    let search = MenuItem::with_id(app, "search", "Search", true, Some("CmdOrCtrl+K"))?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+Comma"))?;
    let toggle = MenuItem::with_id(app, "toggle-sidebar", "Toggle sidebar", true, Some("CmdOrCtrl+B"))?;
    let file = Submenu::with_items(
        app,
        "File",
        true,
        &[&new_chat, &search, &PredefinedMenuItem::separator(app)?, &settings, &PredefinedMenuItem::separator(app)?, &PredefinedMenuItem::quit(app, None)?],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let view = Submenu::with_items(app, "View", true, &[&toggle])?;
    Menu::with_items(app, &[&file, &edit, &view])
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![save_file, store_token, read_token, clear_token])
        .setup(|app| {
            let menu = build_menu(app)?;
            app.set_menu(menu)?;
            Ok(())
        })
        .on_menu_event(|app, event| {
            let _ = app.emit("menu-action", event.id().0.clone());
        })
        .run(tauri::generate_context!())
        .expect("error while running the Baize desktop app");
}
