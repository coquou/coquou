//! The pet overlay: one transparent, borderless, always-on-top, non-focusable
//! window covering one display's work area (screens.rs).
//!
//! Clicks pass through it except over opaque pet pixels. A click-through
//! window gets no mouse events, so a native thread feeds it the global
//! cursor position (`cursor` events, only when the cursor moves); the page
//! hit-tests its sprite buffer and flips pass-through with `set_passthrough`.
//!
//! A 1 Hz watcher keeps the overlay on the right display, hides it while
//! another app is fullscreen there and reports battery power (`power`).

use crate::sync::lock;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, Runtime, State, WebviewWindow};

pub const LABEL: &str = "main";

/// The overlay's position, size and scale, kept current from its
/// Moved / Resized / ScaleFactorChanged events so the cursor feed doesn't
/// have to query the window 60 times a second.
#[derive(Default, Clone, Copy)]
struct Geometry {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    scale: f64,
}

#[derive(Default)]
pub struct GeometryState(Mutex<Option<Geometry>>);

/// Re-read the overlay geometry (call when it moved, resized or changed scale).
pub fn refresh_geometry<R: Runtime>(window: &WebviewWindow<R>) {
    let g = (|| {
        let p = window.inner_position().ok()?;
        let s = window.inner_size().ok()?;
        let scale = window.scale_factor().ok()?;
        Some(Geometry { x: p.x as f64, y: p.y as f64, width: s.width as f64, height: s.height as f64, scale })
    })();
    *lock(&window.app_handle().state::<GeometryState>().0) = g;
}

/// Cursor in logical (CSS) pixels relative to the window's top-left.
fn cursor_in<R: Runtime>(window: &WebviewWindow<R>) -> Option<(f64, f64)> {
    let cursor = window.cursor_position().ok()?;
    let origin = window.inner_position().ok()?;
    let scale = window.scale_factor().ok()?;
    Some(((cursor.x - origin.x as f64) / scale, (cursor.y - origin.y as f64) / scale))
}

/// One-off cursor query (after moving the overlay to another display).
#[tauri::command]
pub fn cursor_local(window: WebviewWindow) -> Option<(f64, f64)> {
    cursor_in(&window)
}

/// Whether the overlay is catching the mouse, i.e. the cursor is over the
/// pet (or dragging it).
#[derive(Default)]
pub struct MouseState(AtomicBool);

/// `true` → clicks fall through to the windows below; `false` → we catch them.
#[tauri::command]
pub fn set_passthrough(window: WebviewWindow, state: State<MouseState>, enabled: bool) -> Result<(), String> {
    if window.label() != LABEL {
        return Err("only the pet window has pass-through".into());
    }
    window.set_ignore_cursor_events(enabled).map_err(|e| e.to_string())?;
    state.0.store(!enabled, Ordering::Relaxed);
    Ok(())
}

/// macOS 14+ also sends "reopen" when a click on one of our windows brings
/// Coquou to the front, so a click on the pet looks like clicking Coquou in
/// Finder. Such a click can only land on the pet while the overlay catches
/// the mouse; a launch from Finder / Launchpad never does.
#[cfg(target_os = "macos")]
pub fn reopen_is_pet_click<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.try_state::<MouseState>().is_some_and(|s| s.0.load(Ordering::Relaxed))
}

/// macOS: clicking the pet must not bring Coquou to the front. Activating
/// the app also raises the app window from behind other windows (the overlay
/// can't become key, so AppKit makes the app window key instead).
///
/// The window server activates an app on mouse-down, before the app sees
/// the event, unless the clicked window is marked as non-activating. Only
/// NSPanel offers that publicly (NSWindowStyleMaskNonactivatingPanel is
/// ignored on other windows), and the overlay is tao's NSWindow, so this
/// sets the flag that style sets, through AppKit's private
/// `_setPreventsActivation:` (a stopgap the maintainer approved; remove it once Tauri
/// can create the overlay as a non-activating panel). If a macOS release
/// drops the selector, clicks on the pet just activate Coquou again.
/// Must run on the main thread.
#[cfg(target_os = "macos")]
pub fn click_without_activating(window: &WebviewWindow) {
    use objc2::{msg_send, sel};
    use objc2_app_kit::NSWindow;

    let Ok(ptr) = window.ns_window() else {
        log::warn!("no native overlay window; clicks on the pet will activate Coquou");
        return;
    };
    // SAFETY: tao's NSWindow, which lives as long as the app; we are on the
    // main thread (setup).
    let w: &NSWindow = unsafe { &*(ptr as *const NSWindow) };
    let supported: bool = unsafe { msg_send![w, respondsToSelector: sel!(_setPreventsActivation:)] };
    if !supported {
        log::warn!("this macOS has no _setPreventsActivation:; clicks on the pet will activate Coquou");
        return;
    }
    // SAFETY: the selector exists (checked above) and takes one BOOL.
    let _: () = unsafe { msg_send![w, _setPreventsActivation: true] };
}

/// Webview console output, written to the app log (no devtools in release
/// builds). `level`: "info" | "warn" | "error".
#[tauri::command]
pub fn log(level: String, msg: String) {
    let level = match level.as_str() {
        "error" => log::Level::Error,
        "warn" => log::Level::Warn,
        _ => log::Level::Info,
    };
    log::log!(target: "webview", level, "{msg}");
}

/// `paused`: hidden because another app is fullscreen on our display.
/// `battery`: running on battery with the saver on (30 fps).
#[derive(Default, Clone, Serialize)]
pub struct Power {
    paused: bool,
    battery: bool,
}

#[derive(Default)]
pub struct PowerState(Mutex<Power>);

#[tauri::command]
pub fn power_state(state: State<PowerState>) -> Power {
    lock(&state.0).clone()
}

fn power<R: Runtime>(app: &AppHandle<R>) -> Power {
    lock(&app.state::<PowerState>().0).clone()
}

/// Battery mode (30 fps): the saver is on and the machine runs on battery.
fn saving_battery(settings: &crate::settings::Settings) -> bool {
    settings.battery_saver && crate::system::on_battery()
}

/// Record the battery mode; returns whether it changed.
fn set_battery(p: &mut Power, battery: bool) -> bool {
    if p.battery == battery {
        return false;
    }
    log::info!("power source: {}", if battery { "battery → 30 fps" } else { "AC" });
    p.battery = battery;
    true
}

/// Re-read the power source now (e.g. after toggling the battery saver).
pub fn refresh_power<R: Runtime>(app: &AppHandle<R>) {
    let battery = saving_battery(&crate::settings::get(app));
    let state = app.state::<PowerState>();
    let mut p = lock(&state.0);
    if set_battery(&mut p, battery) {
        let _ = app.emit_to(LABEL, "power", p.clone());
    }
}

/// How far outside the overlay the cursor still matters (eyes follow it).
const CURSOR_MARGIN: f64 = 600.0;

/// Push the cursor to the overlay whenever it moves: ~60 Hz (30 on battery),
/// nothing while hidden or while the cursor stands still.
pub fn start_cursor_feed<R: Runtime>(app: AppHandle<R>) {
    std::thread::spawn(move || {
        let mut last: Option<(f64, f64)> = None;
        loop {
            let p = power(&app);
            if p.paused {
                std::thread::sleep(Duration::from_millis(250));
                continue;
            }
            let geometry = *lock(&app.state::<GeometryState>().0);
            if let (Some(window), Some(g)) = (app.get_webview_window(LABEL), geometry) {
                let pos = window
                    .cursor_position()
                    .ok()
                    .map(|c| ((c.x - g.x) / g.scale, (c.y - g.y) / g.scale))
                    .filter(|&(x, y)| {
                        let (w, h) = (g.width / g.scale, g.height / g.scale);
                        x > -CURSOR_MARGIN && y > -CURSOR_MARGIN && x < w + CURSOR_MARGIN && y < h + CURSOR_MARGIN
                    });
                let moved = match (pos, last) {
                    (Some(a), Some(b)) => (a.0 - b.0).abs() >= 0.5 || (a.1 - b.1).abs() >= 0.5,
                    (a, b) => a.is_some() != b.is_some(),
                };
                if moved {
                    last = pos;
                    let _ = app.emit_to(LABEL, "cursor", pos);
                }
            }
            std::thread::sleep(Duration::from_millis(if p.battery { 33 } else { 16 }));
        }
    });
}

/// 1 Hz: display placement, fullscreen hiding, battery (every 30 s).
pub fn start_watcher<R: Runtime>(app: AppHandle<R>) {
    std::thread::spawn(move || {
        let mut tick: u64 = 0;
        let mut displays = String::new();
        loop {
            if let Some(window) = app.get_webview_window(LABEL) {
                let monitors = crate::screens::list(&window);
                let keys: Vec<&str> = monitors.iter().map(|(k, _, _)| k.as_str()).collect();
                let moved = crate::screens::ensure_placement_among(&window, &monitors);
                if moved || keys.join("|") != displays {
                    displays = keys.join("|");
                    // The app window lists displays.
                    let _ = app.emit_to(crate::windows::APP, "settings-changed", ());
                }

                let rect = (|| {
                    let s = window.scale_factor().ok()?;
                    let p = window.outer_position().ok()?;
                    let z = window.outer_size().ok()?;
                    Some((p.x as f64 / s, p.y as f64 / s, z.width as f64 / s, z.height as f64 / s))
                })();
                let settings = crate::settings::get(&app);
                let fullscreen = settings.hide_fullscreen && rect.is_some_and(crate::system::fullscreen_app_on);
                let battery = tick.is_multiple_of(30).then(|| saving_battery(&settings));

                let state = app.state::<PowerState>();
                let mut p = lock(&state.0);
                let mut changed = false;
                if p.paused != fullscreen {
                    p.paused = fullscreen;
                    changed = true;
                    let _ = if fullscreen { window.hide() } else { window.show() };
                    log::info!("{}", if fullscreen { "fullscreen app → hidden" } else { "shown again" });
                }
                if let Some(b) = battery {
                    changed |= set_battery(&mut p, b);
                }
                if changed {
                    let _ = app.emit_to(LABEL, "power", p.clone());
                }
            }
            tick += 1;
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}
