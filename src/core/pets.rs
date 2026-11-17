//! Pet discovery and sandboxed file access.
//!
//! A pet is a directory holding a Codex-compatible `pet.json` + spritesheet,
//! optionally extended by `coquou.json` (see doc/pet-format.md). Pets are
//! looked up in three roots:
//!   - `coquou`: <app data>/pets            — pets installed into Coquou
//!   - `codex`:  ${CODEX_HOME:-~/.codex}/pets — read in place, never modified
//!   - `preset`: <resources>/presets          — shipped with the app, read-only
//!
//! The frontend addresses a pet by its key `"<source>:<dir name>"` and may only
//! read files inside that pet's directory.

use serde::Serialize;
use std::path::{Component, Path, PathBuf};
use tauri::{AppHandle, Manager, Runtime};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PetEntry {
    pub key: String,
    pub id: String,
    /// The user's name for the pet (prefs `custom`), else its own.
    pub display_name: String,
    /// The pet's own name (`own_name`), in the interface language when it has one.
    pub own_name: String,
    /// Hidden by the user: left out of automatic pet choices and, in the app
    /// window, of the default list.
    pub hidden: bool,
    pub source: &'static str,
    /// The extension file the pet has (`coquou.json`, or the legacy
    /// `perchy.json`), so the frontend never has to probe for it.
    pub ext_file: Option<&'static str>,
}

/// Where Coquou installs pets.
pub fn coquou_root<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("pets"))
}

/// Where Codex keeps its pets (`$CODEX_HOME/pets`, default ~/.codex/pets).
pub fn codex_root<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    roots(app).into_iter().find(|(s, _)| *s == "codex").map(|(_, p)| p)
}

fn roots<R: Runtime>(app: &AppHandle<R>) -> Vec<(&'static str, PathBuf)> {
    let mut roots = Vec::new();
    if let Some(dir) = coquou_root(app) {
        roots.push(("coquou", dir));
    }
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| app.path().home_dir().ok().map(|h| h.join(".codex")));
    if let Some(home) = codex_home {
        roots.push(("codex", home.join("pets")));
    }
    if let Ok(res) = app.path().resource_dir() {
        roots.push(("preset", res.join("presets")));
    }
    roots
}

/// A single ordinary path segment: no separators, no `.`/`..`, no hidden
/// prefix, and no `:` (on Windows `C:` would replace the root when joined).
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\', ':'])
        && matches!(Path::new(name).components().collect::<Vec<_>>().as_slice(), [Component::Normal(_)])
}

/// Largest pet.json / coquou.json read while listing.
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

fn read_small_json(path: &Path) -> Option<serde_json::Value> {
    if std::fs::metadata(path).map_or(true, |m| m.len() > MAX_MANIFEST_BYTES) {
        return None;
    }
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// A pet's own name in the interface language: the extension's
/// `translations.<lang>.displayName`, else pet.json `displayName`, else its
/// id (doc/pet-format.md).
pub fn own_name(manifest: &serde_json::Value, ext: Option<&serde_json::Value>, id: &str) -> String {
    ext.and_then(|e| e["translations"][crate::i18n::code()]["displayName"].as_str())
        .or_else(|| manifest["displayName"].as_str())
        .filter(|n| !n.trim().is_empty())
        .unwrap_or(id)
        .to_string()
}

fn list<R: Runtime>(app: &AppHandle<R>) -> Vec<PetEntry> {
    let custom = crate::settings::get(app).prefs.custom;
    let mut pets = Vec::new();
    for (source, root) in roots(app) {
        let Ok(entries) = std::fs::read_dir(&root) else { continue };
        let mut dirs: Vec<_> = entries.flatten().filter(|e| e.path().is_dir()).collect();
        dirs.sort_by_key(|e| e.file_name());
        for entry in dirs {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !plain_name(&name) {
                continue;
            }
            let Some(manifest) = read_small_json(&entry.path().join("pet.json")) else { continue };
            let id = manifest["id"].as_str().unwrap_or(&name).to_string();
            let ext_file = crate::install::EXT_FILES.iter().copied().find(|f| entry.path().join(f).is_file());
            let ext = ext_file.and_then(|f| read_small_json(&entry.path().join(f)));
            let own_name = own_name(&manifest, ext.as_ref(), &id);
            let key = format!("{source}:{name}");
            let mine = custom.get(&key);
            pets.push(PetEntry {
                display_name: mine.and_then(|c| c.name.clone()).unwrap_or_else(|| own_name.clone()),
                hidden: mine.is_some_and(|c| c.hidden),
                key,
                id,
                own_name,
                source,
                ext_file,
            });
        }
    }
    pets
}

/// Canonical directory of the pet identified by `key` ("<source>:<dir name>"),
/// which must be a direct child of that source's root.
pub fn pet_dir<R: Runtime>(app: &AppHandle<R>, key: &str) -> Result<PathBuf, String> {
    let (source, name) = key.split_once(':').ok_or("bad pet key")?;
    if !plain_name(name) {
        return Err("bad pet key".into());
    }
    let root = roots(app)
        .into_iter()
        .find(|(s, _)| *s == source)
        .map(|(_, r)| r)
        .ok_or("unknown pet source")?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let dir = root.join(name).canonicalize().map_err(|e| e.to_string())?;
    if dir.parent() != Some(root.as_path()) {
        return Err("bad pet key".into());
    }
    Ok(dir)
}

/// Resolve `path` inside the pet identified by `key`, refusing anything that
/// could escape the pet's directory (absolute paths, `..`, symlinks out).
fn resolve<R: Runtime>(app: &AppHandle<R>, key: &str, path: &str) -> Result<PathBuf, String> {
    let rel = Path::new(path);
    if path.is_empty() || !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(format!("illegal path: {path}"));
    }
    let dir = pet_dir(app, key)?;
    let file = dir.join(rel).canonicalize().map_err(|e| format!("{path}: {e}"))?;
    if !file.starts_with(&dir) {
        return Err(format!("illegal path: {path}"));
    }
    Ok(file)
}

#[tauri::command]
pub fn list_pets(app: AppHandle) -> Vec<PetEntry> {
    list(&app)
}

/// Raw bytes of a file inside a pet (arrives in JS as an ArrayBuffer). Runs
/// off the main thread: sprite sheets are megabytes.
#[tauri::command(async)]
pub fn read_pet_file(app: AppHandle, key: String, path: String) -> Result<tauri::ipc::Response, String> {
    let file = resolve(&app, &key, &path)?;
    std::fs::read(file)
        .map(tauri::ipc::Response::new)
        .map_err(|e| format!("{path}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::{own_name, plain_name};
    use serde_json::json;

    #[test]
    fn own_name_prefers_the_translation() {
        let manifest = json!({ "displayName": "团团" });
        crate::i18n::apply(crate::i18n::Language::En);
        let ext = json!({ "translations": { "en": { "displayName": "Tuantuan" } } });
        assert_eq!(own_name(&manifest, Some(&ext), "tuantuan"), "Tuantuan");
        assert_eq!(own_name(&manifest, Some(&json!({})), "tuantuan"), "团团");
        assert_eq!(own_name(&json!({}), None, "tuantuan"), "tuantuan");
        crate::i18n::apply(crate::i18n::Language::Zh);
        assert_eq!(own_name(&manifest, Some(&ext), "tuantuan"), "团团");
    }

    #[test]
    fn pet_names_are_single_plain_segments() {
        assert!(plain_name("tuantuan"));
        assert!(plain_name("橘子-2"));
        for bad in ["", ".hidden", "..", "a/b", "a\\b", "C:", "C:foo", "x:y"] {
            assert!(!plain_name(bad), "{bad}");
        }
    }
}
