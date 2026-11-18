//! Pet library commands: install (drop, double-click, import, hatch), export,
//! reveal, delete. Validation and the .pet format live in install.rs.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use serde::Serialize;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

use crate::install::{self, Installed};
use crate::sync::lock;

/// What happened before the overlay page was listening: the pet to show
/// first (one installed, or named with `--pet`) and installs that failed
/// (files given at launch). Events sent then would be lost, so the overlay
/// claims these with `overlay_ready`; after that, they reach it as
/// `pet-installed` / `pet-install-failed` events. One lock covers `ready`
/// and the queue, so nothing is queued after the overlay has claimed it.
#[derive(Default)]
pub struct Startup(Mutex<Early>);

#[derive(Default, Serialize)]
pub struct Early {
    #[serde(skip)]
    ready: bool,
    pet: Option<String>,
    failed: Vec<InstallFailure>,
}

/// Payload of `pet-install-failed`.
#[derive(Serialize, Clone)]
pub struct InstallFailure {
    file: Option<String>,
    error: String,
}

pub fn set_pending<R: Runtime>(app: &AppHandle<R>, key: String) {
    let startup = app.state::<Startup>();
    let mut early = lock(&startup.0);
    if !early.ready {
        early.pet = Some(key);
    }
}

/// Tell the overlay an install failed, or keep it until the overlay is ready.
fn report_failure<R: Runtime>(app: &AppHandle<R>, failure: InstallFailure) {
    {
        let startup = app.state::<Startup>();
        let mut early = lock(&startup.0);
        if !early.ready {
            early.failed.push(failure);
            return;
        }
    }
    let _ = app.emit("pet-install-failed", failure);
}

/// The overlay is listening now; returns the pet it should start with and
/// the installs that failed before.
#[tauri::command]
pub fn overlay_ready(startup: State<Startup>) -> Early {
    let mut early = lock(&startup.0);
    let claimed = std::mem::take(&mut *early);
    early.ready = true;
    claimed
}

/// A fresh path in the app cache for temporary install input.
fn scratch<R: Runtime>(app: &AppHandle<R>, stem: &str, ext: &str) -> Result<PathBuf, String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = app.path().app_cache_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    Ok(dir.join(format!("{stem}-{}-{n}{ext}", std::process::id())))
}

/// Install one pet and announce it (`pet-installed`). Every install goes
/// through here, so this is where the pet shows it checking the package and
/// looks sad when it is refused.
fn install_one<R: Runtime>(app: &AppHandle<R>, path: &Path) -> Result<Installed, String> {
    let busy = crate::activity::begin(app, crate::activity::Kind::Review);
    let result = install::install(app, path);
    drop(busy);
    let done = result.inspect_err(|_| crate::activity::failed(app))?;
    log::info!("installed {} from {}", done.key, path.display());
    set_pending(app, done.key.clone());
    let _ = app.emit("pet-installed", done.clone());
    Ok(done)
}

/// Install pets from dropped / opened paths on a worker thread (archives can
/// be large); failures become `pet-install-failed` events.
pub fn install_paths_in_background<R: Runtime>(app: &AppHandle<R>, paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || install_paths(&app, paths));
}

/// Install pets from dropped / opened paths; failures become
/// `pet-install-failed` events (the overlay says what went wrong).
pub fn install_paths<R: Runtime>(app: &AppHandle<R>, paths: Vec<PathBuf>) {
    for path in paths {
        if let Err(error) = install_one(app, &path) {
            log::warn!("install failed for {}: {error}", path.display());
            report_failure(app, InstallFailure { file: path.file_name().map(|n| n.to_string_lossy().into_owned()), error });
        }
    }
}

/// Save a pet assembled on the hatch page (Codex v2 PNG atlas, its 2× HD copy,
/// optional coquou.json), then install it through the same validated path as a .pet.
/// `hatch_files` (name → PNG) is the hatch material kept for editing the pet
/// later, written to `hatch/<name>.png` and listed in coquou.json `hatch`
/// (doc/pet-format.md); install validates what coquou.json references.
#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub fn install_generated(
    app: AppHandle,
    id: String,
    display_name: String,
    description: String,
    spritesheet_png: String,
    hd_png: String,
    coquou_json: Option<String>,
    hatch_files: Option<std::collections::BTreeMap<String, String>>,
) -> Result<Installed, String> {
    let dir = scratch(&app, "hatched", "")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let manifest = serde_json::json!({
            "id": id,
            "displayName": display_name,
            "description": description,
            "spriteVersionNumber": 2,
            "spritesheetPath": "spritesheet.png",
        });
        let write = |name: &str, bytes: &[u8]| std::fs::write(dir.join(name), bytes).map_err(|e| e.to_string());
        write("pet.json", serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?.as_bytes())?;
        write("spritesheet.png", &B64.decode(spritesheet_png).map_err(|e| e.to_string())?)?;
        // The 2× copy named by coquou.json `hd`; install checks its size.
        write("spritesheet@2x.png", &B64.decode(hd_png).map_err(|e| e.to_string())?)?;
        if let Some(ext) = coquou_json {
            write("coquou.json", ext.as_bytes())?;
        }
        if let Some(files) = hatch_files.filter(|f| !f.is_empty()) {
            std::fs::create_dir_all(dir.join("hatch")).map_err(|e| e.to_string())?;
            for (name, png) in files {
                let name = crate::hatch_run::checked(&name)?;
                write(&format!("hatch/{name}.png"), &B64.decode(png).map_err(|e| e.to_string())?)?;
            }
        }
        install_one(&app, &dir)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// Install a .pet / .zip picked in the app window: the bytes arrive as the
/// raw request body, the file name (percent-encoded) in the `x-name` header.
#[tauri::command(async)]
pub fn install_bytes(app: AppHandle, request: tauri::ipc::Request<'_>) -> Result<Installed, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else { return Err("expected raw bytes".into()) };
    let name = request
        .headers()
        .get("x-name")
        .and_then(|v| v.to_str().ok())
        .map(percent_decode)
        .unwrap_or_else(|| "pet.pet".into());
    let ext = if name.to_lowercase().ends_with(".zip") { ".zip" } else { ".pet" };
    let file = scratch(&app, "import", ext)?;
    std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
    let result = install_one(&app, &file);
    let _ = std::fs::remove_file(&file);
    result.map_err(|e| format!("{name}：{e}"))
}

/// Paths of the last drop on the app window. The page decides what they are
/// (a hatch reference image or pets to install); these two commands only
/// accept exactly these paths.
#[derive(Default)]
pub struct Dropped(Mutex<Vec<PathBuf>>);

pub fn set_dropped<R: Runtime>(app: &AppHandle<R>, paths: Vec<PathBuf>) {
    *lock(&app.state::<Dropped>().0) = paths;
}

#[tauri::command(async)]
pub fn install_dropped(app: AppHandle, dropped: State<Dropped>) {
    let paths = std::mem::take(&mut *lock(&dropped.0));
    install_paths(&app, paths);
}

/// Largest reference image accepted from a drop.
const MAX_DROPPED_IMAGE: u64 = 40 * 1024 * 1024;

#[tauri::command(async)]
pub fn read_dropped(dropped: State<Dropped>, path: String) -> Result<tauri::ipc::Response, String> {
    let file = lock(&dropped.0).iter().find(|p| p.display().to_string() == path).cloned().ok_or(tr!("不是刚拖进来的文件", "Not one of the files just dropped"))?;
    let meta = std::fs::metadata(&file).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > MAX_DROPPED_IMAGE {
        return Err(tr!("图片太大或不是文件", "The image is too large or not a file").into());
    }
    std::fs::read(&file).map(tauri::ipc::Response::new).map_err(|e| e.to_string())
}

/// Export a pet as `<Downloads>/<id>.pet` and reveal it.
#[tauri::command(async)]
pub fn export_pet(app: AppHandle, key: String) -> Result<String, String> {
    let path = install::export(&app, &key)?;
    crate::windows::reveal(&path);
    log::info!("exported {key} → {}", path.display());
    Ok(path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
}

/// Show one pet's folder in Finder / Explorer (selected, never opened: a
/// folder named `*.app` must not launch).
#[tauri::command(async)]
pub fn reveal_pet(app: AppHandle, key: String) -> Result<(), String> {
    crate::windows::reveal(&crate::pets::pet_dir(&app, &key)?);
    Ok(())
}

/// Delete a pet installed into Coquou (presets and Codex pets are read-only).
#[tauri::command(async)]
pub fn delete_pet(app: AppHandle, key: String) -> Result<(), String> {
    if !key.starts_with("coquou:") {
        return Err(tr!("只能删除安装到 Coquou 的宠物", "Only pets installed in Coquou can be deleted").into());
    }
    // A real directory directly inside the Coquou pets folder (pet_dir refuses links).
    let dir = crate::pets::pet_dir(&app, &key)?;
    std::fs::remove_dir_all(&dir).map_err(|e| tr!(format!("删除失败：{e}"), format!("Could not delete: {e}")))?;
    log::info!("deleted {key}");
    crate::prefs::forget_pet(&app, &key);
    let _ = app.emit("pets-changed", serde_json::json!({ "removed": key }));
    Ok(())
}

/// %XX decoding (UTF-8) for the file-name header; bad escapes stay as-is.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 3 <= bytes.len() {
            if let Some(v) = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::percent_decode;

    #[test]
    fn decodes_file_names() {
        assert_eq!(percent_decode("%E6%A9%98%E5%AD%90.pet"), "橘子.pet");
        assert_eq!(percent_decode("a%20b.zip"), "a b.zip");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz.pet"), "%zz.pet");
        assert_eq!(percent_decode("end%4"), "end%4");
    }
}
