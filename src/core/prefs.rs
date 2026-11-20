//! Pet preferences: which pet, its size, the user's own name / lines /
//! hidden flag per pet, where it roams, how lively and chatty it is, and the
//! first-run flags. Saved with the app settings
//! (settings.rs) so they survive updates; the overlay applies them and the
//! app window changes them through `prefs_set`.
//!
//! Which pet is *shown* right now is runtime state (`report_current_pet`):
//! when the remembered pet is missing the overlay falls back to another one
//! without overwriting the user's choice.

use crate::install::{check_lines, Lines, LINE_KINDS};
use crate::sync::lock;
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

/// Size multipliers the UI offers (超小 / 小 / 标准 / 大 / 超大); anything
/// else is rejected, and older saved values snap to the nearest one.
pub const SIZES: [f64; 5] = [0.5, 0.75, 1.0, 1.5, 2.0];
/// "Doze off after" delays the UI offers, in seconds (0 = never); anything
/// else is rejected, and older saved values snap to the nearest one.
pub const SLEEP_CHOICES: [u32; 3] = [30, 1800, 0];
/// Longest pet name the user may give, in characters.
const MAX_NAME_CHARS: usize = 24;

/// The user's own touches to one pet. Kept here rather than in the pet's
/// files, so read-only pets (presets, Codex) can be customised too and an
/// update or reinstall never loses them.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Custom {
    /// Shown instead of the pet's own displayName.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Replaces the pet's own lines, occasion by occasion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<Lines>,
    /// Left out of the tray menu and of automatic pet choices.
    pub hidden: bool,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    /// The pet the user picked (pet key, e.g. "preset:tuanzi").
    pub pet: Option<String>,
    /// Display size multiplier, one of `SIZES`, shared by every pet.
    pub size: f64,
    /// Size per pet key, saved by versions before the size became shared;
    /// read once by `upgrade` and never written back.
    #[serde(rename = "sizes", skip_serializing)]
    pub old_sizes: BTreeMap<String, f64>,
    /// The user's name / lines / hidden flag per pet key.
    pub custom: BTreeMap<String, Custom>,
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
            size: 1.0,
            old_sizes: BTreeMap::new(),
            custom: BTreeMap::new(),
            zone: Zone::default(),
            activity: Activity::default(),
            sleep_after: 1800,
            chatter: false,
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
    size: Option<f64>,
    /// Customise one pet: `{ "pet": key, "name"?, "lines"?, "hidden"? }`.
    custom: Option<CustomPatch>,
    zone: Option<Zone>,
    activity: Option<Activity>,
    sleep_after: Option<u32>,
    chatter: Option<bool>,
    first_run_done: Option<bool>,
    welcome_dismissed: Option<bool>,
}

/// Fields left out stay as they are; an empty name or an empty lines object
/// goes back to the pet's own.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomPatch {
    pet: String,
    name: Option<String>,
    lines: Option<Lines>,
    hidden: Option<bool>,
}

impl CustomPatch {
    fn check(&self) -> Result<(), String> {
        if let Some(name) = &self.name {
            if name.trim().chars().count() > MAX_NAME_CHARS || name.chars().any(char::is_control) {
                return Err(tr!(format!("名字最多 {MAX_NAME_CHARS} 个字"), format!("Names can have at most {MAX_NAME_CHARS} characters")));
            }
        }
        if let Some(lines) = &self.lines {
            if let Some(kind) = lines.keys().find(|k| !LINE_KINDS.contains(&k.as_str())) {
                return Err(format!("unknown line kind {kind}"));
            }
            check_lines(lines)?;
        }
        Ok(())
    }

    fn apply(self, all: &mut BTreeMap<String, Custom>) {
        let c = all.entry(self.pet.clone()).or_default();
        if let Some(name) = self.name {
            let name = name.trim();
            c.name = (!name.is_empty()).then(|| name.to_string());
        }
        if let Some(lines) = self.lines {
            let lines: Lines = lines
                .into_iter()
                .map(|(k, v)| (k, v.into_iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect::<Vec<_>>()))
                .filter(|(_, v)| !v.is_empty())
                .collect();
            c.lines = (!lines.is_empty()).then_some(lines);
        }
        if let Some(h) = self.hidden {
            c.hidden = h;
        }
        if *c == Custom::default() {
            all.remove(&self.pet);
        }
    }
}

impl Patch {
    fn apply(self, p: &mut Prefs) -> Result<(), String> {
        if let Some(s) = self.size {
            if !SIZES.iter().any(|v| (v - s).abs() < 1e-6) {
                return Err(format!("unsupported size {s}"));
            }
        }
        if self.sleep_after.is_some_and(|s| !SLEEP_CHOICES.contains(&s)) {
            return Err("sleepAfter is not one of the offered delays".into());
        }
        if let Some(c) = &self.custom {
            c.check()?;
        }
        if let Some(pet) = self.pet {
            p.pet = Some(pet);
        }
        if let Some(s) = self.size {
            p.size = s;
        }
        if let Some(c) = self.custom {
            c.apply(&mut p.custom);
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

impl Prefs {
    /// Settings saved by older versions: the built-in cat became the preset
    /// 团团, which was later re-hatched as 团子 (choice and customisation
    /// follow it), the size per pet became one shared size (the chosen
    /// pet's), and the doze-off delays changed to 30 s / 30 min / never.
    pub fn upgrade(&mut self) {
        const OLD: [&str; 2] = ["builtin", "preset:tuantuan"];
        const NEW: &str = "preset:tuanzi";
        if let Some(size) = self.pet.as_deref().and_then(|k| self.old_sizes.get(k)) {
            self.size = *size;
        }
        self.old_sizes.clear();
        if self.pet.as_deref().is_some_and(|k| OLD.contains(&k)) {
            self.pet = Some(NEW.into());
        }
        for old in OLD {
            if let Some(custom) = self.custom.remove(old) {
                self.custom.entry(NEW.into()).or_insert(custom);
            }
        }
        if !SIZES.iter().any(|v| (v - self.size).abs() < 1e-6) {
            let s = self.size;
            self.size = SIZES.into_iter().min_by(|a, b| (a - s).abs().total_cmp(&(b - s).abs())).unwrap_or(1.0);
        }
        if !SLEEP_CHOICES.contains(&self.sleep_after) {
            let s = self.sleep_after;
            self.sleep_after = SLEEP_CHOICES.into_iter().filter(|&c| c > 0).min_by_key(|&c| c.abs_diff(s)).unwrap_or(0);
        }
    }
}

/// A deleted pet: drop its customisation and, if it was the chosen one, the
/// choice.
pub fn forget_pet<R: Runtime>(app: &AppHandle<R>, key: &str) {
    let result = crate::settings::update(app, |s| {
        s.prefs.custom.remove(key);
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
    app.try_state::<Current>().map(|c| lock(&c.0).clone()).unwrap_or_default()
}

/// What windows see: the saved prefs plus the pet on screen.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct View {
    #[serde(flatten)]
    prefs: Prefs,
    current: String,
}

fn view<R: Runtime>(app: &AppHandle<R>) -> View {
    View { prefs: crate::settings::get(app).prefs, current: current(app) }
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
        let mut cur = lock(&state.0);
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
        assert_eq!(p.sleep_after, 1800);
        assert_eq!(p.size, 1.0);
        assert!(!p.chatter);
    }

    #[test]
    fn patch_applies_and_validates() {
        let mut p = Prefs::default();
        patch(r#"{ "size": 1.5, "activity": "lively" }"#).unwrap().apply(&mut p).unwrap();
        assert_eq!(p.size, 1.5);
        assert_eq!(p.activity, Activity::Lively);
        assert!(patch(r#"{ "size": 1.3 }"#).unwrap().apply(&mut p).is_err());
        assert!(patch(r#"{ "size": { "pet": "x", "value": 1 } }"#).is_err());
        assert!(patch(r#"{ "sleepAfter": 99999 }"#).unwrap().apply(&mut p).is_err());
        assert!(patch(r#"{ "sleepAfter": 45 }"#).unwrap().apply(&mut p).is_err());
        patch(r#"{ "sleepAfter": 1800 }"#).unwrap().apply(&mut p).unwrap();
        assert_eq!(p.sleep_after, 1800);
        assert!(patch(r#"{ "zone": "middle" }"#).is_err());
        assert!(patch(r#"{ "bogus": 1 }"#).is_err());
    }

    #[test]
    fn custom_name_lines_hidden() {
        let mut p = Prefs::default();
        patch(r#"{ "custom": { "pet": "preset:tuanzi", "name": "  小橘 ", "lines": { "click": ["嗨", " "] } } }"#)
            .unwrap()
            .apply(&mut p)
            .unwrap();
        let c = &p.custom["preset:tuanzi"];
        assert_eq!(c.name.as_deref(), Some("小橘"));
        assert_eq!(c.lines.as_ref().unwrap()["click"], vec!["嗨".to_string()]);
        // Back to the pet's own: the entry disappears.
        patch(r#"{ "custom": { "pet": "preset:tuanzi", "name": "", "lines": {} } }"#).unwrap().apply(&mut p).unwrap();
        assert!(p.custom.is_empty());
        patch(r#"{ "custom": { "pet": "codex:x", "hidden": true } }"#).unwrap().apply(&mut p).unwrap();
        assert!(p.custom["codex:x"].hidden);
        let long = "名".repeat(MAX_NAME_CHARS + 1);
        assert!(patch(&format!(r#"{{ "custom": {{ "pet": "a", "name": "{long}" }} }}"#)).unwrap().apply(&mut p).is_err());
        assert!(patch(r#"{ "custom": { "pet": "a", "lines": { "dance": ["x"] } } }"#).unwrap().apply(&mut p).is_err());
    }

    #[test]
    fn builtin_becomes_preset() {
        let mut p: Prefs = serde_json::from_str(r#"{ "pet": "builtin", "sizes": { "builtin": 2 } }"#).unwrap();
        p.upgrade();
        assert_eq!(p.pet.as_deref(), Some("preset:tuanzi"));
        assert_eq!(p.size, 2.0);
    }

    #[test]
    fn tuantuan_becomes_tuanzi() {
        let json = r#"{ "pet": "preset:tuantuan", "custom": { "preset:tuantuan": { "name": "小橘", "hidden": true } } }"#;
        let mut p: Prefs = serde_json::from_str(json).unwrap();
        p.upgrade();
        assert_eq!(p.pet.as_deref(), Some("preset:tuanzi"));
        assert_eq!(p.custom.keys().collect::<Vec<_>>(), ["preset:tuanzi"]);
        assert_eq!(p.custom["preset:tuanzi"].name.as_deref(), Some("小橘"));
    }

    #[test]
    fn size_per_pet_becomes_the_chosen_pets() {
        // Old offered sizes 1.3 and 1.6 snap to 大 (1.5); other pets' sizes go.
        for (old, new) in [(0.75, 0.75), (1.3, 1.5), (1.6, 1.5), (2.0, 2.0)] {
            let json = format!(r#"{{ "pet": "codex:a", "sizes": {{ "codex:a": {old}, "codex:b": 0.75 }} }}"#);
            let mut p: Prefs = serde_json::from_str(&json).unwrap();
            p.upgrade();
            assert_eq!(p.size, new, "{old}");
            assert!(p.old_sizes.is_empty());
            assert!(!serde_json::to_string(&p).unwrap().contains("sizes"));
        }
        let mut p: Prefs = serde_json::from_str(r#"{ "sizes": { "codex:b": 2 } }"#).unwrap();
        p.upgrade();
        assert_eq!(p.size, 1.0);
    }

    #[test]
    fn old_sleep_delays_snap_to_offered_ones() {
        for (old, new) in [(20, 30), (45, 30), (120, 30), (300, 30), (1200, 1800), (3600, 1800), (0, 0)] {
            let mut p: Prefs = serde_json::from_str(&format!(r#"{{ "sleepAfter": {old} }}"#)).unwrap();
            p.upgrade();
            assert_eq!(p.sleep_after, new, "{old}");
        }
    }
}
