//! Tray menu: only quick actions; everything else lives in the app window.
//! Right-clicking the pet pops up the same menu minus「退出」, and the same handler serves
//! the macOS app menu.

use tauri::{
    menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, Runtime, WebviewWindow,
};

const ID: &str = "main";

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    TrayIconBuilder::with_id(ID)
        .icon(icon())
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Coquou")
        .menu(&menu(app, true)?)
        // Windows convention: left click opens the app, right click the menu.
        .show_menu_on_left_click(!cfg!(target_os = "windows"))
        .on_tray_icon_event(|tray, event| {
            let left_up = matches!(
                event,
                TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
            );
            if cfg!(target_os = "windows") && left_up {
                crate::windows::open_app(tray.app_handle(), None);
            }
        })
        .build(app)?;
    Ok(())
}

/// Rebuild the menu (the update state changed).
pub fn refresh<R: Runtime>(app: &AppHandle<R>) {
    if let (Some(tray), Ok(menu)) = (app.tray_by_id(ID), menu(app, true)) {
        let _ = tray.set_menu(Some(menu));
    }
}

/// Tray icon: a monochrome template on macOS (adapts to the menu bar), the
/// coloured icon elsewhere.
fn icon() -> tauri::image::Image<'static> {
    #[cfg(target_os = "macos")]
    let bytes: &[u8] = include_bytes!("../assets/icons/tray-template.png");
    #[cfg(not(target_os = "macos"))]
    let bytes: &[u8] = include_bytes!("../assets/icons/tray.png");
    tauri::image::Image::from_bytes(bytes).expect("bundled tray icon is a valid PNG")
}

/// `with_quit`: the tray offers 退出; the pet's right-click menu leaves it
/// out so a stray click on the pet can't close the app.
fn menu<R: Runtime>(app: &AppHandle<R>, with_quit: bool) -> tauri::Result<Menu<R>> {
    let mut items: Vec<Box<dyn IsMenuItem<R>>> = vec![Box::new(MenuItem::with_id(app, "open", "打开 Coquou", true, None::<&str>)?)];
    if let Some(v) = crate::update::available_version(app) {
        items.push(Box::new(MenuItem::with_id(app, "update", format!("有新版本 {v}，查看更新…"), true, None::<&str>)?));
    }
    items.push(Box::new(PredefinedMenuItem::separator(app)?));
    items.push(Box::new(MenuItem::with_id(app, "reset", "叫TA回来", true, None::<&str>)?));
    items.push(Box::new(MenuItem::with_id(app, "sleep", "哄TA睡觉", true, None::<&str>)?));
    if with_quit {
        items.push(Box::new(PredefinedMenuItem::separator(app)?));
        items.push(Box::new(MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?));
    }
    let refs: Vec<&dyn IsMenuItem<R>> = items.iter().map(|b| b.as_ref()).collect();
    Menu::with_items(app, &refs)
}

/// Right-click on the pet: the tray menu (without 退出) at the cursor. Its clicks arrive
/// through the app-wide menu handler like the tray's.
#[tauri::command]
pub fn show_pet_menu(window: WebviewWindow) -> Result<(), String> {
    if window.label() != crate::overlay::LABEL {
        return Err("only the pet window has this menu".into());
    }
    let menu = menu(window.app_handle(), false).map_err(|e| e.to_string())?;
    window.popup_menu(&menu).map_err(|e| e.to_string())
}

/// Tray, pet-menu and app-menu actions. Pet actions (`reset`, `sleep`) go to
/// the overlay as `pet-command` events.
pub fn on_menu<R: Runtime>(app: &AppHandle<R>, id: &str) {
    use crate::windows::open_app;
    match id {
        "quit" => app.exit(0),
        "open" => open_app(app, None),
        "update" | "menu-about" => open_app(app, Some("about")),
        "menu-settings" => open_app(app, Some("general")),
        "reset" | "sleep" => {
            let _ = app.emit_to(crate::overlay::LABEL, "pet-command", id);
        }
        _ => {}
    }
}
