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
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(body) = serde_json::to_string(&prefs) {
        let _ = std::fs::write(path, body);
    }
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
