use std::path::Path;

use serde::{Deserialize, Serialize};

// Not Copy: `rice` is a String. Callers pass &UiPrefs to save().
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiPrefs {
    pub x: i32,
    pub y: i32,
    // Defaults keep ui.json files from pre-opacity builds loading.
    #[serde(default = "default_opacity")]
    pub opacity: f64,
    #[serde(default)]
    pub compact: bool,
    // Selected "rice" skin profile: "classic" (the default card), "bento"
    // (2×2 cell grid, card-sized), "audiowave_orb" (ring, bars only), or
    // "audiowave_orb_peak" (ring with peak-hold caps). serde default keeps
    // pre-rice ui.json files loading.
    #[serde(default = "default_rice")]
    pub rice: String,
}

fn default_opacity() -> f64 {
    1.0
}

pub fn default_rice() -> String {
    "classic".to_string()
}

pub fn load(path: &Path) -> Option<UiPrefs> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

pub fn save(path: &Path, prefs: &UiPrefs) {
    let Some(dir) = path.parent() else { return };
    let _ = std::fs::create_dir_all(dir);
    let Ok(body) = serde_json::to_string(prefs) else { return };
    // Atomic like every other write into ~/.clawdometer: save() fires on
    // every window-move event, so a plain fs::write racing a crash could
    // leave a torn ui.json.
    let res = tempfile::NamedTempFile::new_in(dir).and_then(|mut tmp| {
        use std::io::Write as _;
        tmp.write_all(body.as_bytes())?;
        tmp.persist(path).map_err(|e| e.error)?;
        Ok(())
    });
    if let Err(e) = res {
        // Disk-full / permissions: prefs silently revert on next launch
        // otherwise. Leave a breadcrumb on the hidden console.
        eprintln!("clawdometer: ui prefs save failed: {e}");
    }
}

/// Coalesces the continuous stream of window-move events (one per pixel of
/// drag on Windows) into a single save once the window stops moving.
pub struct MoveDebouncer {
    pending: Option<(i32, i32)>,
    last_move: Option<std::time::Instant>,
}

impl MoveDebouncer {
    pub const fn new() -> Self {
        Self { pending: None, last_move: None }
    }

    pub fn record(&mut self, x: i32, y: i32, now: std::time::Instant) {
        self.pending = Some((x, y));
        self.last_move = Some(now);
    }

    /// The latest position, once no move has arrived for `settle`.
    /// None while the drag is still in motion or nothing is pending.
    pub fn take_if_settled(
        &mut self,
        now: std::time::Instant,
        settle: std::time::Duration,
    ) -> Option<(i32, i32)> {
        if now.duration_since(self.last_move?) < settle {
            return None;
        }
        self.last_move = None;
        self.pending.take()
    }

    /// Flush whatever is pending regardless of settle time (quit path).
    pub fn take_now(&mut self) -> Option<(i32, i32)> {
        self.last_move = None;
        self.pending.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn debouncer_holds_while_moving_and_releases_after_settle() {
        let mut d = MoveDebouncer::new();
        let t0 = Instant::now();
        let settle = Duration::from_millis(500);
        assert_eq!(d.take_if_settled(t0, settle), None); // nothing pending
        d.record(10, 20, t0);
        assert_eq!(d.take_if_settled(t0, settle), None); // still moving
        d.record(30, 40, t0 + Duration::from_millis(100)); // newer position wins
        assert_eq!(d.take_if_settled(t0 + Duration::from_millis(400), settle), None);
        assert_eq!(
            d.take_if_settled(t0 + Duration::from_millis(700), settle),
            Some((30, 40))
        );
        assert_eq!(d.take_if_settled(t0 + Duration::from_secs(2), settle), None); // consumed
    }

    #[test]
    fn debouncer_take_now_flushes_immediately() {
        let mut d = MoveDebouncer::new();
        assert_eq!(d.take_now(), None);
        d.record(1, 2, Instant::now());
        assert_eq!(d.take_now(), Some((1, 2)));
        assert_eq!(d.take_now(), None);
    }

    #[test]
    fn round_trips_and_tolerates_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui.json");
        assert!(load(&path).is_none());
        let prefs = UiPrefs { x: -100, y: 2000, opacity: 0.7, compact: true, rice: "audiowave_orb".into() };
        save(&path, &prefs);
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
        assert_eq!(p.rice, "classic");
    }
}
