use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UiPrefs {
    pub x: i32,
    pub y: i32,
}

pub fn load(path: &Path) -> Option<UiPrefs> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

pub fn save(path: &Path, prefs: UiPrefs) {
    let Some(dir) = path.parent() else { return };
    let _ = std::fs::create_dir_all(dir);
    let Ok(body) = serde_json::to_string(&prefs) else { return };
    // Atomic like every other write into ~/.clawdometer: save() fires on
    // every window-move event, so a plain fs::write racing a crash could
    // leave a torn ui.json.
    let _ = tempfile::NamedTempFile::new_in(dir).and_then(|mut tmp| {
        use std::io::Write as _;
        tmp.write_all(body.as_bytes())?;
        tmp.persist(path).map_err(|e| e.error)?;
        Ok(())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_tolerates_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui.json");
        assert!(load(&path).is_none());
        save(&path, UiPrefs { x: -100, y: 2000 });
        assert_eq!(load(&path), Some(UiPrefs { x: -100, y: 2000 }));
        std::fs::write(&path, "garbage").unwrap();
        assert!(load(&path).is_none());
    }
}
