use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    embed_presets();
    tauri_build::try_build(
        tauri_build::Attributes::new().capabilities_path_pattern("./src/core/capabilities/**/*"),
    )
    .expect("tauri build script failed");
}

/// Write `$OUT_DIR/presets.rs`, the `PRESETS` table that presets.rs includes:
/// every pet directory under src/assets/presets/ with each of its files
/// embedded by `include_bytes!`, sorted so the output is reproducible.
fn embed_presets() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let root = Path::new(&manifest).join("src/assets/presets");
    println!("cargo:rerun-if-changed={}", root.display());

    let mut code = String::from("pub static PRESETS: &[Preset] = &[\n");
    for dir in sorted_entries(&root) {
        assert!(dir.is_dir(), "{}: only pet directories belong in src/assets/presets", dir.display());
        let name = file_name(&dir);
        let mut files = Vec::new();
        collect(&dir, "", &mut files);
        writeln!(code, "    Preset {{ name: {name:?}, files: &[").unwrap();
        for (rel, path) in files {
            let path = path.to_str().expect("preset paths are UTF-8");
            writeln!(code, "        ({rel:?}, include_bytes!({path:?})),").unwrap();
        }
        code.push_str("    ] },\n");
    }
    code.push_str("];\n");

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("presets.rs");
    std::fs::write(&out, code).expect("cannot write presets.rs");
}

/// Files under `dir` as ('/'-separated path relative to the pet, absolute path).
/// Hidden files (.DS_Store and the like) are not part of a pet.
fn collect(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) {
    for path in sorted_entries(dir) {
        let rel = format!("{prefix}{}", file_name(&path));
        if path.is_dir() {
            collect(&path, &format!("{rel}/"), out);
        } else {
            out.push((rel, path));
        }
    }
}

fn sorted_entries(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.expect("readable directory entry").path())
        .filter(|p| !file_name(p).starts_with('.'))
        .collect();
    entries.sort();
    entries
}

fn file_name(path: &Path) -> String {
    path.file_name().and_then(|n| n.to_str()).expect("preset file names are UTF-8").to_string()
}
