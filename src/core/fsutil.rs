//! Small file helpers for the JSON state Coquou keeps on disk.

use serde::de::DeserializeOwned;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// Replace `path` with `bytes` so that a crash or power loss leaves either
/// the old file or the new one, never a truncated mix.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

/// Read a JSON file. Missing → `T::default()`. Unreadable or invalid → the
/// file is kept as `<name>.bad` (so a later save can't destroy it) and the
/// default is used; the problem is logged.
pub fn read_json_or_default<T: DeserializeOwned + Default>(path: &Path) -> T {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return T::default(),
        Err(e) => {
            log::error!("cannot read {}: {e}", path.display());
            return T::default();
        }
    };
    match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            let mut bad = path.as_os_str().to_owned();
            bad.push(".bad");
            let kept = fs::rename(path, &bad).is_ok();
            log::error!(
                "{} is not valid ({e}); starting from defaults{}",
                path.display(),
                if kept { format!(", the old file is kept as {}", Path::new(&bad).display()) } else { String::new() }
            );
            T::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_and_bad_json_is_kept() {
        let dir = std::env::temp_dir().join(format!("coquou-fsutil-{}", std::process::id()));
        let file = dir.join("state.json");
        write_atomic(&file, b"{\"a\":1}").unwrap();
        let v: std::collections::BTreeMap<String, i32> = read_json_or_default(&file);
        assert_eq!(v.get("a"), Some(&1));
        fs::write(&file, b"{ truncated").unwrap();
        let v: std::collections::BTreeMap<String, i32> = read_json_or_default(&file);
        assert!(v.is_empty());
        assert!(dir.join("state.json.bad").exists());
        assert!(!file.exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
