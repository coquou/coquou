//! Coquou — an open-source desktop pet.
//!
//! Windows: the pet overlay (`main`, overlay.rs) and the app window (`app`,
//! windows.rs). Modules:
//!   overlay   click-through overlay, cursor feed, power / fullscreen watcher
//!   tray      tray menu (and the macOS app-menu actions)
//!   windows   app window, app menu, opening folders and links
//!   prefs     pet preferences;  settings  app settings + persistence
//!   pets      pet discovery and sandboxed file reads
//!   install   .pet validation, install, export;  library  their commands
//!   ai / hatch_run   AI image generation and the saved hatch run
//!   screens / system  displays; fullscreen + battery detection per OS
//!   update    self-update;  migrate  data from the app's former names
//!   fsutil    atomic writes and safe JSON reads
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod fsutil;
mod hatch_run;
mod install;
mod library;
mod migrate;
mod overlay;
mod pets;
mod prefs;
mod screens;
mod settings;
mod system;
mod tray;
mod update;
mod windows;

use std::path::{Path, PathBuf};
use tauri::{DragDropEvent, Emitter, Manager, WindowEvent};

/// Command line: `coquou [--window] [--pet <key>] [--hatch-selftest <url> <images|chat> [transparent] [persist]] [files…]`.
/// Files are whatever is left after the flags and their values.
#[derive(Default, Debug, PartialEq)]
struct Args {
    files: Vec<PathBuf>,
    pet: Option<String>,
    window: bool,
    /// Dev: `<base url> <protocol> [transparent] [persist]`.
    selftest: Option<Vec<String>>,
}

fn parse_args(args: impl IntoIterator<Item = String>, cwd: &Path) -> Args {
    let mut out = Args::default();
    let mut it = args.into_iter().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--window" => out.window = true,
            "--pet" => out.pet = it.next(),
            // Everything after the self-test flag belongs to it.
            "--hatch-selftest" => out.selftest = Some(it.by_ref().collect()),
            flag if flag.starts_with('-') => log::warn!("unknown argument {flag}"),
            file => {
                let p = cwd.join(file);
                if p.exists() {
                    out.files.push(p);
                }
            }
        }
    }
    out
}

fn on_window_event(window: &tauri::Window, event: &WindowEvent) {
    let app = window.app_handle();
    match (window.label(), event) {
        (overlay::LABEL, WindowEvent::Moved(_) | WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }) => {
            if let Some(w) = app.get_webview_window(overlay::LABEL) {
                overlay::refresh_geometry(&w);
            }
        }
        (windows::APP, WindowEvent::Destroyed) => windows::on_app_closed(app),
        // Dropped on the app window: the page decides what the files are.
        (windows::APP, WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. })) => {
            library::set_dropped(app, paths.clone());
            let list: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
            let _ = app.emit_to(windows::APP, "app-drop", list);
        }
        // Drops only reach the overlay while the cursor is over the pet
        // (elsewhere it is click-through), so this is "feeding" it a file.
        (overlay::LABEL, WindowEvent::DragDrop(drop)) => match drop {
            DragDropEvent::Enter { .. } | DragDropEvent::Over { .. } => {
                let _ = app.emit_to(overlay::LABEL, "drop-hover", true);
            }
            DragDropEvent::Leave => {
                let _ = app.emit_to(overlay::LABEL, "drop-hover", false);
            }
            DragDropEvent::Drop { paths, .. } => {
                let _ = app.emit_to(overlay::LABEL, "drop-hover", false);
                library::install_paths_in_background(app, paths.clone());
            }
            _ => {}
        },
        _ => {}
    }
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    // No Dock icon on macOS: the pet lives in the menu-bar tray only.
    #[cfg(target_os = "macos")]
    {
        app.set_activation_policy(tauri::ActivationPolicy::Accessory);
        app.set_menu(windows::app_menu(app.handle())?)?;
    }
    // One handler for both the tray menu and the macOS app menu (a
    // tray-specific handler would fire in addition to this one).
    app.on_menu_event(|app, event| tray::on_menu(app, event.id.as_ref()));

    migrate::run(app.handle());
    install::sweep(app.handle());
    settings::init(app.handle());

    let window = app.get_webview_window(overlay::LABEL).expect("overlay window from tauri.conf.json");
    screens::ensure_placement(&window);
    window.set_ignore_cursor_events(true)?;
    let _ = window.set_visible_on_all_workspaces(true);
    window.show()?;
    overlay::refresh_geometry(&window);
    #[cfg(target_os = "macos")]
    overlay::click_without_activating(&window);
    tray::build(app.handle())?;

    let args = parse_args(std::env::args(), &std::env::current_dir().unwrap_or_default());
    // `--pet` first: a file given at the same time still wins (installed last).
    if let Some(key) = args.pet {
        library::set_pending(app.handle(), key);
    }
    library::install_paths_in_background(app.handle(), args.files);
    if let Some(rest) = args.selftest {
        // Dev: runs the hatch flow end-to-end against src/tests/mock-image-api.py.
        let cfg = serde_json::json!({
            "selftest": rest.first().cloned().unwrap_or_default(),
            "protocol": rest.get(1).cloned().unwrap_or_else(|| "images".into()),
            "transparent": rest.iter().skip(2).any(|a| a == "transparent"),
            "persist": rest.iter().skip(2).any(|a| a == "persist"),
        });
        windows::open_hatch_selftest(app.handle(), cfg);
    } else if args.window {
        windows::open_app(app.handle(), None);
    }

    overlay::start_watcher(app.handle().clone());
    overlay::start_cursor_feed(app.handle().clone());
    update::start(app.handle().clone());
    Ok(())
}

fn main() {
    let app = tauri::Builder::default()
        // First, so start-up messages are logged too: stdout and a rotating
        // file in the app log folder (~/Library/Logs/io.github.coquou,
        // %LOCALAPPDATA%\io.github.coquou\logs).
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .max_file_size(2 * 1024 * 1024)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(2))
                .build(),
        )
        // A second launch forwards its arguments here instead of starting
        // another pet: files are installed, a plain launch opens the window.
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            let args = parse_args(argv, Path::new(&cwd));
            if args.files.is_empty() {
                windows::open_app(app, None);
            }
            library::install_paths_in_background(app, args.files);
        }))
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(update::UpdateState::default())
        .manage(library::Startup::default())
        .manage(library::Dropped::default())
        .manage(overlay::PowerState::default())
        .manage(overlay::GeometryState::default())
        .manage(overlay::MouseState::default())
        .manage(prefs::Current::default())
        .invoke_handler(tauri::generate_handler![
            overlay::cursor_local,
            overlay::set_passthrough,
            overlay::log,
            overlay::power_state,
            prefs::prefs_get,
            prefs::prefs_set,
            prefs::report_current_pet,
            settings::get_settings,
            settings::set_setting,
            windows::open_window,
            windows::open_link,
            tray::show_pet_menu,
            library::overlay_ready,
            library::install_generated,
            library::install_bytes,
            library::install_dropped,
            library::read_dropped,
            library::export_pet,
            library::reveal_pet,
            library::delete_pet,
            pets::list_pets,
            pets::read_pet_file,
            screens::follow_cursor_monitor,
            hatch_run::hatch_put_image,
            hatch_run::hatch_put_meta,
            hatch_run::hatch_load,
            hatch_run::hatch_clear,
            ai::ai_profiles,
            ai::ai_save_profile,
            ai::ai_delete_profile,
            ai::ai_generate,
            update::update_status,
            update::update_check,
            update::update_install,
        ])
        .on_window_event(on_window_event)
        .setup(setup)
        .build(tauri::generate_context!())
        .expect("error while building Coquou");

    app.run(|app, event| match event {
        // macOS: clicking Coquou in Finder / Launchpad while it runs (but not
        // a click on the pet, which macOS reports the same way).
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } if !overlay::reopen_is_pet_click(app) => windows::open_app(app, None),
        // macOS delivers double-clicked files as an "open URLs" event.
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Opened { urls } => {
            let paths = urls.iter().filter_map(|u| u.to_file_path().ok()).collect();
            library::install_paths_in_background(app, paths);
        }
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Args {
        let cwd = std::env::temp_dir();
        parse_args(std::iter::once("coquou").chain(list.iter().copied()).map(String::from), &cwd)
    }

    #[test]
    fn flag_values_are_not_files() {
        let a = args(&["--pet", "preset:tuantuan", "--window"]);
        assert_eq!(a.pet.as_deref(), Some("preset:tuantuan"));
        assert!(a.window && a.files.is_empty());
        let a = args(&["--hatch-selftest", "http://127.0.0.1:8765/v1", "images", "persist"]);
        assert_eq!(a.selftest.unwrap(), ["http://127.0.0.1:8765/v1", "images", "persist"]);
        assert!(a.files.is_empty());
    }
}
