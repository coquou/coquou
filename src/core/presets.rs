//! Preset pets, compiled into the executable: build.rs embeds every pet
//! directory under src/assets/presets/.
//!
//! They used to ship as files next to the app, and an installer that only
//! writes the new version's files (Windows in-app updates) left a renamed
//! preset behind as a second one. In the binary, the presets are always
//! exactly the ones this version has, however it was installed.

use std::fs;
use std::path::Path;

pub struct Preset {
    /// Its directory name in src/assets/presets/: the key is `preset:<name>`.
    pub name: &'static str,
    /// ('/'-separated path inside the pet, contents), sorted by path.
    files: &'static [(&'static str, &'static [u8])],
}

include!(concat!(env!("OUT_DIR"), "/presets.rs"));

pub fn all() -> &'static [Preset] {
    PRESETS
}

pub fn get(name: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.name == name)
}

impl Preset {
    /// A file of the pet by its exact path; nothing else can match, so a path
    /// from the page needs no further checking.
    pub fn file(&self, path: &str) -> Option<&'static [u8]> {
        self.files.iter().find(|(p, _)| *p == path).map(|(_, bytes)| *bytes)
    }

    /// Write the pet out as an ordinary pet directory at `dest`, for the code
    /// that works on directories (copying, exporting).
    pub fn write_to(&self, dest: &Path) -> Result<(), String> {
        for (rel, bytes) in self.files {
            let path = dest.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(&path, bytes).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn files_are_found_by_exact_path() {
        let p = super::get("tuanzi").expect("the tuanzi preset is embedded");
        assert!(p.file("pet.json").is_some());
        for bad in ["", "./pet.json", "../tuanzi/pet.json", "/pet.json", "PET.JSON"] {
            assert!(p.file(bad).is_none(), "{bad}");
        }
        assert!(super::get("tuantuan").is_none());
    }
}
