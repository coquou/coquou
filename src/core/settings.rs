//! Everything Coquou remembers, saved in <app config>/settings.json: app
//! settings (appearance, language, display, fullscreen, battery, updates)
//! and the pet preferences (prefs.rs). `update` saves; callers tell the
//! windows what changed (`settings-changed` here, `prefs` in prefs.rs).

use crate::sync::lock;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_autostart::ManagerExt;

fn yes() -> bool {
    true
}

/// Light or dark appearance for every window; `System` follows the OS.
#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub theme: Theme,
    /// Interface language (i18n.rs).
    #[serde(default)]
    pub language: crate::i18n::Language,
    /// Display the pet lives on (see screens.rs); None = primary.
    #[serde(default)]
    pub monitor: Option<String>,
    /// Hide while another app is fullscreen on the pet's display.
    #[serde(default = "yes")]
    pub hide_fullscreen: bool,
    /// Drop to 30 fps on battery power.
    #[serde(default = "yes")]
    pub battery_saver: bool,
    /// Check GitHub for new versions in the background (update.rs).
    #[serde(default = "yes")]
    pub auto_update: bool,
    /// The pets page offers a 隐藏 tab to find hidden pets again; off, they
    /// stay out of sight.
    #[serde(default)]
    pub show_hidden: bool,
    /// Pet preferences (prefs.rs).
    #[serde(default)]
    pub prefs: crate::prefs::Prefs,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: Theme::System,
            language: crate::i18n::Language::System,
            monitor: None,
            hide_fullscreen: true,
            battery_saver: true,
            auto_update: true,
            show_hidden: false,
            prefs: crate::prefs::Prefs::default(),
        }
    }
}

pub struct SettingsState(Mutex<Settings>);

fn path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("settings.json"))
}

pub fn init<R: Runtime>(app: &AppHandle<R>) {
    let mut settings: Settings = path(app).map(|p| crate::fsutil::read_json_or_default(&p)).unwrap_or_default();
    settings.prefs.upgrade();
    crate::i18n::apply(settings.language);
    apply_theme(app, settings.theme);
    app.manage(SettingsState(Mutex::new(settings)));
}

/// The pages' CSS follows `prefers-color-scheme`, which the webviews take
/// from the native window appearance, so setting the app's theme switches
/// the pages, the title bar and the native menus together.
fn apply_theme<R: Runtime>(app: &AppHandle<R>, theme: Theme) {
    app.set_theme(match theme {
        Theme::System => None,
        Theme::Light => Some(tauri::Theme::Light),
        Theme::Dark => Some(tauri::Theme::Dark),
    });
}

/// After a language change: menus are native, so rebuild them.
fn apply_language<R: Runtime>(app: &AppHandle<R>, language: crate::i18n::Language) {
    crate::i18n::apply(language);
    crate::tray::refresh(app);
    #[cfg(target_os = "macos")]
    match crate::windows::app_menu(app) {
        Ok(menu) => {
            if let Err(e) = app.set_menu(menu) {
                log::warn!("cannot replace the app menu: {e}");
            }
        }
        Err(e) => log::warn!("cannot build the app menu: {e}"),
    }
}

pub fn get<R: Runtime>(app: &AppHandle<R>) -> Settings {
    app.try_state::<SettingsState>().map(|s| lock(&s.0).clone()).unwrap_or_default()
}

/// Change and save (atomically, under the settings lock so concurrent
/// updates are written in order). A failed save is returned; the in-memory
/// value still applies for this session.
pub fn update<R: Runtime>(app: &AppHandle<R>, change: impl FnOnce(&mut Settings)) -> Result<(), String> {
    let state = app.try_state::<SettingsState>().ok_or("settings not initialised")?;
    let mut s = lock(&state.0);
    change(&mut s);
    let p = path(app).ok_or(tr!("找不到配置目录", "Cannot find the settings folder"))?;
    let json = serde_json::to_string_pretty(&*s).map_err(|e| e.to_string())?;
    crate::fsutil::write_atomic(&p, json.as_bytes()).map_err(|e| {
        log::error!("cannot save {}: {e}", p.display());
        tr!(format!("设置保存失败：{e}"), format!("Could not save settings: {e}"))
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Display {
    key: String,
    label: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    #[serde(flatten)]
    settings: Settings,
    /// The display the pet is on right now (may differ from `monitor` when
    /// that one is unplugged).
    current_display: Option<String>,
    displays: Vec<Display>,
    autostart: bool,
    version: String,
    /// Where Coquou installs pets and where Codex pets are read from.
    pets_dir: Option<String>,
    codex_dir: Option<String>,
    /// "macos" | "windows" | "linux", for platform wording.
    platform: &'static str,
    /// The interface language in effect: "zh" or "en" (`language` resolved).
    lang: &'static str,
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> SettingsView {
    let (displays, current_display) = match app.get_webview_window(crate::overlay::LABEL) {
        Some(w) => (
            crate::screens::list(&w).into_iter().map(|(key, label, _)| Display { key, label }).collect(),
            crate::screens::target(&w).map(|(k, _)| k),
        ),
        None => (Vec::new(), None),
    };
    SettingsView {
        settings: get(&app),
        current_display,
        displays,
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        version: app.package_info().version.to_string(),
        pets_dir: crate::pets::coquou_root(&app).map(|p| p.display().to_string()),
        codex_dir: crate::pets::codex_root(&app).map(|p| p.display().to_string()),
        platform: std::env::consts::OS,
        lang: crate::i18n::code(),
    }
}

/// `name` is one of: hideFullscreen, batterySaver, autoUpdate, showHidden, autostart (bool),
/// display (string key), theme (system | light | dark) or language
/// (system | zh | en).
#[tauri::command]
pub fn set_setting(app: AppHandle, name: String, value: serde_json::Value) -> Result<(), String> {
    match name.as_str() {
        "hideFullscreen" => {
            let on = value.as_bool().ok_or("expected a boolean")?;
            update(&app, |s| s.hide_fullscreen = on)?;
        }
        "batterySaver" => {
            let on = value.as_bool().ok_or("expected a boolean")?;
            update(&app, |s| s.battery_saver = on)?;
            crate::overlay::refresh_power(&app);
        }
        "autoUpdate" => {
            let on = value.as_bool().ok_or("expected a boolean")?;
            update(&app, |s| s.auto_update = on)?;
        }
        "showHidden" => {
            let on = value.as_bool().ok_or("expected a boolean")?;
            update(&app, |s| s.show_hidden = on)?;
        }
        "autostart" => {
            let on = value.as_bool().ok_or("expected a boolean")?;
            let auto = app.autolaunch();
            if on { auto.enable() } else { auto.disable() }.map_err(|e| e.to_string())?;
        }
        "theme" => {
            let theme: Theme = serde_json::from_value(value).map_err(|_| "expected system, light or dark")?;
            update(&app, |s| s.theme = theme)?;
            apply_theme(&app, theme);
        }
        "language" => {
            let language: crate::i18n::Language = serde_json::from_value(value).map_err(|_| "expected system, zh or en")?;
            update(&app, |s| s.language = language)?;
            apply_language(&app, language);
        }
        "display" => {
            let key = value.as_str().ok_or("expected a display key")?;
            let window = app.get_webview_window(crate::overlay::LABEL).ok_or("no overlay window")?;
            crate::screens::choose(&window, key)?;
        }
        _ => return Err(format!("unknown setting {name}")),
    }
    // Both windows: the overlay reads the interface language from it too.
    let _ = app.emit("settings-changed", ());
    Ok(())
}
