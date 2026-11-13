//! Which display the pet lives on.
//!
//! The overlay covers exactly one display's work area (macOS gives each display
//! its own Spaces, so one window can't span several). The choice is saved in
//! <app config>/settings.json (settings.rs) by display name and survives restarts; if that
//! display is unplugged the pet falls back to the primary one and returns
//! when it comes back.

use tauri::{Manager, Monitor, PhysicalPosition, PhysicalSize, Runtime, WebviewWindow};

/// Stable-ish identities: the OS display name, or "显示器 N" when unnamed;
/// identical names (two monitors of one model) get " #N" so each display
/// stays selectable.
fn keys(monitors: &[Monitor]) -> Vec<String> {
    let names: Vec<String> = monitors
        .iter()
        .enumerate()
        .map(|(i, m)| m.name().cloned().unwrap_or_else(|| format!("显示器 {}", i + 1)))
        .collect();
    names
        .iter()
        .enumerate()
        .map(|(i, n)| if names.iter().filter(|o| *o == n).count() > 1 { format!("{n} #{}", i + 1) } else { n.clone() })
        .collect()
}

fn same(a: &Monitor, b: &Monitor) -> bool {
    a.position() == b.position() && a.size() == b.size()
}

/// All displays as (key, label, monitor); the label marks the primary one.
pub fn list<R: Runtime>(window: &WebviewWindow<R>) -> Vec<(String, String, Monitor)> {
    let primary = window.primary_monitor().ok().flatten();
    let monitors = window.available_monitors().unwrap_or_default();
    let keys = keys(&monitors);
    monitors
        .into_iter()
        .zip(keys)
        .map(|(m, k)| {
            let main = primary.as_ref().is_some_and(|p| same(p, &m));
            let size = m.size();
            let label = format!("{k}（{}×{}{}）", size.width, size.height, if main { "，主显示器" } else { "" });
            (k, label, m)
        })
        .collect()
}

/// The display the pet should be on right now, and its key.
pub fn target<R: Runtime>(window: &WebviewWindow<R>) -> Option<(String, Monitor)> {
    target_among(window, &list(window))
}

fn target_among<R: Runtime>(window: &WebviewWindow<R>, monitors: &[(String, String, Monitor)]) -> Option<(String, Monitor)> {
    let wanted = crate::settings::get(window.app_handle()).monitor;
    if let Some(found) = monitors.iter().find(|(k, _, _)| Some(k) == wanted.as_ref()) {
        return Some((found.0.clone(), found.2.clone()));
    }
    let primary = window.primary_monitor().ok().flatten();
    monitors
        .iter()
        .find(|(_, _, m)| primary.as_ref().is_some_and(|p| same(p, m)))
        .or(monitors.first())
        .map(|(k, _, m)| (k.clone(), m.clone()))
}

/// Move/resize the overlay onto the target display's work area if it isn't
/// already there (handles hot-plug, resolution and Dock/taskbar changes).
/// Returns true when the window was changed.
pub fn ensure_placement<R: Runtime>(window: &WebviewWindow<R>) -> bool {
    ensure_placement_among(window, &list(window))
}

/// `ensure_placement` with an already fetched display list.
pub fn ensure_placement_among<R: Runtime>(window: &WebviewWindow<R>, monitors: &[(String, String, Monitor)]) -> bool {
    let Some((_, monitor)) = target_among(window, monitors) else { return false };
    let area = monitor.work_area();
    let pos = PhysicalPosition::new(area.position.x, area.position.y);
    let size = PhysicalSize::new(area.size.width, area.size.height);
    let here = window.outer_position().ok() == Some(pos) && window.outer_size().ok() == Some(size);
    if here {
        return false;
    }
    let _ = window.set_position(pos);
    let _ = window.set_size(size);
    true
}

pub fn choose<R: Runtime>(window: &WebviewWindow<R>, monitor_key: &str) -> Result<(), String> {
    crate::settings::update(window.app_handle(), |s| s.monitor = Some(monitor_key.to_string()))?;
    ensure_placement(window);
    Ok(())
}

/// If the cursor is on another display than the pet's, move the pet there
/// (dragging the pet across screens). Returns true when it moved.
#[tauri::command]
pub fn follow_cursor_monitor(window: WebviewWindow) -> bool {
    let Ok(cursor) = window.cursor_position() else { return false };
    let monitors = list(&window);
    let under = monitors.iter().find(|(_, _, m)| {
        let (p, s) = (m.position(), m.size());
        cursor.x >= p.x as f64
            && cursor.x < p.x as f64 + s.width as f64
            && cursor.y >= p.y as f64
            && cursor.y < p.y as f64 + s.height as f64
    });
    match (under, target(&window)) {
        (Some((k, _, _)), Some((current, _))) if *k != current => {
            match choose(&window, k) {
                Ok(()) => true,
                Err(e) => {
                    log::warn!("{e}");
                    false
                }
            }
        }
        _ => false,
    }
}
