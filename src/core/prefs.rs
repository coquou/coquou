//! Pet preferences: which pet, its size, where it roams, how lively and
//! chatty it is, and the first-run flags. Saved with the app settings
//! (settings.rs) so they survive updates; the overlay applies them and the
//! app window changes them through `prefs_set`.
//!
//! Which pet is *shown* right now is runtime state (`report_current_pet`):
//! when the remembered pet is missing the overlay falls back to another one
//! without overwriting the user's choice.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Zone {
    /// Both bottom corners (keeps the middle of the screen free).
    #[default]
    Corners,
    Right,
    Left,
    /// The whole bottom edge.
    Full,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Activity {
    Quiet,
    #[default]
    Normal,
    Lively,
}

/// Size multipliers the UI offers; anything else is rejected.
pub const SIZES: [f64; 5] = [0.75, 1.0, 1.3, 1.6, 2.0];
/// Longest "doze off after" delay accepted, in seconds (0 = never).
const MAX_SLEEP_AFTER: u32 = 3600;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    /// The pet the user picked (pet key, e.g. "preset:qiqi").
    pub pet: Option<String>,
    /// Size multiplier per pet key; missing = 1.
    pub sizes: BTreeMap<String, f64>,
    pub zone: Zone,
    pub activity: Activity,
    /// Seconds without interaction before the pet may doze off; 0 = never.
    pub sleep_after: u32,
    /// Speech bubbles with the pet's lines (coquou.json `lines`).
    pub chatter: bool,
    /// The overlay already showed the first-run welcome.
    pub first_run_done: bool,
    /// The welcome banner in the app window was dismissed.
    pub welcome_dismissed: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            pet: None,
            sizes: BTreeMap::new(),
            zone: Zone::default(),
            activity: Activity::default(),
            sleep_after: 45,
            chatter: true,
            first_run_done: false,
            welcome_dismissed: false,
        }
    }
}

/// A partial update from a window. Unknown fields are an error.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Patch {
    pet: Option<String>,
    /// Size of one pet: `{ "pet": key, "value": multiplier }`.
    size: Option<SizePatch>,
    zone: Option<Zone>,
    activity: Option<Activity>,
    sleep_after: Option<u32>,
    chatter: Option<bool>,
    first_run_done: Option<bool>,
    welcome_dismissed: Option<bool>,
}

#[derive(Deserialize)]
pub struct SizePatch {
    pet: String,
    value: f64,
}

impl Patch {
    fn apply(self, p: &mut Prefs) -> Result<(), String> {
        if let Some(s) = &self.size {
            if !SIZES.iter().any(|v| (v - s.value).abs() < 1e-6) {
                return Err(format!("unsupported size {}", s.value));
            }
        }
        if self.sleep_after.is_some_and(|s| s > MAX_SLEEP_AFTER) {
            return Err("sleepAfter is too large".into());
        }
        if let Some(pet) = self.pet {
            p.pet = Some(pet);
        }
        if let Some(s) = self.size {
            if (s.value - 1.0).abs() < 1e-6 {
                p.sizes.remove(&s.pet);
            } else {
                p.sizes.insert(s.pet, s.value);
            }
        }
        if let Some(z) = self.zone {
            p.zone = z;
        }
        if let Some(a) = self.activity {
            p.activity = a;
        }
        if let Some(s) = self.sleep_after {
            p.sleep_after = s;
        }
        if let Some(c) = self.chatter {
            p.chatter = c;
        }
        if let Some(f) = self.first_run_done {
            p.first_run_done = f;
        }
        if let Some(w) = self.welcome_dismissed {
            p.welcome_dismissed = w;
        }
        Ok(())
    }
}

/// A deleted pet: drop its size and, if it was the chosen one, the choice.
pub fn forget_pet<R: Runtime>(app: &AppHandle<R>, key: &str) {
    let result = crate::settings::update(app, |s| {
        s.prefs.sizes.remove(key);
        if s.prefs.pet.as_deref() == Some(key) {
            s.prefs.pet = None;
        }
    });
    match result {
        Ok(()) => broadcast(app),
        Err(e) => log::warn!("{e}"),
    }
}

/// The pet the overlay is showing right now ("" until it reports).
#[derive(Default)]
pub struct Current(Mutex<String>);

fn current<R: Runtime>(app: &AppHandle<R>) -> String {
    app.try_state::<Current>().map(|c| c.0.lock().unwrap().clone()).unwrap_or_default()
}

/// What windows see: the saved prefs plus the pet on screen and its size.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct View {
    #[serde(flatten)]
    prefs: Prefs,
    current: String,
    size: f64,
}

fn view<R: Runtime>(app: &AppHandle<R>) -> View {
    let prefs = crate::settings::get(app).prefs;
    let current = current(app);
    let size = prefs.sizes.get(&current).copied().unwrap_or(1.0);
    View { prefs, current, size }
}

fn broadcast<R: Runtime>(app: &AppHandle<R>) {
    let _ = app.emit("prefs", view(app));
}

#[tauri::command]
pub fn prefs_get(app: AppHandle) -> View {
    view(&app)
}

#[tauri::command]
pub fn prefs_set(app: AppHandle, patch: Patch) -> Result<View, String> {
    let mut next = crate::settings::get(&app).prefs;
    patch.apply(&mut next)?;
    crate::settings::update(&app, |s| s.prefs = next)?;
    broadcast(&app);
    Ok(view(&app))
}

/// The overlay switched to (or fell back to) `key`.
#[tauri::command]
pub fn report_current_pet(app: AppHandle, state: State<Current>, key: String) {
    let changed = {
        let mut cur = state.0.lock().unwrap();
        let changed = *cur != key;
        *cur = key;
        changed
    };
    if changed {
        broadcast(&app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(json: &str) -> Result<Patch, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn defaults_fill_missing_fields() {
        let p: Prefs = serde_json::from_str(r#"{ "zone": "left" }"#).unwrap();
        assert_eq!(p.zone, Zone::Left);
        assert_eq!(p.activity, Activity::Normal);
        assert_eq!(p.sleep_after, 45);
        assert!(p.chatter);
    }

    #[test]
    fn patch_applies_and_validates() {
        let mut p = Prefs::default();
        patch(r#"{ "size": { "pet": "preset:qiqi", "value": 1.3 }, "activity": "lively" }"#).unwrap().apply(&mut p).unwrap();
        assert_eq!(p.sizes.get("preset:qiqi"), Some(&1.3));
        assert_eq!(p.activity, Activity::Lively);
        // Back to 1 drops the entry instead of storing the default.
        patch(r#"{ "size": { "pet": "preset:qiqi", "value": 1 } }"#).unwrap().apply(&mut p).unwrap();
        assert!(p.sizes.is_empty());
        assert!(patch(r#"{ "size": { "pet": "x", "value": 7 } }"#).unwrap().apply(&mut p).is_err());
        assert!(patch(r#"{ "sleepAfter": 99999 }"#).unwrap().apply(&mut p).is_err());
        assert!(patch(r#"{ "zone": "middle" }"#).is_err());
        assert!(patch(r#"{ "bogus": 1 }"#).is_err());
    }
}
