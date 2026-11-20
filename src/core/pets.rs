//! Pet discovery and sandboxed file access.
//!
//! A pet is a directory holding a Codex-compatible `pet.json` + spritesheet,
//! optionally extended by `coquou.json` (see doc/pet-format.md). Pets come
//! from three sources:
//!   - `coquou`: <app data>/pets            — pets installed into Coquou
//!   - `codex`:  ${CODEX_HOME:-~/.codex}/pets — read in place, never modified
//!   - `preset`: compiled into the app (presets.rs) — read-only, no directory
//!
//! The frontend addresses a pet by its key `"<source>:<dir name>"` and may only
//! read files inside that pet.

use crate::presets::{self, Preset};
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
    /// Hidden by the user: left out of automatic pet choices and of the pets
    /// page, unless the 显示隐藏宠物 setting shows its 隐藏 tab.
    pub hidden: bool,
    pub source: &'static str,
    /// The extension file the pet has (`coquou.json`, or the legacy
    /// `perchy.json`), so the frontend never has to probe for it.
    pub ext_file: Option<&'static str>,
    /// Made by Coquou's AI hatch (`tags` has "ai-hatched"), so the hatch page
    /// can load it for changes. Read here with the rest of the listing, so
    /// the page never has to open pets' files that may be gone by then.
    pub hatched: bool,
}

/// Where Coquou installs pets.
pub fn coquou_root<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("pets"))
}

/// Where Codex keeps its pets (`$CODEX_HOME/pets`, default ~/.codex/pets).
pub fn codex_root<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    roots(app).into_iter().find(|(s, _)| *s == "codex").map(|(_, p)| p)
}

/// The sources whose pets are directories on disk.
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

/// A pet, wherever its files are.
pub enum Pet {
    Dir(PathBuf),
    Preset(&'static Preset),
}

impl Pet {
    /// pet.json or an extension file, parsed.
    fn json(&self, name: &str) -> Option<serde_json::Value> {
        match self {
            Pet::Dir(dir) => read_small_json(&dir.join(name)),
            Pet::Preset(p) => serde_json::from_slice(p.file(name)?).ok(),
        }
    }

    fn has(&self, name: &str) -> bool {
        match self {
            Pet::Dir(dir) => dir.join(name).is_file(),
            Pet::Preset(p) => p.file(name).is_some(),
        }
    }
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
    let mut found = Vec::new();
    for (source, root) in roots(app) {
        let Ok(entries) = std::fs::read_dir(&root) else { continue };
        let mut dirs: Vec<_> = entries.flatten().filter(|e| e.path().is_dir()).collect();
        dirs.sort_by_key(|e| e.file_name());
        for entry in dirs {
            let name = entry.file_name().to_string_lossy().into_owned();
            if plain_name(&name) {
                found.push((source, name, Pet::Dir(entry.path())));
            }
        }
    }
    found.extend(presets::all().iter().map(|p| ("preset", p.name.to_string(), Pet::Preset(p))));

    let mut pets = Vec::new();
    for (source, name, pet) in found {
        let Some(manifest) = pet.json("pet.json") else { continue };
        let id = manifest["id"].as_str().unwrap_or(&name).to_string();
        let ext_file = crate::install::EXT_FILES.iter().copied().find(|f| pet.has(f));
        let ext = ext_file.and_then(|f| pet.json(f));
        let own_name = own_name(&manifest, ext.as_ref(), &id);
        let hatched = is_hatched(ext.as_ref());
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
            hatched,
        });
    }
    pets
}

/// The extension file marks the pet as made by the AI hatch.
fn is_hatched(ext: Option<&serde_json::Value>) -> bool {
    ext.and_then(|e| e["tags"].as_array()).is_some_and(|tags| tags.iter().any(|t| t == "ai-hatched"))
}

/// The pet identified by `key` ("<source>:<dir name>"): an embedded preset,
/// or a direct child of that source's root that is not a link.
pub fn locate<R: Runtime>(app: &AppHandle<R>, key: &str) -> Result<Pet, String> {
    let (source, name) = key.split_once(':').ok_or("bad pet key")?;
    if source == "preset" {
        return presets::get(name).map(Pet::Preset).ok_or_else(|| format!("no preset {name}"));
    }
    let root = roots(app)
        .into_iter()
        .find(|(s, _)| *s == source)
        .map(|(_, r)| r)
        .ok_or("unknown pet source")?;
    child_dir(&root, name).map(Pet::Dir).map_err(|e| format!("bad pet key: {e}"))
}

/// Directory of the pet identified by `key`; presets have none.
pub fn pet_dir<R: Runtime>(app: &AppHandle<R>, key: &str) -> Result<PathBuf, String> {
    match locate(app, key)? {
        Pet::Dir(dir) => Ok(dir),
        Pet::Preset(_) => Err(tr!("预设宠物在应用里面，没有自己的文件夹", "Preset pets live inside the app and have no folder of their own").into()),
    }
}

/// `root/name` for a plain `name`, if it is a real directory.
fn child_dir(root: &Path, name: &str) -> Result<PathBuf, String> {
    if !plain_name(name) {
        return Err("not a plain name".into());
    }
    let dir = root.join(name);
    if !no_link(&dir)?.is_dir() {
        return Err("not a directory".into());
    }
    Ok(dir)
}

/// `dir/path` where `path` is made of plain components, none of them a link.
///
/// Links are refused rather than resolved and compared by prefix: resolving
/// (`canonicalize`) also follows reparse points that are not links, such as
/// the per-package copies Windows makes of AppData files written from inside
/// a packaged (MSIX) app, and those resolve outside `dir` although nothing
/// escapes, which made every pet there fail to load.
fn inside(dir: &Path, path: &str) -> Result<PathBuf, String> {
    let rel = Path::new(path);
    if path.is_empty() || !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(format!("illegal path: {path}"));
    }
    let mut file = dir.to_path_buf();
    for part in rel.components() {
        file.push(part);
        no_link(&file).map_err(|e| format!("{path}: {e}"))?;
    }
    Ok(file)
}

/// Metadata of `p` itself, failing if it is a symlink or junction.
fn no_link(p: &Path) -> Result<std::fs::Metadata, String> {
    let meta = std::fs::symlink_metadata(p).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() {
        return Err("links are not allowed".into());
    }
    Ok(meta)
}

#[tauri::command]
pub fn list_pets(app: AppHandle) -> Vec<PetEntry> {
    list(&app)
}

/// Raw bytes of a file inside a pet (arrives in JS as an ArrayBuffer). Runs
/// off the main thread: sprite sheets are megabytes. A path in a directory
/// may not escape it (`inside`).
#[tauri::command(async)]
pub fn read_pet_file(app: AppHandle, key: String, path: String) -> Result<tauri::ipc::Response, String> {
    let bytes = match locate(&app, &key)? {
        Pet::Dir(dir) => std::fs::read(inside(&dir, &path)?).map_err(|e| format!("{path}: {e}"))?,
        Pet::Preset(p) => p.file(&path).ok_or_else(|| format!("{path}: not found"))?.to_vec(),
    };
    Ok(tauri::ipc::Response::new(bytes))
}

#[cfg(test)]
mod tests {
    use super::{child_dir, inside, is_hatched, own_name, plain_name};
    use serde_json::json;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("coquou-pets-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pet/sub")).unwrap();
        fs::write(dir.join("pet/pet.json"), "{}").unwrap();
        fs::write(dir.join("pet/sub/a.png"), "x").unwrap();
        dir
    }

    /// A directory link (a junction on Windows, which needs no privileges).
    fn link_dir(target: &Path, link: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        assert!(std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap()
            .status
            .success());
    }

    #[test]
    fn files_inside_a_pet_resolve() {
        let root = temp_root("ok");
        let pet = child_dir(&root, "pet").unwrap();
        assert_eq!(pet, root.join("pet"));
        assert_eq!(inside(&pet, "pet.json").unwrap(), pet.join("pet.json"));
        assert_eq!(inside(&pet, "sub/a.png").unwrap(), pet.join("sub").join("a.png"));
        for bad in ["", "..", "../x", "/etc/passwd", "sub/../pet.json", "C:\\x"] {
            assert!(inside(&pet, bad).is_err(), "{bad}");
        }
        assert!(inside(&pet, "missing.png").is_err());
        assert!(child_dir(&root, "missing").is_err());
        assert!(child_dir(&root, "pet/pet.json").is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn links_are_refused() {
        let root = temp_root("link");
        let outside = temp_root("outside");
        link_dir(&outside.join("pet"), &root.join("linked"));
        link_dir(&outside.join("pet"), &root.join("pet").join("out"));
        assert!(child_dir(&root, "linked").is_err());
        let pet = child_dir(&root, "pet").unwrap();
        assert!(inside(&pet, "out/pet.json").is_err());
        assert!(inside(&pet, "out").is_err());
        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn own_name_prefers_the_translation() {
        let manifest = json!({ "displayName": "团子" });
        crate::i18n::apply(crate::i18n::Language::En);
        let ext = json!({ "translations": { "en": { "displayName": "Tuanzi" } } });
        assert_eq!(own_name(&manifest, Some(&ext), "tuanzi"), "Tuanzi");
        assert_eq!(own_name(&manifest, Some(&json!({})), "tuanzi"), "团子");
        assert_eq!(own_name(&json!({}), None, "tuanzi"), "tuanzi");
        crate::i18n::apply(crate::i18n::Language::Zh);
        assert_eq!(own_name(&manifest, Some(&ext), "tuanzi"), "团子");
    }

    #[test]
    fn pet_names_are_single_plain_segments() {
        assert!(plain_name("tuanzi"));
        assert!(plain_name("橘子-2"));
        for bad in ["", ".hidden", "..", "a/b", "a\\b", "C:", "C:foo", "x:y"] {
            assert!(!plain_name(bad), "{bad}");
        }
    }

    #[test]
    fn hatched_from_tags() {
        assert!(is_hatched(Some(&json!({ "tags": ["猫", "ai-hatched"] }))));
        assert!(!is_hatched(Some(&json!({ "tags": ["猫"] }))));
        assert!(!is_hatched(Some(&json!({ "tags": "ai-hatched" }))));
        assert!(!is_hatched(None));
    }
}
