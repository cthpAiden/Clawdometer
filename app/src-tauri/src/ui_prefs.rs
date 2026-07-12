use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UiPrefs {
    pub x: i32,
    pub y: i32,
    // Defaults keep ui.json files from pre-opacity builds loading.
    #[serde(default = "default_opacity")]
    pub opacity: f64,
    #[serde(default)]
    pub compact: bool,
}

fn default_opacity() -> f64 {
    1.0
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
        let prefs = UiPrefs { x: -100, y: 2000, opacity: 0.7, compact: true };
        save(&path, prefs);
        assert_eq!(load(&path), Some(prefs));
        std::fs::write(&path, "garbage").unwrap();
        assert!(load(&path).is_none());
    }

    #[test]
    fn old_position_only_file_loads_with_defaults() {
        // ui.json written by a pre-opacity build must still parse: opaque,
        // full size.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui.json");
        std::fs::write(&path, r#"{"x": 40, "y": 60}"#).unwrap();
        let p = load(&path).unwrap();
        assert_eq!((p.x, p.y), (40, 60));
        assert_eq!(p.opacity, 1.0);
        assert!(!p.compact);
    }
}
