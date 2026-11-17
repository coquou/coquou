//! The app window ("Coquou": pets, hatch, behaviour, general, about), the
//! macOS app menu, and opening folders / project links.

use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

pub const APP: &str = "app";

/// Show the app window, optionally on a page; build it if needed.
pub fn open_app<R: Runtime>(app: &AppHandle<R>, page: Option<&str>) {
    open_app_with(app, page, None);
}

/// `init`: extra script injected when the window is created (dev self-test).
fn open_app_with<R: Runtime>(app: &AppHandle<R>, page: Option<&str>, init: Option<String>) {
    if let Some(w) = app.get_webview_window(APP) {
        if let Some(page) = page {
            let _ = app.emit_to(APP, "app-page", page);
        }
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    // While it is open, show Coquou in the Dock / app switcher like a normal app.
    #[cfg(target_os = "macos")]
    show_in_dock(app);
    let mut b = WebviewWindowBuilder::new(app, APP, WebviewUrl::App("app.html".into()))
        .title("Coquou")
        .inner_size(1000.0, 680.0)
        .min_inner_size(780.0, 540.0);
    if let Some(p) = page {
        b = b.initialization_script(format!("window.__COQUOU_PAGE__ = {};", serde_json::json!(p)));
    }
    if let Some(js) = init {
        b = b.initialization_script(js);
    }
    // Unified title bar: the sidebar runs up under the traffic lights.
    #[cfg(target_os = "macos")]
    let b = b
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .traffic_light_position(tauri::LogicalPosition::new(18.0, 24.0));
    match b.build() {
        Ok(w) => {
            let _ = w.set_focus();
        }
        Err(e) => log::warn!("cannot open the app window: {e}"),
    }
}

/// Make Coquou a regular app with a Dock tile (macOS).
///
/// An unbundled build (`cargo run`) has no Info.plist icon, so Tauri sets
/// the application icon image at start-up instead. The app starts as an
/// Accessory app, and the Dock tile created when it later turns Regular
/// shows the generic executable icon unless the image is set again. The
/// bundled app gets its icon from the bundle, so this is a no-op there.
#[cfg(target_os = "macos")]
fn show_in_dock<R: Runtime>(app: &AppHandle<R>) {
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    // Queued after the policy change, which is also handled on the main thread.
    let queued = app.run_on_main_thread(|| {
        let Some(mtm) = objc2::MainThreadMarker::new() else {
            log::warn!("Dock icon refresh ran off the main thread");
            return;
        };
        let ns_app = objc2_app_kit::NSApplication::sharedApplication(mtm);
        let icon = ns_app.applicationIconImage();
        // SAFETY: called on the main thread with an image AppKit itself returned.
        unsafe { ns_app.setApplicationIconImage(icon.as_deref()) };
    });
    if let Err(e) = queued {
        log::warn!("cannot refresh the Dock icon: {e}");
    }
}

/// Back to a menu-bar-only app once the app window is gone (macOS).
pub fn on_app_closed<R: Runtime>(_app: &AppHandle<R>) {
    #[cfg(target_os = "macos")]
    let _ = _app.set_activation_policy(tauri::ActivationPolicy::Accessory);
}

/// Dev self-test (start-up only, before any app window exists): the app
/// window on the hatch page with `cfg` injected.
pub fn open_hatch_selftest<R: Runtime>(app: &AppHandle<R>, cfg: serde_json::Value) {
    open_app_with(app, Some("hatch"), Some(format!("window.__COQUOU_SELFTEST__ = {cfg};")));
}

/// Async on purpose: Tauri runs synchronous commands on the main thread, and
/// on Windows building a webview window from there deadlocks the event loop
/// (the overlay's first-run `open_window("app")` froze the app).
#[tauri::command]
pub async fn open_window(app: AppHandle, name: String) -> Result<(), String> {
    match name.as_str() {
        "app" => open_app(&app, None),
        "pets-dir" => {
            let dir = crate::pets::coquou_root(&app).ok_or("找不到宠物文件夹")?;
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            open_path(&dir);
        }
        _ => return Err(format!("unknown window {name}")),
    }
    Ok(())
}

/// Open a folder or URL with the system's default handler.
pub fn open_path(target: impl AsRef<std::ffi::OsStr>) {
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(target).spawn();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("rundll32").arg("url.dll,FileProtocolHandler").arg(target).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let r = std::process::Command::new("xdg-open").arg(target).spawn();
    if let Err(e) = r {
        log::warn!("cannot open: {e}");
    }
}

/// Reveal a file in Finder / Explorer.
pub fn reveal(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    if let Some(dir) = path.parent() {
        open_path(dir);
    }
}

const REPO: &str = "https://github.com/coquou/coquou";

/// Project links shown in the app window (fixed list; nothing else opens).
#[tauri::command]
pub fn open_link(which: String) -> Result<(), String> {
    let url = match which.as_str() {
        "home" => REPO.to_string(),
        "issues" => format!("{REPO}/issues"),
        "releases" => format!("{REPO}/releases"),
        _ => return Err(format!("unknown link {which}")),
    };
    open_path(url);
    Ok(())
}

/// Chinese app menu bar (shown while the app window makes Coquou a regular
/// app). The Edit menu keeps ⌘C / ⌘V working in text fields.
#[cfg(target_os = "macos")]
pub fn app_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<tauri::menu::Menu<R>> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
    let sep = || PredefinedMenuItem::separator(app);
    let coquou = Submenu::with_items(app, "Coquou", true, &[
        &MenuItem::with_id(app, "menu-about", "关于 Coquou", true, None::<&str>)?,
        &sep()?,
        &MenuItem::with_id(app, "menu-settings", "设置…", true, Some("CmdOrCtrl+,"))?,
        &sep()?,
        &PredefinedMenuItem::hide(app, Some("隐藏 Coquou"))?,
        &PredefinedMenuItem::hide_others(app, Some("隐藏其他"))?,
        &PredefinedMenuItem::show_all(app, Some("全部显示"))?,
        &sep()?,
        &PredefinedMenuItem::quit(app, Some("退出"))?,
    ])?;
    let edit = Submenu::with_items(app, "编辑", true, &[
        &PredefinedMenuItem::undo(app, Some("撤销"))?,
        &PredefinedMenuItem::redo(app, Some("重做"))?,
        &sep()?,
        &PredefinedMenuItem::cut(app, Some("剪切"))?,
        &PredefinedMenuItem::copy(app, Some("拷贝"))?,
        &PredefinedMenuItem::paste(app, Some("粘贴"))?,
        &PredefinedMenuItem::select_all(app, Some("全选"))?,
    ])?;
    let window = Submenu::with_items(app, "窗口", true, &[
        &PredefinedMenuItem::minimize(app, Some("最小化"))?,
        &PredefinedMenuItem::close_window(app, Some("关闭窗口"))?,
    ])?;
    Menu::with_items(app, &[&coquou, &edit, &window])
}
