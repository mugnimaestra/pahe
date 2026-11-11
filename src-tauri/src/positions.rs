//! Playback positions, so an Episode opens again where the user stopped.
//!
//! The positions live in the app data directory, not in webview storage: the
//! Resolver clears browsing data when the Base URL changes, and on Linux the
//! webview context is shared with the main window.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const POSITIONS_FILE: &str = "positions.json";

/// A stop before this many seconds is a glance, not a place to come back to.
const MIN_SECONDS: f64 = 10.0;

/// A stop this close to the end means the Episode was watched.
const END_MARGIN_SECONDS: f64 = 30.0;

/// Positions by key (`<anime>:<episode>`), in whole seconds.
///
/// ponytail: one entry per Episode ever paused, never pruned, and the whole
/// map is rewritten on each change. Thousands of Episodes stay well under a
/// megabyte; prune by age if that ever grows.
pub struct Positions {
    path: PathBuf,
    current: Mutex<HashMap<String, f64>>,
}

impl Positions {
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let current = fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();

        Self {
            path,
            current: Mutex::new(current),
        }
    }

    /// Where playback of `key` stopped, when there is a place to resume.
    pub fn get(&self, key: &str) -> Option<f64> {
        self.current
            .lock()
            .expect("positions lock poisoned")
            .get(key)
            .copied()
    }

    /// Keeps `seconds` for `key` and drops the `older` keys the same Episode
    /// had before, so a migrated position cannot come back. A stop near the
    /// start or the end forgets the key, so a watched Episode opens from the
    /// start next time.
    pub fn save(
        &self,
        key: &str,
        older: &[String],
        seconds: f64,
        duration: Option<f64>,
    ) -> Result<(), String> {
        if key.trim().is_empty() {
            return Err("A playback position needs an Episode.".to_owned());
        }

        let watched = duration.is_some_and(|duration| {
            duration.is_finite() && duration - seconds < END_MARGIN_SECONDS
        });
        let keep = seconds.is_finite() && seconds >= MIN_SECONDS && !watched;

        let mut current = self.current.lock().expect("positions lock poisoned");
        let mut changed = false;
        for old in older.iter().filter(|old| old.as_str() != key) {
            changed |= current.remove(old).is_some();
        }
        changed |= if keep {
            let seconds = seconds.floor();
            current.insert(key.to_owned(), seconds) != Some(seconds)
        } else {
            current.remove(key).is_some()
        };

        if changed {
            save(&self.path, &current)
        } else {
            Ok(())
        }
    }
}

fn save(path: &Path, positions: &HashMap<String, f64>) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|error| format!("could not create {dir:?}: {error}"))?;
    }
    let json = serde_json::to_vec(positions)
        .map_err(|error| format!("could not encode playback positions: {error}"))?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, json)
        .map_err(|error| format!("could not save playback positions: {error}"))?;
    fs::rename(&temp, path).map_err(|error| format!("could not save playback positions: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn positions(dir: &TempDir) -> Positions {
        Positions::open(dir.path().join(POSITIONS_FILE))
    }

    #[test]
    fn a_saved_position_survives_a_restart() {
        let dir = TempDir::new().unwrap();
        positions(&dir)
            .save("anime:3", &[], 754.6, Some(1420.0))
            .unwrap();

        assert_eq!(positions(&dir).get("anime:3"), Some(754.0));
        assert_eq!(positions(&dir).get("anime:4"), None);
    }

    #[test]
    fn a_stop_near_the_end_forgets_the_episode() {
        let dir = TempDir::new().unwrap();
        let store = positions(&dir);
        store.save("anime:3", &[], 600.0, Some(1420.0)).unwrap();

        store.save("anime:3", &[], 1400.0, Some(1420.0)).unwrap();

        assert_eq!(store.get("anime:3"), None);
        assert_eq!(positions(&dir).get("anime:3"), None);
    }

    #[test]
    fn a_stop_near_the_start_forgets_the_episode() {
        let dir = TempDir::new().unwrap();
        let store = positions(&dir);
        store.save("anime:3", &[], 600.0, None).unwrap();

        store.save("anime:3", &[], 4.0, None).unwrap();

        assert_eq!(store.get("anime:3"), None);
    }

    #[test]
    fn an_unknown_duration_still_keeps_the_position() {
        let dir = TempDir::new().unwrap();
        let store = positions(&dir);

        store.save("anime:3", &[], 600.0, None).unwrap();
        store
            .save("anime:4", &[], 600.0, Some(f64::INFINITY))
            .unwrap();

        assert_eq!(store.get("anime:3"), Some(600.0));
        assert_eq!(store.get("anime:4"), Some(600.0));
    }

    #[test]
    fn a_damaged_file_starts_empty() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(POSITIONS_FILE), "{not json").unwrap();

        assert_eq!(positions(&dir).get("anime:3"), None);
    }

    #[test]
    fn an_empty_key_is_refused() {
        let dir = TempDir::new().unwrap();

        assert!(positions(&dir).save(" ", &[], 600.0, None).is_err());
    }

    #[test]
    fn saving_under_the_current_key_drops_the_older_keys() {
        let dir = TempDir::new().unwrap();
        let store = positions(&dir);
        store.save("old-session:3", &[], 600.0, None).unwrap();

        store
            .save("#68:3", &["old-session:3".to_owned()], 700.0, None)
            .unwrap();

        assert_eq!(positions(&dir).get("#68:3"), Some(700.0));
        assert_eq!(positions(&dir).get("old-session:3"), None);
    }
}
