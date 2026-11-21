//! Local AniList-style Watchlist and Netflix-style Continue Watching store.
//!
//! Persists atomically in `app_data_dir/watchlist.json`, keyed by animepahe's
//! stable numeric id (`#<id>`) with a title-slug fallback (`slug:<slug>`).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub const WATCHLIST_FILE: &str = "watchlist.json";
pub const EVENT: &str = "watchlist://updated";

/// Matches `positions.rs`: stops within this margin of the end mark the Episode watched.
const END_MARGIN_SECONDS: f64 = 30.0;
const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WatchStatus {
    Watching,
    PlanToWatch,
    Completed,
    OnHold,
    Dropped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WatchSourceKind {
    Stream,
    Library,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportMode {
    Merge,
    Replace,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastWatched {
    pub episode: u32,
    pub session: Option<String>,
    pub quality: Option<String>,
    pub position_seconds: f64,
    pub duration_seconds: Option<f64>,
    pub source_kind: WatchSourceKind,
    pub updated_at: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchlistEntry {
    pub key: String,
    pub stable_id: Option<u64>,
    pub anime_id: Option<String>,
    pub title: String,
    pub poster: Option<String>,
    pub status: WatchStatus,
    pub progress: u32,
    pub total_episodes: Option<u32>,
    pub score: Option<u8>,
    pub notes: String,
    pub last_watched: Option<LastWatched>,
    pub updated_at: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertWatchlistEntry {
    pub stable_id: Option<u64>,
    pub anime_id: Option<String>,
    pub title: String,
    pub poster: Option<String>,
    pub status: WatchStatus,
    pub progress: Option<u32>,
    pub total_episodes: Option<u32>,
    pub score: Option<u8>,
    pub notes: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordWatchProgress {
    pub stable_id: Option<u64>,
    pub anime_id: Option<String>,
    pub title: String,
    pub poster: Option<String>,
    pub episode: u32,
    pub session: Option<String>,
    pub quality: Option<String>,
    pub position_seconds: f64,
    pub duration_seconds: Option<f64>,
    pub source_kind: WatchSourceKind,
    pub total_episodes: Option<u32>,
    pub next_episode: Option<u32>,
    pub next_session: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOutcome {
    pub imported: usize,
    pub total: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedWatchlist {
    version: u32,
    entries: Vec<WatchlistEntry>,
}

pub struct Watchlist {
    path: PathBuf,
    entries: Mutex<HashMap<String, WatchlistEntry>>,
}

impl Watchlist {
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let entries = fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<PersistedWatchlist>(&raw).ok())
            .map(|persisted| {
                persisted
                    .entries
                    .into_iter()
                    .map(|entry| (entry.key.clone(), entry))
                    .collect()
            })
            .unwrap_or_default();

        Self {
            path,
            entries: Mutex::new(entries),
        }
    }

    pub fn list(&self) -> Vec<WatchlistEntry> {
        let entries = self.entries.lock().expect("watchlist lock poisoned");
        sorted_entries(&entries)
    }

    pub fn get(
        &self,
        stable_id: Option<u64>,
        anime_id: Option<&str>,
        title: &str,
    ) -> Option<WatchlistEntry> {
        let entries = self.entries.lock().expect("watchlist lock poisoned");
        find_key(&entries, stable_id, anime_id, title).and_then(|key| entries.get(&key).cloned())
    }

    pub fn upsert(&self, input: UpsertWatchlistEntry) -> Result<WatchlistEntry, String> {
        let title = input.title.trim().to_owned();
        if title.is_empty() {
            return Err("A Watchlist entry needs a title.".to_owned());
        }
        if let Some(score) = input.score
            && !(1..=10).contains(&score)
        {
            return Err("Score must be between 1 and 10.".to_owned());
        }

        let now = now_millis();
        let mut entries = self.entries.lock().expect("watchlist lock poisoned");
        let target_key = canonical_key(input.stable_id, input.anime_id.as_deref(), &title);
        let existing = take_existing(
            &mut entries,
            input.stable_id,
            input.anime_id.as_deref(),
            &title,
        );

        let total_episodes = input
            .total_episodes
            .or_else(|| existing.as_ref().and_then(|e| e.total_episodes));
        let mut progress = input
            .progress
            .unwrap_or_else(|| existing.as_ref().map_or(0, |e| e.progress));
        if input.status == WatchStatus::Completed
            && let Some(total) = total_episodes
            && total > 0
        {
            progress = progress.max(total);
        }

        let entry = WatchlistEntry {
            key: target_key.clone(),
            stable_id: input
                .stable_id
                .or_else(|| existing.as_ref().and_then(|e| e.stable_id)),
            anime_id: non_empty(input.anime_id)
                .or_else(|| existing.as_ref().and_then(|e| e.anime_id.clone())),
            title,
            poster: non_empty(input.poster)
                .or_else(|| existing.as_ref().and_then(|e| e.poster.clone())),
            status: input.status,
            progress,
            total_episodes,
            score: input.score,
            notes: input.notes.unwrap_or_else(|| {
                existing
                    .as_ref()
                    .map_or_else(String::new, |e| e.notes.clone())
            }),
            last_watched: existing.and_then(|e| e.last_watched),
            updated_at: now,
        };

        entries.insert(target_key, entry.clone());
        save(&self.path, &entries)?;
        Ok(entry)
    }

    pub fn remove(&self, key: &str) -> Result<bool, String> {
        let mut entries = self.entries.lock().expect("watchlist lock poisoned");
        let removed = entries.remove(key).is_some();
        if removed {
            save(&self.path, &entries)?;
        }
        Ok(removed)
    }

    /// Records live playback progress for an Episode (streamed or local Library).
    ///
    /// Auto-adds the Anime as `Watching` if absent, promotes `PlanToWatch` to
    /// `Watching`, and when the Episode finishes (within `END_MARGIN_SECONDS`
    /// of the end) advances `progress`, marks `Completed` if at `total_episodes`,
    /// or queues up the next Episode (`0:00`) for Continue Watching.
    pub fn record_progress(&self, input: RecordWatchProgress) -> Result<WatchlistEntry, String> {
        let title = input.title.trim().to_owned();
        if title.is_empty() {
            return Err("Watch progress needs an Anime title.".to_owned());
        }

        let now = now_millis();
        let mut entries = self.entries.lock().expect("watchlist lock poisoned");
        let target_key = canonical_key(input.stable_id, input.anime_id.as_deref(), &title);
        let existing = take_existing(
            &mut entries,
            input.stable_id,
            input.anime_id.as_deref(),
            &title,
        );

        let duration = input.duration_seconds.filter(|d| d.is_finite() && *d > 0.0);
        let position = if input.position_seconds.is_finite() && input.position_seconds >= 0.0 {
            input.position_seconds.floor()
        } else {
            0.0
        };
        let finished = duration.is_some_and(|d| d - position < END_MARGIN_SECONDS);

        let total_episodes = input
            .total_episodes
            .or_else(|| existing.as_ref().and_then(|e| e.total_episodes));

        let mut progress = existing.as_ref().map_or(0, |e| e.progress);
        if finished {
            progress = progress.max(input.episode);
        }

        let mut status = existing
            .as_ref()
            .map_or(WatchStatus::Watching, |e| e.status);
        if status == WatchStatus::PlanToWatch {
            status = WatchStatus::Watching;
        }

        let completed_series =
            finished && total_episodes.is_some_and(|total| total > 0 && progress >= total);
        if completed_series {
            status = WatchStatus::Completed;
        }

        let last_watched = if completed_series {
            None
        } else if finished {
            let next_ep = input.next_episode.or_else(|| {
                let candidate = input.episode.saturating_add(1);
                match total_episodes {
                    Some(total) if total > 0 && candidate > total => None,
                    _ => Some(candidate),
                }
            });
            next_ep.map(|ep| LastWatched {
                episode: ep,
                session: non_empty(input.next_session.clone()),
                quality: non_empty(input.quality.clone()),
                position_seconds: 0.0,
                duration_seconds: None,
                source_kind: input.source_kind,
                updated_at: now,
            })
        } else {
            Some(LastWatched {
                episode: input.episode,
                session: non_empty(input.session),
                quality: non_empty(input.quality),
                position_seconds: position,
                duration_seconds: duration,
                source_kind: input.source_kind,
                updated_at: now,
            })
        };

        let entry = WatchlistEntry {
            key: target_key.clone(),
            stable_id: input
                .stable_id
                .or_else(|| existing.as_ref().and_then(|e| e.stable_id)),
            anime_id: non_empty(input.anime_id)
                .or_else(|| existing.as_ref().and_then(|e| e.anime_id.clone())),
            title,
            poster: non_empty(input.poster)
                .or_else(|| existing.as_ref().and_then(|e| e.poster.clone())),
            status,
            progress,
            total_episodes,
            score: existing.as_ref().and_then(|e| e.score),
            notes: existing.map_or_else(String::new, |e| e.notes),
            last_watched,
            updated_at: now,
        };

        entries.insert(target_key, entry.clone());
        save(&self.path, &entries)?;
        Ok(entry)
    }

    pub fn export_json(&self) -> Result<String, String> {
        let entries = self.entries.lock().expect("watchlist lock poisoned");
        let persisted = PersistedWatchlist {
            version: SCHEMA_VERSION,
            entries: sorted_entries(&entries),
        };
        serde_json::to_string_pretty(&persisted)
            .map_err(|error| format!("could not encode watchlist backup: {error}"))
    }

    pub fn import_json(&self, raw: &str, mode: ImportMode) -> Result<ImportOutcome, String> {
        let parsed: PersistedWatchlist = serde_json::from_str(raw)
            .map_err(|error| format!("could not read watchlist backup JSON: {error}"))?;
        if parsed.version != SCHEMA_VERSION {
            return Err(format!(
                "Unsupported watchlist backup version {} (expected {}).",
                parsed.version, SCHEMA_VERSION
            ));
        }
        for entry in &parsed.entries {
            if entry.title.trim().is_empty() {
                return Err("Backup contains an entry without a title.".to_owned());
            }
            if let Some(score) = entry.score
                && !(1..=10).contains(&score)
            {
                return Err(format!(
                    "Backup entry '{}' has invalid score {score} (expected 1–10).",
                    entry.title
                ));
            }
        }

        let imported = parsed.entries.len();
        let mut entries = self.entries.lock().expect("watchlist lock poisoned");
        if mode == ImportMode::Replace {
            entries.clear();
        }

        for mut incoming in parsed.entries {
            let key = canonical_key(
                incoming.stable_id,
                incoming.anime_id.as_deref(),
                &incoming.title,
            );
            incoming.key = key.clone();
            match entries.get(&key) {
                Some(current)
                    if mode == ImportMode::Merge && current.updated_at > incoming.updated_at =>
                {
                    // Keep the newer local entry during merge.
                }
                _ => {
                    entries.insert(key, incoming);
                }
            }
        }

        let total = entries.len();
        save(&self.path, &entries)?;
        Ok(ImportOutcome { imported, total })
    }
}

fn canonical_key(stable_id: Option<u64>, anime_id: Option<&str>, title: &str) -> String {
    if let Some(id) = stable_id {
        return format!("#{id}");
    }
    let slug = title_slug(title);
    if !slug.is_empty() {
        return format!("slug:{slug}");
    }
    anime_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| format!("session:{id}"))
        .unwrap_or_else(|| "slug:unknown".to_owned())
}

fn find_key(
    entries: &HashMap<String, WatchlistEntry>,
    stable_id: Option<u64>,
    anime_id: Option<&str>,
    title: &str,
) -> Option<String> {
    if let Some(id) = stable_id {
        let stable_key = format!("#{id}");
        if entries.contains_key(&stable_key) {
            return Some(stable_key);
        }
    }
    let slug = title_slug(title);
    if !slug.is_empty() {
        let slug_key = format!("slug:{slug}");
        if entries.contains_key(&slug_key) {
            return Some(slug_key);
        }
    }
    if let Some(aid) = anime_id.map(str::trim).filter(|s| !s.is_empty()) {
        let session_key = format!("session:{aid}");
        if entries.contains_key(&session_key) {
            return Some(session_key);
        }
        if let Some((k, _)) = entries
            .iter()
            .find(|(_, entry)| entry.anime_id.as_deref() == Some(aid))
        {
            return Some(k.clone());
        }
    }
    if !slug.is_empty()
        && let Some((k, _)) = entries
            .iter()
            .find(|(_, entry)| title_slug(&entry.title) == slug)
    {
        return Some(k.clone());
    }
    None
}

fn take_existing(
    entries: &mut HashMap<String, WatchlistEntry>,
    stable_id: Option<u64>,
    anime_id: Option<&str>,
    title: &str,
) -> Option<WatchlistEntry> {
    let key = find_key(entries, stable_id, anime_id, title)?;
    entries.remove(&key)
}

fn title_slug(title: &str) -> String {
    let mut out = String::new();
    let mut sep = false;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            if sep && !out.is_empty() {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
            sep = false;
        } else {
            sep = true;
        }
    }
    out
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

fn sorted_entries(entries: &HashMap<String, WatchlistEntry>) -> Vec<WatchlistEntry> {
    let mut list: Vec<WatchlistEntry> = entries.values().cloned().collect();
    list.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
    list
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn save(path: &Path, entries: &HashMap<String, WatchlistEntry>) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|error| format!("could not create {dir:?}: {error}"))?;
    }
    let persisted = PersistedWatchlist {
        version: SCHEMA_VERSION,
        entries: sorted_entries(entries),
    };
    let json = serde_json::to_vec_pretty(&persisted)
        .map_err(|error| format!("could not encode watchlist: {error}"))?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, json).map_err(|error| format!("could not save watchlist: {error}"))?;
    fs::rename(&temp, path).map_err(|error| format!("could not save watchlist: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn store(dir: &TempDir) -> Watchlist {
        Watchlist::open(dir.path().join(WATCHLIST_FILE))
    }

    #[test]
    fn upsert_and_remove_survive_a_restart() {
        let dir = TempDir::new().unwrap();
        let wl = store(&dir);

        let entry = wl
            .upsert(UpsertWatchlistEntry {
                stable_id: Some(42),
                anime_id: Some("sess-1".to_owned()),
                title: "Frieren".to_owned(),
                poster: Some("https://img.example/frieren.jpg".to_owned()),
                status: WatchStatus::PlanToWatch,
                progress: Some(0),
                total_episodes: Some(28),
                score: Some(10),
                notes: Some("Must watch".to_owned()),
            })
            .unwrap();

        assert_eq!(entry.key, "#42");
        assert_eq!(store(&dir).list().len(), 1);
        assert_eq!(
            store(&dir).get(Some(42), None, "Frieren").unwrap().score,
            Some(10)
        );

        assert!(wl.remove("#42").unwrap());
        assert!(store(&dir).list().is_empty());
    }

    #[test]
    fn record_progress_auto_adds_watching_and_tracks_last_stream_position() {
        let dir = TempDir::new().unwrap();
        let wl = store(&dir);

        let entry = wl
            .record_progress(RecordWatchProgress {
                stable_id: Some(100),
                anime_id: Some("abc".to_owned()),
                title: "Dandadan".to_owned(),
                poster: None,
                episode: 3,
                session: Some("ep3-sess".to_owned()),
                quality: Some("1080p".to_owned()),
                position_seconds: 612.8,
                duration_seconds: Some(1440.0),
                source_kind: WatchSourceKind::Stream,
                total_episodes: Some(12),
                next_episode: Some(4),
                next_session: Some("ep4-sess".to_owned()),
            })
            .unwrap();

        assert_eq!(entry.status, WatchStatus::Watching);
        assert_eq!(entry.progress, 0);
        let lw = entry.last_watched.unwrap();
        assert_eq!(lw.episode, 3);
        assert_eq!(lw.position_seconds, 612.0);
        assert_eq!(lw.source_kind, WatchSourceKind::Stream);
    }

    #[test]
    fn finishing_an_episode_advances_progress_and_queues_next_up() {
        let dir = TempDir::new().unwrap();
        let wl = store(&dir);

        let entry = wl
            .record_progress(RecordWatchProgress {
                stable_id: Some(100),
                anime_id: Some("abc".to_owned()),
                title: "Dandadan".to_owned(),
                poster: None,
                episode: 3,
                session: Some("ep3-sess".to_owned()),
                quality: Some("1080p".to_owned()),
                position_seconds: 1420.0,
                duration_seconds: Some(1440.0),
                source_kind: WatchSourceKind::Stream,
                total_episodes: Some(12),
                next_episode: Some(4),
                next_session: Some("ep4-sess".to_owned()),
            })
            .unwrap();

        assert_eq!(entry.status, WatchStatus::Watching);
        assert_eq!(entry.progress, 3);
        let lw = entry.last_watched.unwrap();
        assert_eq!(lw.episode, 4);
        assert_eq!(lw.position_seconds, 0.0);
        assert_eq!(lw.session.as_deref(), Some("ep4-sess"));
    }

    #[test]
    fn finishing_the_final_episode_marks_completed_and_clears_continue_watching() {
        let dir = TempDir::new().unwrap();
        let wl = store(&dir);

        let entry = wl
            .record_progress(RecordWatchProgress {
                stable_id: Some(100),
                anime_id: Some("abc".to_owned()),
                title: "Dandadan".to_owned(),
                poster: None,
                episode: 12,
                session: Some("ep12-sess".to_owned()),
                quality: Some("1080p".to_owned()),
                position_seconds: 1430.0,
                duration_seconds: Some(1440.0),
                source_kind: WatchSourceKind::Library,
                total_episodes: Some(12),
                next_episode: None,
                next_session: None,
            })
            .unwrap();

        assert_eq!(entry.status, WatchStatus::Completed);
        assert_eq!(entry.progress, 12);
        assert!(entry.last_watched.is_none());
    }

    #[test]
    fn migrates_slug_key_to_stable_id_without_duplicating() {
        let dir = TempDir::new().unwrap();
        let wl = store(&dir);

        wl.upsert(UpsertWatchlistEntry {
            stable_id: None,
            anime_id: None,
            title: "Monster".to_owned(),
            poster: None,
            status: WatchStatus::PlanToWatch,
            progress: Some(0),
            total_episodes: Some(74),
            score: None,
            notes: Some("Classic".to_owned()),
        })
        .unwrap();

        let migrated = wl
            .record_progress(RecordWatchProgress {
                stable_id: Some(19),
                anime_id: Some("monster-sess".to_owned()),
                title: "Monster".to_owned(),
                poster: None,
                episode: 1,
                session: Some("ep1".to_owned()),
                quality: Some("720p".to_owned()),
                position_seconds: 120.0,
                duration_seconds: Some(1440.0),
                source_kind: WatchSourceKind::Stream,
                total_episodes: Some(74),
                next_episode: Some(2),
                next_session: None,
            })
            .unwrap();

        assert_eq!(migrated.key, "#19");
        assert_eq!(migrated.notes, "Classic");
        assert_eq!(migrated.status, WatchStatus::Watching);
        assert_eq!(wl.list().len(), 1);
    }

    #[test]
    fn export_and_import_support_merge_and_replace() {
        let dir_a = TempDir::new().unwrap();
        let dir_b = TempDir::new().unwrap();
        let a = store(&dir_a);
        let b = store(&dir_b);

        a.upsert(UpsertWatchlistEntry {
            stable_id: Some(1),
            anime_id: None,
            title: "Cowboy Bebop".to_owned(),
            poster: None,
            status: WatchStatus::Completed,
            progress: Some(26),
            total_episodes: Some(26),
            score: Some(10),
            notes: None,
        })
        .unwrap();

        b.upsert(UpsertWatchlistEntry {
            stable_id: Some(2),
            anime_id: None,
            title: "Samurai Champloo".to_owned(),
            poster: None,
            status: WatchStatus::Watching,
            progress: Some(5),
            total_episodes: Some(26),
            score: Some(9),
            notes: None,
        })
        .unwrap();

        let json = a.export_json().unwrap();
        let merged = b.import_json(&json, ImportMode::Merge).unwrap();
        assert_eq!(merged.imported, 1);
        assert_eq!(merged.total, 2);

        let replaced = b.import_json(&json, ImportMode::Replace).unwrap();
        assert_eq!(replaced.imported, 1);
        assert_eq!(replaced.total, 1);
    }

    #[test]
    fn damaged_file_starts_empty_and_invalid_score_is_rejected() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(WATCHLIST_FILE), "{broken").unwrap();
        let wl = store(&dir);
        assert!(wl.list().is_empty());

        let err = wl.upsert(UpsertWatchlistEntry {
            stable_id: Some(1),
            anime_id: None,
            title: "Bad Score".to_owned(),
            poster: None,
            status: WatchStatus::Watching,
            progress: None,
            total_episodes: None,
            score: Some(11),
            notes: None,
        });
        assert!(err.is_err());
    }
}
