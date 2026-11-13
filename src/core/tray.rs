//! Tray menu: only quick actions; everything else lives in the app window.
//! The same handler serves the macOS app menu.

use tauri::{
    menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Runtime,
};

const ID: &str = "main";

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    TrayIconBuilder::with_id(ID)
        .icon(icon())
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Coquou")
        .menu(&menu(app)?)
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

/// Rebuild the menu (pets, the current pet or the update state changed).
pub fn refresh<R: Runtime>(app: &AppHandle<R>) {
    if let (Some(tray), Ok(menu)) = (app.tray_by_id(ID), menu(app)) {
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

fn menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let current = crate::prefs::current(app);
    let switcher = Submenu::with_id(app, "pets", "换宠物", true)?;
    for pet in crate::pets::list(app) {
        let on = pet.key == current;
        switcher.append(&CheckMenuItem::with_id(app, format!("pet:{}", pet.key), &pet.display_name, true, on, None::<&str>)?)?;
    }
    switcher.append(&CheckMenuItem::with_id(app, "pet:builtin", "团团", true, current == "builtin", None::<&str>)?)?;
    switcher.append(&PredefinedMenuItem::separator(app)?)?;
    switcher.append(&MenuItem::with_id(app, "library", "管理宠物…", true, None::<&str>)?)?;

    let mut items: Vec<Box<dyn IsMenuItem<R>>> = vec![Box::new(MenuItem::with_id(app, "open", "打开 Coquou", true, None::<&str>)?)];
    if let Some(v) = crate::update::available_version(app) {
        items.push(Box::new(MenuItem::with_id(app, "update", format!("有新版本 {v}，查看更新…"), true, None::<&str>)?));
    }
    items.push(Box::new(PredefinedMenuItem::separator(app)?));
    items.push(Box::new(switcher));
    items.push(Box::new(MenuItem::with_id(app, "reset", "叫它回来", true, None::<&str>)?));
    items.push(Box::new(MenuItem::with_id(app, "sleep", "哄它睡觉", true, None::<&str>)?));
    items.push(Box::new(PredefinedMenuItem::separator(app)?));
    items.push(Box::new(MenuItem::with_id(app, "quit", "退出 Coquou", true, None::<&str>)?));
    let refs: Vec<&dyn IsMenuItem<R>> = items.iter().map(|b| b.as_ref()).collect();
    Menu::with_items(app, &refs)
}

/// Tray and app-menu actions. Pet actions (`pet:<key>`, `reset`, `sleep`)
/// go to the overlay as `pet-command` events.
pub fn on_menu<R: Runtime>(app: &AppHandle<R>, id: &str) {
    use crate::windows::open_app;
    match id {
        "quit" => app.exit(0),
        "open" => open_app(app, None),
        "library" => open_app(app, Some("pets")),
        "update" | "menu-about" => open_app(app, Some("about")),
        "menu-settings" => open_app(app, Some("general")),
        id if id.starts_with("pet:") || id == "reset" || id == "sleep" => {
            let _ = app.emit_to(crate::overlay::LABEL, "pet-command", id);
        }
        _ => {}
    }
}
