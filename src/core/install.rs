//! Installing pets into Coquou's own pets directory from a `.pet` / `.zip`
//! archive or a plain directory, copying a pet into it (`duplicate`), and
//! exporting a pet as a `.pet`. An embedded preset is written out to a
//! staging dir first, so all of this works on directories.
//!
//! Untrusted input, so every step is defensive:
//!   - archive entries must stay inside the target (`enclosed_name`), symlinks
//!     are skipped, and file count / per-file / total sizes are capped on the
//!     bytes actually decompressed (headers can lie);
//!   - files referenced by the manifests must be plain relative paths inside
//!     the pet, and only their headers are read;
//!   - the pet is unpacked into a hidden staging dir, validated, and only then
//!     swapped into place; an older install of the same id is restored if the
//!     swap fails. Leftovers from a crash are swept at start-up.

use crate::pets::{self, Pet};
use serde::Serialize;
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Runtime};

const MAX_FILES: usize = 256;
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 96 * 1024 * 1024;
const CODEX_CELL: (u32, u32) = (192, 208);

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub key: String,
    pub display_name: String,
    pub replaced: bool,
    /// coquou.json `states` no Coquou behaviour plays (see STATE_NAMES).
    pub unused_states: Vec<String>,
}

/// Largest JSON file read from a pet (pet.json, coquou.json).
const MAX_JSON_BYTES: u64 = 1024 * 1024;

#[derive(Default)]
struct Budget {
    files: usize,
    bytes: u64,
}

impl Budget {
    fn add_file(&mut self) -> Result<(), String> {
        self.files += 1;
        if self.files > MAX_FILES {
            return Err(tr!(format!("文件太多（上限 {MAX_FILES} 个）"), format!("Too many files (at most {MAX_FILES})")));
        }
        Ok(())
    }

    /// Bytes the next file may still use.
    fn room(&self) -> u64 {
        MAX_FILE_BYTES.min(MAX_TOTAL_BYTES - self.bytes)
    }

    fn charge(&mut self, len: u64) -> Result<(), String> {
        if len > self.room() {
            return Err(tr!("宠物包太大", "The pet package is too large").into());
        }
        self.bytes += len;
        Ok(())
    }
}

/// Junk that archivers and Finder add; never part of a pet.
fn is_junk(path: &Path) -> bool {
    path.components().any(|c| match c {
        Component::Normal(s) => {
            let s = s.to_string_lossy();
            s.starts_with('.') || s == "__MACOSX" || s == "Thumbs.db"
        }
        _ => true,
    })
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| tr!("不是有效的 .pet / .zip 文件", "Not a valid .pet / .zip file").to_string())?;
    let mut budget = Budget::default();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let rel = entry
            .enclosed_name()
            .ok_or_else(|| tr!(format!("压缩包里有非法路径：{}", entry.name()), format!("Invalid path in the archive: {}", entry.name())))?;
        if entry.is_dir() || entry.is_symlink() || is_junk(&rel) {
            continue;
        }
        budget.add_file()?;
        let out = dest.join(&rel);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut writer = fs::File::create(&out).map_err(|e| e.to_string())?;
        // Count the bytes really decompressed, never the declared size.
        let room = budget.room();
        let copied = io::copy(&mut (&mut entry).take(room + 1), &mut writer)
            .map_err(|e| format!("{}: {e}", rel.display()))?;
        budget.charge(copied)?;
    }
    Ok(())
}

fn copy_dir(src: &Path, dest: &Path, budget: &mut Budget) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(src).map_err(|e| e.to_string())?.flatten() {
        let name = PathBuf::from(entry.file_name());
        let meta = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
        if is_junk(&name) || meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            copy_dir(&entry.path(), &dest.join(&name), budget)?;
        } else {
            budget.add_file()?;
            budget.charge(meta.len())?;
            fs::copy(entry.path(), dest.join(&name)).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// The pet is either the staging root itself or its single sub-directory
/// (archives made by "compress folder" nest everything one level down).
fn locate_pet_dir(staging: &Path) -> Result<PathBuf, String> {
    if staging.join("pet.json").is_file() {
        return Ok(staging.to_path_buf());
    }
    let dirs: Vec<PathBuf> = fs::read_dir(staging)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    match dirs.as_slice() {
        [only] if only.join("pet.json").is_file() => Ok(only.clone()),
        _ => Err(tr!("没找到 pet.json，这不是一个宠物包", "No pet.json found; this is not a pet package").into()),
    }
}

/// A plain relative path inside the pet: no root, prefix, `..` or `.`.
fn safe_rel(path: &str) -> bool {
    !path.is_empty() && Path::new(path).components().all(|c| matches!(c, Component::Normal(_)))
}

/// The first bytes of a file (enough for PNG / WebP size headers).
fn read_header(path: &Path) -> Option<Vec<u8>> {
    let mut head = Vec::with_capacity(64);
    fs::File::open(path).ok()?.take(64).read_to_end(&mut head).ok()?;
    Some(head)
}

/// A small text file, refusing anything larger than `MAX_JSON_BYTES`.
fn read_json_text(path: &Path) -> Option<Result<String, String>> {
    let meta = fs::metadata(path).ok().filter(|m| m.is_file())?;
    if meta.len() > MAX_JSON_BYTES {
        let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
        return Some(Err(tr!(format!("{name} 太大"), format!("{name} is too large"))));
    }
    Some(fs::read_to_string(path).map_err(|e| e.to_string()))
}

/// Width/height from a PNG or WebP header.
fn image_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let u16le = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]) as u32;
    let u24le = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], 0]);
    if bytes.len() >= 24 && bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
        let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
        return Some((w, h));
    }
    if bytes.len() >= 30 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return match &bytes[12..16] {
            b"VP8 " => Some((u16le(26) & 0x3fff, u16le(28) & 0x3fff)),
            b"VP8L" => {
                let b = u32::from_le_bytes(bytes[21..25].try_into().ok()?);
                Some((1 + (b & 0x3fff), 1 + ((b >> 14) & 0x3fff)))
            }
            b"VP8X" => Some((1 + u24le(24), 1 + u24le(27))),
            _ => None,
        };
    }
    None
}

/// Speech lines: { "<occasion>": ["…", …] } (doc/pet-format.md).
pub type Lines = std::collections::BTreeMap<String, Vec<String>>;

/// The occasions the overlay speaks on.
pub const LINE_KINDS: [&str; 9] = ["hello", "click", "pet", "wake", "held", "idle", "working", "failed", "waiting"];

/// Bounded sizes for lines, from a pet file or the user's own (prefs.rs).
pub fn check_lines(lines: &Lines) -> Result<(), String> {
    const MAX_PER_KIND: usize = 50;
    const MAX_CHARS: usize = 80;
    for (kind, list) in lines {
        if list.len() > MAX_PER_KIND {
            return Err(tr!(format!("lines.{kind} 最多 {MAX_PER_KIND} 句"), format!("lines.{kind} can have at most {MAX_PER_KIND} lines")));
        }
        if list.iter().any(|t| t.chars().count() > MAX_CHARS) {
            return Err(tr!(
                format!("lines.{kind} 里有一句超过 {MAX_CHARS} 个字"),
                format!("A line in lines.{kind} is longer than {MAX_CHARS} characters")
            ));
        }
    }
    Ok(())
}

fn parse_lines(value: &serde_json::Value) -> Result<Lines, String> {
    serde_json::from_value(value.clone()).map_err(|_| tr!("coquou.json 的 lines 必须是 { 场合: [字符串…] }", "lines in coquou.json must be { occasion: [string…] }").to_string())
}

/// `translations`: { "<language>": { "displayName"?, "description"?, "lines"? } },
/// a language being a lowercase 2–3 letter code such as "en" or "zh"
/// (doc/pet-format.md). Other fields of an entry are left for later format
/// versions.
fn check_translations(value: &serde_json::Value) -> Result<(), String> {
    const MAX_LANGUAGES: usize = 16;
    let shape = || {
        tr!(
            "coquou.json 的 translations 必须是 { 语言: { lines: … } }",
            "translations in coquou.json must be { language: { lines: … } }"
        )
        .to_string()
    };
    let map = value.as_object().ok_or_else(shape)?;
    if map.len() > MAX_LANGUAGES {
        return Err(tr!(
            format!("coquou.json 的 translations 最多 {MAX_LANGUAGES} 种语言"),
            format!("translations in coquou.json can have at most {MAX_LANGUAGES} languages")
        ));
    }
    for (language, entry) in map {
        if !(2..=3).contains(&language.len()) || !language.bytes().all(|b| b.is_ascii_lowercase()) {
            return Err(tr!(
                format!("coquou.json 的 translations 里语言代码不合法：\"{language}\"（用 en、zh 这样的小写代码）"),
                format!("Invalid language code in translations in coquou.json: \"{language}\" (use lowercase codes like en or zh)")
            ));
        }
        let entry = entry.as_object().ok_or_else(shape)?;
        for (field, max) in [("displayName", 64), ("description", 500)] {
            let Some(value) = entry.get(field) else { continue };
            if !value.as_str().is_some_and(|s| s.chars().count() <= max) {
                return Err(tr!(
                    format!("coquou.json 的 translations.{language}.{field} 必须是最多 {max} 个字的字符串"),
                    format!("translations.{language}.{field} in coquou.json must be a string of at most {max} characters")
                ));
            }
        }
        if let Some(lines) = entry.get("lines") {
            check_lines(&parse_lines(lines)?)?;
        }
    }
    Ok(())
}

/// `hatch`: what an AI hatch was made from, kept so the hatch page can load
/// the pet again and regenerate parts of it (doc/pet-format.md). Images only:
/// `base` (required), `reference`, and one raw sheet per entry of `rows`;
/// rows without a sheet are taken from the atlas.
fn check_hatch(dir: &Path, value: &serde_json::Value) -> Result<(), String> {
    const MAX_ROWS: usize = 16;
    let bad = |field: &str| tr!(format!("coquou.json 的 hatch.{field} 不合法"), format!("hatch.{field} in coquou.json is invalid"));
    let image = |field: &str, path: &serde_json::Value| -> Result<(), String> {
        let ok = path.as_str().is_some_and(|p| safe_rel(p) && read_header(&dir.join(p)).as_deref().and_then(image_size).is_some());
        if ok {
            Ok(())
        } else {
            Err(tr!(
                format!("coquou.json 的 hatch.{field} 不存在，或不是 PNG / WebP 图片"),
                format!("hatch.{field} in coquou.json is missing or not a PNG / WebP image")
            ))
        }
    };
    let map = value.as_object().ok_or_else(|| tr!("coquou.json 的 hatch 必须是对象", "hatch in coquou.json must be an object").to_string())?;
    if !matches!(map.get("key").and_then(|v| v.as_str()), Some("green" | "magenta")) {
        return Err(bad("key"));
    }
    for (field, max) in [("style", 32), ("description", 2000)] {
        if map.get(field).is_some_and(|v| !v.as_str().is_some_and(|s| s.chars().count() <= max)) {
            return Err(bad(field));
        }
    }
    if map.get("withLook").is_some_and(|v| !v.is_boolean()) {
        return Err(bad("withLook"));
    }
    image("base", map.get("base").unwrap_or(&serde_json::Value::Null))?;
    if let Some(reference) = map.get("reference") {
        image("reference", reference)?;
    }
    if let Some(rows) = map.get("rows") {
        let rows = rows.as_object().filter(|r| r.len() <= MAX_ROWS).ok_or_else(|| bad("rows"))?;
        for (id, path) in rows {
            let id_ok = (1..=40).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
            if !id_ok {
                return Err(bad("rows"));
            }
            image(&format!("rows.{id}"), path)?;
        }
    }
    Ok(())
}

/// The extension file: `coquou.json`, or `perchy.json` from before the rename.
pub const EXT_FILES: [&str; 2] = ["coquou.json", "perchy.json"];

/// Animation names the overlay plays (src/ui/sprite.js `choose`): the Codex
/// rows, which `states` may override, and the extension states. Any other
/// name in `states` is accepted, so packages made for a later Coquou still
/// install, but it is reported as unused.
pub const STATE_NAMES: [&str; 17] = [
    "idle", "running-right", "running-left", "waving", "jumping", "failed", "waiting", "running", "review",
    "eat", "held", "rise", "fall", "land", "sleep", "happy", "loved",
];

/// The `states` nothing plays: names not in STATE_NAMES, and `rise`
/// without `fall` (the overlay only splits the flight in two when the
/// pet has a falling animation).
fn unused_states(states: &serde_json::Map<String, serde_json::Value>) -> Vec<String> {
    states
        .keys()
        .filter(|n| !STATE_NAMES.contains(&n.as_str()) || (*n == "rise" && !states.contains_key("fall")))
        .cloned()
        .collect()
}

/// What validating a pet found.
#[derive(Debug)]
struct Checked {
    id: String,
    display_name: String,
    unused_states: Vec<String>,
}

/// Check the pet against doc/pet-format.md.
fn validate(dir: &Path) -> Result<Checked, String> {
    let text = read_json_text(&dir.join("pet.json")).ok_or(tr!("读不了 pet.json", "Cannot read pet.json"))??;
    let manifest: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| tr!(format!("pet.json 格式错误：{e}"), format!("pet.json is malformed: {e}")))?;

    let id = manifest["id"].as_str().unwrap_or_default().to_string();
    let id_ok = (1..=64).contains(&id.len())
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && !id.starts_with(['-', '_']);
    if !id_ok {
        return Err(tr!(
            format!("pet.json 里的 id 不合法：\"{id}\"（只能用字母、数字、- 和 _）"),
            format!("Invalid id in pet.json: \"{id}\" (letters, digits, - and _ only)")
        ));
    }

    let sheet = manifest["spritesheetPath"].as_str().unwrap_or("spritesheet.webp");
    if !safe_rel(sheet) {
        return Err(tr!(format!("spritesheetPath 不合法：{sheet}"), format!("Invalid spritesheetPath: {sheet}")));
    }
    let head = read_header(&dir.join(sheet)).ok_or_else(|| tr!(format!("缺少精灵图 {sheet}"), format!("Missing spritesheet {sheet}")))?;
    let (w, h) = image_size(&head).ok_or(tr!("精灵图必须是 PNG 或 WebP", "The spritesheet must be PNG or WebP"))?;
    let version = manifest["spriteVersionNumber"].as_u64().unwrap_or(1);
    let rows = if version >= 2 { 11 } else { 9 };
    let (cw, ch) = CODEX_CELL;
    if (w, h) != (cw * 8, ch * rows) {
        let (nw, nh) = (cw * 8, ch * rows);
        return Err(tr!(
            format!("精灵图尺寸 {w}×{h} 不对，Codex v{version} 需要 {nw}×{nh}"),
            format!("The spritesheet is {w}×{h}; Codex v{version} needs {nw}×{nh}")
        ));
    }

    let mut ext_value = None;
    if let Some(text) = EXT_FILES.iter().find_map(|f| read_json_text(&dir.join(f))) {
        let ext: serde_json::Value =
            serde_json::from_str(&text?).map_err(|e| tr!(format!("coquou.json 格式错误：{e}"), format!("coquou.json is malformed: {e}")))?;
        if let Some(hd) = ext.get("hd").and_then(|v| v.as_str()) {
            let size = safe_rel(hd).then(|| read_header(&dir.join(hd))).flatten().as_deref().and_then(image_size);
            let ok = size.is_some_and(|(hw, hh)| hw % w == 0 && hw / w >= 2 && hw / w <= 4 && hw * h == hh * w);
            if !ok {
                return Err(tr!(
                    format!("coquou.json 的 hd 图集 {hd} 不存在，或不是主图集的 2–4 倍"),
                    format!("The hd atlas {hd} in coquou.json is missing or not 2–4× the main atlas")
                ));
            }
        }
        if let Some(facing) = ext.get("facing") {
            if !matches!(facing.as_str(), Some("left" | "right" | "front")) {
                return Err(tr!("coquou.json 的 facing 只能是 left、right 或 front", "facing in coquou.json must be left, right or front").into());
            }
        }
        if let Some(lines) = ext.get("lines") {
            check_lines(&parse_lines(lines)?)?;
        }
        if let Some(translations) = ext.get("translations") {
            check_translations(translations)?;
        }
        if let Some(hatch) = ext.get("hatch") {
            check_hatch(dir, hatch)?;
        }
        if let Some(states) = ext["states"].as_object() {
            for (name, state) in states {
                if let Some(sheet) = state["sheet"].as_str() {
                    if !safe_rel(sheet) || !dir.join(sheet).is_file() {
                        return Err(tr!(
                            format!("coquou.json 的状态 {name} 引用的 {sheet} 不存在"),
                            format!("{sheet}, used by state {name} in coquou.json, does not exist")
                        ));
                    }
                }
            }
        }
        ext_value = Some(ext);
    }
    let display_name = pets::own_name(&manifest, ext_value.as_ref(), &id);
    let unused_states = ext_value.as_ref().and_then(|ext| ext["states"].as_object()).map(unused_states).unwrap_or_default();
    Ok(Checked { id, display_name, unused_states })
}

/// Hidden work directories inside the pets root (never listed as pets).
const STAGING_PREFIX: &str = ".staging-";
const OLD_PREFIX: &str = ".old-";

fn nonce() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

/// Remove work directories a crash may have left behind.
pub fn sweep<R: Runtime>(app: &AppHandle<R>) {
    let Some(root) = pets::coquou_root(app) else { return };
    let Ok(entries) = fs::read_dir(&root) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with(STAGING_PREFIX) || name.starts_with(OLD_PREFIX) {
            if let Err(err) = fs::remove_dir_all(e.path()) {
                log::warn!("cannot remove leftover {}: {err}", e.path().display());
            }
        }
    }
}

/// Move `new` to `dest`, keeping the previous `dest` until the move worked.
fn swap_into_place(new: &Path, dest: &Path, root: &Path) -> Result<bool, String> {
    if !dest.exists() {
        fs::rename(new, dest).map_err(|e| e.to_string())?;
        return Ok(false);
    }
    let old = root.join(format!("{OLD_PREFIX}{}", nonce()));
    fs::rename(dest, &old).map_err(|e| e.to_string())?;
    if let Err(e) = fs::rename(new, dest) {
        let _ = fs::rename(&old, dest);
        return Err(e.to_string());
    }
    if let Err(e) = fs::remove_dir_all(&old) {
        log::warn!("cannot remove replaced pet {}: {e}", old.display());
    }
    Ok(true)
}

/// Coquou's pets folder (created) and a fresh staging path inside it, where
/// `sweep` finds it again if Coquou dies before cleaning up.
fn new_staging<R: Runtime>(app: &AppHandle<R>) -> Result<(PathBuf, PathBuf), String> {
    let root = pets::coquou_root(app).ok_or(tr!("找不到应用数据目录", "Cannot find the app data folder"))?;
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let staging = root.join(format!("{STAGING_PREFIX}{}", nonce()));
    Ok((root, staging))
}

/// Put a copy of `pet`'s files at `dest`, under the same limits as installing.
fn stage(pet: &Pet, dest: &Path) -> Result<(), String> {
    match pet {
        Pet::Dir(dir) => copy_dir(dir, dest, &mut Budget::default()),
        Pet::Preset(p) => p.write_to(dest),
    }
}

pub fn install<R: Runtime>(app: &AppHandle<R>, src: &Path) -> Result<Installed, String> {
    let (root, staging) = new_staging(app)?;

    let result = (|| {
        if src.is_dir() {
            copy_dir(src, &staging, &mut Budget::default())?;
        } else {
            let ext = src.extension().map(|e| e.to_string_lossy().to_lowercase());
            match ext.as_deref() {
                Some("pet") | Some("zip") => {
                    fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
                    extract_zip(src, &staging)?;
                }
                _ => return Err(tr!("只支持 .pet、.zip 或宠物文件夹", "Only .pet, .zip or pet folders are supported").into()),
            }
        }
        let pet_dir = locate_pet_dir(&staging)?;
        let Checked { id, display_name, unused_states } = validate(&pet_dir)?;
        if !unused_states.is_empty() {
            log::warn!("{id}: coquou.json states never played: {}", unused_states.join(", "));
        }
        let replaced = swap_into_place(&pet_dir, &root.join(&id), &root)?;
        Ok(Installed { key: format!("coquou:{id}"), display_name, replaced, unused_states })
    })();

    let _ = fs::remove_dir_all(&staging);
    result
}

/// Pack a pet into `<Downloads>/<id>.pet` (zip, files at the root, same junk
/// filter and size limits as installing). Returns the file path.
pub fn export<R: Runtime>(app: &AppHandle<R>, key: &str) -> Result<PathBuf, String> {
    let pet = pets::locate(app, key)?;
    if let Pet::Dir(dir) = &pet {
        return export_dir(app, dir);
    }
    let (_, staging) = new_staging(app)?;
    let result = stage(&pet, &staging).and_then(|()| export_dir(app, &staging));
    let _ = fs::remove_dir_all(&staging);
    result
}

fn export_dir<R: Runtime>(app: &AppHandle<R>, dir: &Path) -> Result<PathBuf, String> {
    use tauri::Manager;
    use zip::write::SimpleFileOptions;

    let id = validate(dir)?.id;
    let out_dir = app.path().download_dir().map_err(|e| e.to_string())?;
    let mut out = out_dir.join(format!("{id}.pet"));
    let mut n = 2;
    while out.exists() {
        out = out_dir.join(format!("{id}-{n}.pet"));
        n += 1;
    }

    let mut files = Vec::new();
    collect(dir, dir, &mut files, &mut Budget::default())?;
    let tmp = out.with_extension("pet.part");
    let result = (|| {
        let mut zip = zip::ZipWriter::new(fs::File::create(&tmp).map_err(|e| e.to_string())?);
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for rel in &files {
            let name = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
            zip.start_file(name, opts).map_err(|e| e.to_string())?;
            io::copy(&mut fs::File::open(dir.join(rel)).map_err(|e| e.to_string())?, &mut zip).map_err(|e| e.to_string())?;
        }
        zip.finish().map_err(|e| e.to_string())?;
        fs::rename(&tmp, &out).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map(|_| out)
}

/// Copy a pet into Coquou's pets folder as a new pet of the user's own (a
/// read-only preset or Codex pet becomes one they can delete or replace):
/// a free id, the name marked as a copy and no `preset` tag. The
/// user's own touches to the original (prefs.rs: `own` name and lines) are
/// written into the copy, so it starts out as the user sees the original.
/// Same limits and validation as installing.
pub fn duplicate<R: Runtime>(app: &AppHandle<R>, key: &str, own: &Own) -> Result<Installed, String> {
    let pet = pets::locate(app, key)?;
    let (root, staging) = new_staging(app)?;
    let result = (|| {
        stage(&pet, &staging)?;
        let old_id = validate(&staging)?.id;
        mark_copy(&staging, &copy_id(&root, &old_id), tr!(" 副本", " copy"), own)?;
        let Checked { id, display_name, unused_states } = validate(&staging)?;
        let dest = root.join(&id);
        // Never replace a pet: the id was free a moment ago.
        if dest.exists() {
            return Err(tr!(format!("已经有 ID 为 {id} 的宠物了"), format!("A pet with the id {id} already exists")));
        }
        fs::rename(&staging, &dest).map_err(|e| e.to_string())?;
        Ok(Installed { key: format!("coquou:{id}"), display_name, replaced: false, unused_states })
    })();
    if staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

/// The user's name and lines for a pet (prefs.rs `Custom`), carried into a copy.
#[derive(Default)]
pub struct Own<'a> {
    pub name: Option<&'a str>,
    pub lines: Option<&'a Lines>,
}

/// The first of `<id>-copy`, `<id>-copy-2`, … not taken in `root`; a copy of a
/// copy counts on from the original's id instead of growing `-copy-copy`.
fn copy_id(root: &Path, id: &str) -> String {
    let base = match id.rsplit_once("-copy") {
        Some((b, n)) if !b.is_empty() && (n.is_empty() || n.strip_prefix('-').is_some_and(|d| !d.is_empty() && d.bytes().all(|c| c.is_ascii_digit()))) => b,
        _ => id,
    };
    // Room for "-copy-NNNN" within the 64-character id limit (ids are ASCII).
    let base = &base[..base.len().min(52)];
    (1..)
        .map(|n| if n == 1 { format!("{base}-copy") } else { format!("{base}-copy-{n}") })
        .find(|c| !root.join(c).exists())
        .expect("an unbounded range always has a free id")
}

/// Give the pet in `dir` its new id and mark its name as a copy; drop the
/// `preset` tag. The user's own name and lines replace the pet's, in every
/// language: the user's touches apply whatever the interface language, so
/// the translations they override are removed rather than left to win.
fn mark_copy(dir: &Path, id: &str, suffix: &str, own: &Own) -> Result<(), String> {
    let read = |file: &Path| -> Result<serde_json::Value, String> {
        let text = read_json_text(file).ok_or_else(|| tr!(format!("读不了 {}", file.display()), format!("Cannot read {}", file.display())))??;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    };
    let write = |file: &Path, value: &serde_json::Value| {
        fs::write(file, serde_json::to_string_pretty(value).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    };

    let manifest_file = dir.join("pet.json");
    let mut m = read(&manifest_file)?;
    let name = own.name.map(str::to_string).unwrap_or_else(|| {
        m["displayName"].as_str().filter(|n| !n.trim().is_empty()).unwrap_or(m["id"].as_str().unwrap_or(id)).to_string()
    });
    m["id"] = id.into();
    m["displayName"] = format!("{name}{suffix}").into();
    write(&manifest_file, &m)?;

    let ext_file = EXT_FILES.iter().map(|f| dir.join(f)).find(|f| f.is_file());
    let lines = own.lines.filter(|l| !l.is_empty());
    if ext_file.is_none() && lines.is_none() {
        return Ok(());
    }
    let ext_file = ext_file.unwrap_or_else(|| dir.join(EXT_FILES[0]));
    let mut e = if ext_file.is_file() { read(&ext_file)? } else { serde_json::json!({ "format": "coquou/1" }) };
    if let Some(tags) = e.get_mut("tags").and_then(|t| t.as_array_mut()) {
        tags.retain(|t| t != "preset");
    }
    if let Some(translations) = e.get_mut("translations").and_then(|t| t.as_object_mut()) {
        for (lang, t) in translations.iter_mut() {
            let Some(t) = t.as_object_mut() else { continue };
            if own.name.is_some() {
                t.remove("displayName");
            } else if let Some(n) = t.get("displayName").and_then(|n| n.as_str()) {
                let marked = format!("{n}{}", if lang == "zh" { " 副本" } else { " copy" });
                t.insert("displayName".into(), marked.into());
            }
            if let (Some(lines), Some(their)) = (lines, t.get_mut("lines").and_then(|l| l.as_object_mut())) {
                their.retain(|kind, _| !lines.contains_key(kind));
            }
        }
    }
    if let Some(lines) = lines {
        if !e["lines"].is_object() {
            e["lines"] = serde_json::json!({});
        }
        for (kind, list) in lines {
            e["lines"][kind] = serde_json::json!(list);
        }
    }
    write(&ext_file, &e)
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<PathBuf>, budget: &mut Budget) -> Result<(), String> {
    let mut entries: Vec<_> = fs::read_dir(dir).map_err(|e| e.to_string())?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let rel = entry.path().strip_prefix(root).expect("collect walks inside root").to_path_buf();
        let meta = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
        if is_junk(&rel) || meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            collect(root, &entry.path(), out, budget)?;
        } else {
            budget.add_file()?;
            budget.charge(meta.len())?;
            out.push(rel);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_presets_are_valid() {
        let mut ids = Vec::new();
        for p in crate::presets::all() {
            let dir = std::env::temp_dir().join(format!("coquou-preset-{}-{}", p.name, std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            p.write_to(&dir).unwrap();
            let id = validate(&dir).unwrap_or_else(|e| panic!("{}: {e}", p.name)).id;
            fs::remove_dir_all(&dir).unwrap();
            assert_eq!(p.name, id);
            ids.push(id);
        }
        assert_eq!(ids, ["tuanzi"]);
    }

    #[test]
    fn lines_are_checked() {
        use serde_json::json;
        let check = |v: serde_json::Value| parse_lines(&v).and_then(|l| check_lines(&l));
        assert!(check(json!({ "click": ["喵", "嗯？"] })).is_ok());
        assert!(check(json!(["喵"])).is_err());
        assert!(check(json!({ "click": [1] })).is_err());
        assert!(check(json!({ "click": ["x".repeat(81)] })).is_err());
    }

    #[test]
    fn translations_are_checked() {
        use serde_json::json;
        assert!(check_translations(&json!({ "en": { "lines": { "click": ["Meow!"] } } })).is_ok());
        assert!(check_translations(&json!({ "en": {} })).is_ok());
        assert!(check_translations(&json!(["en"])).is_err());
        assert!(check_translations(&json!({ "en": ["Meow!"] })).is_err());
        assert!(check_translations(&json!({ "EN": { "lines": {} } })).is_err());
        assert!(check_translations(&json!({ "../x": { "lines": {} } })).is_err());
        assert!(check_translations(&json!({ "en": { "lines": { "click": [1] } } })).is_err());
        assert!(check_translations(&json!({ "en": { "displayName": "Tuantuan", "description": "A cat." } })).is_ok());
        assert!(check_translations(&json!({ "en": { "displayName": 1 } })).is_err());
        assert!(check_translations(&json!({ "en": { "description": "x".repeat(501) } })).is_err());
    }

    #[test]
    fn png_and_webp_headers() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(1536u32.to_be_bytes());
        png.extend(2288u32.to_be_bytes());
        assert_eq!(image_size(&png), Some((1536, 2288)));
        assert_eq!(image_size(b"nope"), None);
    }

    #[test]
    fn budget_counts_real_bytes() {
        let mut b = Budget::default();
        b.add_file().unwrap();
        b.charge(MAX_FILE_BYTES).unwrap();
        assert_eq!(b.room(), MAX_FILE_BYTES.min(MAX_TOTAL_BYTES - MAX_FILE_BYTES));
        b.charge(MAX_FILE_BYTES).unwrap();
        b.charge(MAX_FILE_BYTES).unwrap();
        assert_eq!(b.room(), 0);
        assert!(b.charge(1).is_err());
        let mut many = Budget::default();
        for _ in 0..MAX_FILES {
            many.add_file().unwrap();
        }
        assert!(many.add_file().is_err());
    }

    /// The preset's pet.json and atlas with the given coquou.json, in a temp dir.
    fn preset_with_ext(ext: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("coquou-test-{}", nonce()));
        let preset = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/assets/presets/tuanzi");
        fs::create_dir_all(&dir).unwrap();
        for f in ["pet.json", "spritesheet.png"] {
            fs::copy(preset.join(f), dir.join(f)).unwrap();
        }
        fs::write(dir.join("coquou.json"), ext).unwrap();
        dir
    }

    #[test]
    fn hd_outside_the_pet_is_rejected() {
        let dir = preset_with_ext(r#"{ "hd": "/dev/zero" }"#);
        let err = validate(&dir).unwrap_err();
        assert!(err.contains("hd"), "{err}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unknown_states_are_reported_not_refused() {
        let dir = preset_with_ext(r#"{ "states": { "sleep": { "row": 0 }, "running": { "row": 7 }, "dance": { "row": 3 } } }"#);
        assert_eq!(validate(&dir).unwrap().unused_states, ["dance"]);
        fs::remove_dir_all(&dir).unwrap();
        let unused = |states: serde_json::Value| unused_states(states.as_object().unwrap());
        assert_eq!(unused(serde_json::json!({ "rise": {}, "land": {} })), ["rise"]);
        assert!(unused(serde_json::json!({ "rise": {}, "fall": {} })).is_empty());
        assert!(unused(serde_json::json!({ "fall": {} })).is_empty());
    }

    #[test]
    fn facing_is_checked() {
        let dir = preset_with_ext(r#"{ "facing": "up" }"#);
        let err = validate(&dir).unwrap_err();
        assert!(err.contains("facing"), "{err}");
        fs::write(dir.join("coquou.json"), r#"{ "facing": "left" }"#).unwrap();
        assert!(validate(&dir).is_ok());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hatch_material_is_checked() {
        let dir = preset_with_ext("{}");
        fs::create_dir_all(dir.join("hatch")).unwrap();
        fs::copy(dir.join("spritesheet.png"), dir.join("hatch/base.png")).unwrap();
        fs::write(dir.join("hatch/notes.txt"), "x").unwrap();
        let check = |hatch: &str| {
            fs::write(dir.join("coquou.json"), format!(r#"{{ "hatch": {hatch} }}"#)).unwrap();
            validate(&dir)
        };
        assert!(check(r#"{ "key": "green", "base": "hatch/base.png", "withLook": true, "rows": { "idle": "hatch/base.png" } }"#).is_ok());
        for bad in [
            r#"{ "key": "blue", "base": "hatch/base.png" }"#,
            r#"{ "key": "green" }"#,
            r#"{ "key": "green", "base": "../spritesheet.png" }"#,
            r#"{ "key": "green", "base": "hatch/notes.txt" }"#,
            r#"{ "key": "green", "base": "hatch/base.png", "reference": "hatch/missing.png" }"#,
            r#"{ "key": "green", "base": "hatch/base.png", "rows": { "../x": "hatch/base.png" } }"#,
            r#"{ "key": "green", "base": "hatch/base.png", "rows": ["hatch/base.png"] }"#,
            r#"{ "key": "green", "base": "hatch/base.png", "withLook": "yes" }"#,
            r#""hatch/base.png""#,
        ] {
            let err = check(bad).unwrap_err();
            assert!(err.contains("hatch"), "{bad}: {err}");
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn copies_get_a_free_id() {
        let root = std::env::temp_dir().join(format!("coquou-test-{}", nonce()));
        fs::create_dir_all(root.join("cat-copy")).unwrap();
        assert_eq!(copy_id(&root, "cat"), "cat-copy-2");
        assert_eq!(copy_id(&root, "cat-copy"), "cat-copy-2");
        assert_eq!(copy_id(&root, "cat-copy-7"), "cat-copy-2");
        assert_eq!(copy_id(&root, "dog"), "dog-copy");
        assert_eq!(copy_id(&root, "my-copycat"), "my-copycat-copy");
        assert_eq!(copy_id(&root, &"a".repeat(64)).len(), 57);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_copy_is_marked_and_still_valid() {
        let dir = preset_with_ext(r#"{ "tags": ["preset", "ai-hatched"], "translations": { "en": { "displayName": "Tuanzi" } } }"#);
        mark_copy(&dir, "tuanzi-copy", " 副本", &Own::default()).unwrap();
        let checked = validate(&dir).unwrap();
        assert_eq!(checked.id, "tuanzi-copy");
        let manifest: serde_json::Value = serde_json::from_str(&fs::read_to_string(dir.join("pet.json")).unwrap()).unwrap();
        assert_eq!(manifest["displayName"], "团子 副本");
        let ext: serde_json::Value = serde_json::from_str(&fs::read_to_string(dir.join("coquou.json")).unwrap()).unwrap();
        assert_eq!(ext["tags"], serde_json::json!(["ai-hatched"]));
        assert_eq!(ext["translations"]["en"]["displayName"], "Tuanzi copy");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_copy_takes_the_users_name_and_lines() {
        let ext = r#"{ "lines": { "click": ["喵！"], "pet": ["呼噜"] },
            "translations": { "en": { "displayName": "Tuanzi", "lines": { "click": ["Meow!"], "pet": ["Purr"] } } } }"#;
        let dir = preset_with_ext(ext);
        let lines: Lines = [("click".to_string(), vec!["嗨".to_string()])].into();
        mark_copy(&dir, "tuanzi-copy", " 副本", &Own { name: Some("小橘"), lines: Some(&lines) }).unwrap();
        validate(&dir).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&fs::read_to_string(dir.join("pet.json")).unwrap()).unwrap();
        assert_eq!(manifest["displayName"], "小橘 副本");
        let ext: serde_json::Value = serde_json::from_str(&fs::read_to_string(dir.join("coquou.json")).unwrap()).unwrap();
        assert_eq!(ext["lines"]["click"], serde_json::json!(["嗨"]));
        assert_eq!(ext["lines"]["pet"], serde_json::json!(["呼噜"]));
        // The user's name and lines win in English too.
        assert!(ext["translations"]["en"].get("displayName").is_none());
        assert_eq!(ext["translations"]["en"]["lines"], serde_json::json!({ "pet": ["Purr"] }));
        fs::remove_dir_all(&dir).unwrap();

        // A pet without coquou.json gets one for the lines.
        let dir = preset_with_ext("{}");
        fs::remove_file(dir.join("coquou.json")).unwrap();
        mark_copy(&dir, "x-copy", " copy", &Own { name: None, lines: Some(&lines) }).unwrap();
        validate(&dir).unwrap();
        let ext: serde_json::Value = serde_json::from_str(&fs::read_to_string(dir.join("coquou.json")).unwrap()).unwrap();
        assert_eq!(ext["lines"]["click"], serde_json::json!(["嗨"]));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn junk_and_paths() {
        assert!(is_junk(Path::new("__MACOSX/x")));
        assert!(is_junk(Path::new("a/.DS_Store")));
        assert!(!is_junk(Path::new("extra/sleep.webp")));
        assert!(!safe_rel("../x.webp"));
        assert!(!safe_rel("/etc/passwd"));
        assert!(safe_rel("extra/sleep.webp"));
    }
}
