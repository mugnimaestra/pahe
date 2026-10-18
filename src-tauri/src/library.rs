use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::resolver::SearchResult;

pub const INDEX_FILE: &str = "library.json";

const PREFIX: &str = "AnimePahe_";
const SEPARATOR: &str = "_-_";
const EXTENSION: &str = ".mp4";

/// A CLI-compatible Episode filename, before it becomes a Library item.
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedName {
    pub title_slug: String,
    pub episode: f64,
    pub version: u32,
    pub quality: String,
    pub group: String,
}

/// The Library's identity for an Anime: the animepahe id once enrichment has
/// found it, otherwise the title slug the filename carries.
pub fn item_key(anime: &str, episode: f64, quality: &str) -> String {
    format!("{anime}\u{1f}{}\u{1f}{quality}", episode_key(episode))
}

/// A downloaded Episode that pahe knows about.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LibraryItem {
    pub anime_id: Option<String>,
    pub title_slug: String,
    pub poster: Option<String>,
    pub episode: f64,
    pub version: u32,
    pub quality: String,
    pub group: String,
    pub path: String,
}

impl LibraryItem {
    fn identity(&self) -> &str {
        self.anime_id.as_deref().unwrap_or(&self.title_slug)
    }

    fn key(&self) -> String {
        item_key(self.identity(), self.episode, &self.quality)
    }
}

impl Default for LibraryItem {
    fn default() -> Self {
        Self {
            anime_id: None,
            title_slug: String::new(),
            poster: None,
            episode: 0.0,
            version: 1,
            quality: String::new(),
            group: String::new(),
            path: String::new(),
        }
    }
}

/// One Anime's row in the Library view.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryAnime {
    pub anime_id: Option<String>,
    pub title: String,
    pub poster: Option<String>,
    pub episode_count: usize,
}

/// The state of the Library folder, scanned at startup and on demand.
pub struct Library {
    index_path: PathBuf,
    items: Mutex<Vec<LibraryItem>>,
    enriching: tokio::sync::Mutex<()>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct LibraryIndex {
    items: Vec<LibraryItem>,
}

impl Library {
    /// Loads the metadata index, or starts empty when it is missing or corrupt.
    pub fn open(index_path: impl Into<PathBuf>) -> Self {
        let index_path = index_path.into();
        let items = fs::read_to_string(&index_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<LibraryIndex>(&raw).ok())
            .map(|index| index.items)
            .unwrap_or_default();

        Self {
            index_path,
            items: Mutex::new(dedupe(items)),
            enriching: tokio::sync::Mutex::new(()),
        }
    }

    /// The Library grouped by Anime.
    pub fn groups(&self) -> Vec<LibraryAnime> {
        groups_of(&self.items.lock().expect("library lock poisoned"))
    }

    /// Adopts matching files, drops entries whose files are gone, and writes
    /// the index. A missing or unreadable folder is an explicit error.
    pub fn refresh(&self, folder: &Path) -> Result<Vec<LibraryAnime>, String> {
        let scanned = scan(folder)?;
        let mut items = self.items.lock().expect("library lock poisoned");
        let reconciled = reconcile(scanned, &items);

        save(&self.index_path, &reconciled)?;
        *items = reconciled;

        Ok(groups_of(&items))
    }

    /// Looks up the Anime behind each unenriched title slug and caches its
    /// identity and poster URL in the index. A failed or empty search leaves
    /// the item in place for the placeholder to cover.
    pub async fn enrich<F, Fut>(&self, search: F) -> Result<Vec<LibraryAnime>, String>
    where
        F: Fn(String) -> Fut,
        Fut: Future<Output = Result<Vec<SearchResult>, String>>,
    {
        let _guard = self.enriching.lock().await;

        let pending: Vec<String> = {
            let items = self.items.lock().expect("library lock poisoned");
            items
                .iter()
                .filter(|item| item.anime_id.is_none())
                .map(|item| item.title_slug.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        };

        let mut matches = Vec::new();
        for title_slug in pending {
            let title = humanize(&title_slug);
            if let Ok(results) = search(title.clone()).await
                && let Some(found) = best_match(&title, &results)
            {
                matches.push((title_slug, found.clone()));
            }
        }

        {
            let mut items = self.items.lock().expect("library lock poisoned");
            for item in items.iter_mut() {
                if item.anime_id.is_some() {
                    continue;
                }
                let Some((_, found)) = matches.iter().find(|(slug, _)| *slug == item.title_slug)
                else {
                    continue;
                };
                item.anime_id = Some(found.anime_id.clone());
                item.poster = found.poster.clone();
            }

            let enriched = dedupe(std::mem::take(&mut *items));
            save(&self.index_path, &enriched)?;
            *items = enriched;
        }

        Ok(self.groups())
    }
}

/// Parses `AnimePahe_<title_slug>_-_<NN>_<quality>_<group>.mp4`, where the
/// Episode may carry a version (`01v2`) or a decimal (`12.5`). Partial files
/// and anything else are ignored.
pub fn parse_filename(name: &str) -> Option<ParsedName> {
    if !name.to_ascii_lowercase().ends_with(EXTENSION) {
        return None;
    }
    let stem = &name[..name.len() - EXTENSION.len()];
    let rest = stem.strip_prefix(PREFIX)?;
    let (title_slug, tail) = rest.rsplit_once(SEPARATOR)?;
    if title_slug.is_empty() {
        return None;
    }

    let mut tokens = tail.split('_');
    let episode = tokens.next()?;
    let quality = tokens.next()?;
    let group = tokens.next()?;
    if tokens.next().is_some() || quality.is_empty() || group.is_empty() {
        return None;
    }

    let (episode, version) = parse_episode(episode)?;

    Some(ParsedName {
        title_slug: title_slug.to_owned(),
        episode,
        version,
        quality: quality.to_owned(),
        group: group.to_owned(),
    })
}

fn parse_episode(token: &str) -> Option<(f64, u32)> {
    let (number, version) = match token.rsplit_once('v') {
        Some((number, version)) => (number, version.parse::<u32>().ok()?),
        None => (token, 1),
    };
    if version == 0 {
        return None;
    }
    let episode = number.parse::<f64>().ok()?;
    if !episode.is_finite() || episode <= 0.0 {
        return None;
    }

    Some((episode, version))
}

/// `Seitokai_ni_mo_Ana_wa_Aru` -> `Seitokai ni mo Ana wa Aru`.
pub fn humanize(title_slug: &str) -> String {
    title_slug
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn episode_key(episode: f64) -> String {
    if episode.fract() == 0.0 {
        format!("{}", episode as u64)
    } else {
        format!("{episode}")
    }
}

/// Reads the folder and keeps only files the CLI naming pattern recognises.
fn scan(folder: &Path) -> Result<Vec<LibraryItem>, String> {
    let entries = fs::read_dir(folder).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => format!(
            "The Library folder {} does not exist. Create it or choose another folder in Settings.",
            folder.display()
        ),
        _ => format!(
            "The Library folder {} cannot be read: {error}",
            folder.display()
        ),
    })?;

    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "The Library folder {} cannot be read: {error}",
                folder.display()
            )
        })?;
        if !matches!(entry.file_type(), Ok(file_type) if file_type.is_file()) {
            continue;
        }

        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(parsed) = parse_filename(name) else {
            continue;
        };

        found.push((path, parsed));
    }
    found.sort_by(|left, right| left.0.cmp(&right.0));

    Ok(found
        .into_iter()
        .map(|(path, parsed)| LibraryItem {
            anime_id: None,
            title_slug: parsed.title_slug,
            poster: None,
            episode: parsed.episode,
            version: parsed.version,
            quality: parsed.quality,
            group: parsed.group,
            path: path.display().to_string(),
        })
        .collect())
}

/// Reconciles a fresh scan against the index: an entry whose path survived
/// keeps its enrichment, and a new file of a known Anime inherits it by slug.
fn reconcile(scanned: Vec<LibraryItem>, existing: &[LibraryItem]) -> Vec<LibraryItem> {
    let scanned = scanned
        .into_iter()
        .map(|mut item| {
            let previous = existing
                .iter()
                .find(|old| old.path == item.path)
                .or_else(|| {
                    existing
                        .iter()
                        .find(|old| old.anime_id.is_some() && old.title_slug == item.title_slug)
                });

            if let Some(previous) = previous {
                item.anime_id = previous.anime_id.clone();
                item.poster = previous.poster.clone();
            }
            item
        })
        .collect();

    dedupe(scanned)
}

/// One Library item per Anime identity, Episode, and Quality. A higher version
/// wins; a tie keeps the first entry, which keeps repeated scans deterministic.
fn dedupe(items: Vec<LibraryItem>) -> Vec<LibraryItem> {
    let mut unique: BTreeMap<String, LibraryItem> = BTreeMap::new();

    for item in items {
        match unique.get(&item.key()) {
            Some(kept) if kept.version >= item.version => {}
            _ => {
                unique.insert(item.key(), item);
            }
        }
    }

    unique.into_values().collect()
}

fn groups_of(items: &[LibraryItem]) -> Vec<LibraryAnime> {
    struct Group {
        anime_id: Option<String>,
        title: String,
        poster: Option<String>,
        episodes: BTreeSet<String>,
    }

    let mut groups: BTreeMap<String, Group> = BTreeMap::new();
    for item in items {
        let group = groups
            .entry(item.identity().to_owned())
            .or_insert_with(|| Group {
                anime_id: item.anime_id.clone(),
                title: humanize(&item.title_slug),
                poster: None,
                episodes: BTreeSet::new(),
            });

        group.episodes.insert(episode_key(item.episode));
        if item.anime_id.is_some() {
            group.anime_id = item.anime_id.clone();
        }
        if group.poster.is_none() {
            group.poster = item.poster.clone();
        }
    }

    let mut groups: Vec<LibraryAnime> = groups
        .into_values()
        .map(|group| LibraryAnime {
            anime_id: group.anime_id,
            title: group.title,
            poster: group.poster,
            episode_count: group.episodes.len(),
        })
        .collect();
    groups.sort_by(|left, right| {
        left.title
            .to_lowercase()
            .cmp(&right.title.to_lowercase())
            .then_with(|| left.title.cmp(&right.title))
    });

    groups
}

fn normalize(title: &str) -> String {
    title
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

/// The search result that names the same Anime as the title slug: an exact
/// title first, then a containment match. animepahe localises result titles, so
/// a romaji slug can miss both; its own relevance order then puts the Anime the
/// filename came from first, which is the best match available.
fn best_match<'a>(wanted: &str, results: &'a [SearchResult]) -> Option<&'a SearchResult> {
    let wanted = normalize(wanted);
    if wanted.is_empty() {
        return results.first();
    }

    results
        .iter()
        .find(|result| normalize(&result.title) == wanted)
        .or_else(|| {
            results.iter().find(|result| {
                let candidate = normalize(&result.title);
                !candidate.is_empty()
                    && (candidate.contains(&wanted) || wanted.contains(&candidate))
            })
        })
        .or_else(|| results.first())
}

fn save(path: &Path, items: &[LibraryItem]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .map_err(|error| format!("could not create the Library index folder: {error}"))?;
    }
    let json = serde_json::to_vec_pretty(&LibraryIndex {
        items: items.to_vec(),
    })
    .map_err(|error| format!("could not encode the Library index: {error}"))?;

    let temp = path.with_extension("json.tmp");
    fs::write(&temp, json).map_err(|error| format!("could not save the Library index: {error}"))?;
    fs::rename(&temp, path)
        .map_err(|error| format!("could not save the Library index: {error}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const ONE: &str = "AnimePahe_Seitokai_ni_mo_Ana_wa_Aru_-_01v2_1080p_ADN.mp4";
    const TWO: &str = "AnimePahe_Seitokai_ni_mo_Ana_wa_Aru_-_02_1080p_ADN.mp4";
    const THREE: &str = "AnimePahe_Seitokai_ni_mo_Ana_wa_Aru_-_03_1080p_ADN.mp4";

    struct Shelf {
        _dir: TempDir,
        folder: PathBuf,
        index: PathBuf,
    }

    impl Shelf {
        fn new() -> Self {
            let dir = TempDir::new().unwrap();
            let folder = dir.path().join("animepahe");
            fs::create_dir_all(&folder).unwrap();
            let index = dir.path().join("app").join(INDEX_FILE);

            Self {
                _dir: dir,
                folder,
                index,
            }
        }

        fn touch(&self, name: &str) {
            fs::write(self.folder.join(name), b"not a real video").unwrap();
        }

        fn library(&self) -> Library {
            Library::open(&self.index)
        }

        fn stored(&self) -> LibraryIndex {
            serde_json::from_str(&fs::read_to_string(&self.index).unwrap()).unwrap()
        }
    }

    fn search_result(title: &str, poster: Option<&str>) -> SearchResult {
        SearchResult {
            anime_id: "anime-id".to_owned(),
            title: title.to_owned(),
            year: Some(2025),
            poster: poster.map(str::to_owned),
        }
    }

    #[test]
    fn parses_a_versioned_cli_filename() {
        assert_eq!(
            parse_filename(ONE).unwrap(),
            ParsedName {
                title_slug: "Seitokai_ni_mo_Ana_wa_Aru".to_owned(),
                episode: 1.0,
                version: 2,
                quality: "1080p".to_owned(),
                group: "ADN".to_owned(),
            }
        );
    }

    #[test]
    fn parses_plain_and_decimal_episode_numbers() {
        let plain = parse_filename(TWO).unwrap();
        assert_eq!((plain.episode, plain.version), (2.0, 1));

        let decimal = parse_filename("AnimePahe_One_-_12.5_720p_Group.mp4").unwrap();
        assert_eq!((decimal.episode, decimal.version), (12.5, 1));
    }

    #[test]
    fn ignores_partial_and_foreign_files() {
        for name in [
            "AnimePahe_One_-_01_1080p_ADN.mp4.part",
            "AnimePahe_One_-_01_1080p_ADN.part",
            "movie.mp4",
            "AnimePahe_One_-_01_1080p.mp4",
            "AnimePahe_One_-_01v_1080p_ADN.mp4",
            "AnimePahe_One_-_0_1080p_ADN.mp4",
            "AnimePahe_One_-_01_1080p_ADN_extra.mp4",
            "AnimePahe__-__01_1080p_ADN.mp4",
        ] {
            assert_eq!(parse_filename(name), None, "{name} should be ignored");
        }
    }

    #[test]
    fn adopts_the_fixture_files_as_one_anime_with_three_episodes() {
        let shelf = Shelf::new();
        for name in [ONE, TWO, THREE] {
            shelf.touch(name);
        }
        shelf.touch("AnimePahe_Seitokai_ni_mo_Ana_wa_Aru_-_04_1080p_ADN.mp4.part");

        let groups = shelf.library().refresh(&shelf.folder).unwrap();

        assert_eq!(
            groups,
            vec![LibraryAnime {
                anime_id: None,
                title: "Seitokai ni mo Ana wa Aru".to_owned(),
                poster: None,
                episode_count: 3,
            }]
        );
    }

    #[test]
    fn a_later_version_wins_without_double_counting_the_episode() {
        let shelf = Shelf::new();
        shelf.touch("AnimePahe_One_-_01_1080p_ADN.mp4");
        shelf.touch("AnimePahe_One_-_01v2_1080p_ADN.mp4");

        let groups = shelf.library().refresh(&shelf.folder).unwrap();

        assert_eq!(groups[0].episode_count, 1);
        let stored = shelf.stored();
        assert_eq!(stored.items.len(), 1);
        assert!(stored.items[0].path.ends_with("01v2_1080p_ADN.mp4"));
        assert_eq!(stored.items[0].version, 2);
    }

    #[test]
    fn different_qualities_of_one_episode_are_distinct_items() {
        let shelf = Shelf::new();
        shelf.touch("AnimePahe_One_-_01_1080p_ADN.mp4");
        shelf.touch("AnimePahe_One_-_01_720p_ADN.mp4");

        let groups = shelf.library().refresh(&shelf.folder).unwrap();

        assert_eq!(groups[0].episode_count, 1);
        assert_eq!(shelf.stored().items.len(), 2);
    }

    #[test]
    fn groups_items_by_anime_and_sorts_them() {
        let shelf = Shelf::new();
        shelf.touch(TWO);
        shelf.touch("AnimePahe_Another_Show_-_01_720p_Group.mp4");
        shelf.touch(THREE);

        let groups = shelf.library().refresh(&shelf.folder).unwrap();

        assert_eq!(
            groups
                .iter()
                .map(|group| (group.title.as_str(), group.episode_count))
                .collect::<Vec<_>>(),
            vec![("Another Show", 1), ("Seitokai ni mo Ana wa Aru", 2)]
        );
    }

    #[test]
    fn the_index_survives_a_restart_and_is_written_atomically() {
        let shelf = Shelf::new();
        shelf.touch(ONE);
        shelf.library().refresh(&shelf.folder).unwrap();

        let reopened = shelf.library();

        assert_eq!(reopened.groups()[0].episode_count, 1);
        assert!(shelf.index.is_file());
        assert!(!shelf.index.with_extension("json.tmp").exists());
    }

    #[test]
    fn a_deleted_file_is_dropped_from_the_index() {
        let shelf = Shelf::new();
        shelf.touch(TWO);
        shelf.touch(THREE);
        shelf.library().refresh(&shelf.folder).unwrap();

        fs::remove_file(shelf.folder.join(TWO)).unwrap();
        let groups = shelf.library().refresh(&shelf.folder).unwrap();

        assert_eq!(groups[0].episode_count, 1);
        assert_eq!(shelf.stored().items.len(), 1);
    }

    #[test]
    fn a_missing_folder_is_an_explicit_error() {
        let shelf = Shelf::new();
        fs::remove_dir(&shelf.folder).unwrap();

        let error = shelf.library().refresh(&shelf.folder).unwrap_err();

        assert!(error.contains("does not exist"), "{error}");
        assert!(error.contains("Downloads") || error.contains("animepahe"));
    }

    #[test]
    fn an_unreadable_folder_is_an_explicit_error() {
        let shelf = Shelf::new();
        fs::remove_dir(&shelf.folder).unwrap();
        fs::write(&shelf.folder, b"not a folder").unwrap();

        let error = shelf.library().refresh(&shelf.folder).unwrap_err();

        assert!(error.contains("cannot be read"), "{error}");
    }

    #[tokio::test]
    async fn enrichment_caches_the_matched_identity_and_poster() {
        let shelf = Shelf::new();
        for name in [ONE, TWO, THREE] {
            shelf.touch(name);
        }
        let library = shelf.library();
        library.refresh(&shelf.folder).unwrap();

        let groups = library
            .enrich(|query| async move {
                assert_eq!(query, "Seitokai ni mo Ana wa Aru");
                Ok(vec![search_result(
                    "Seitokai ni mo Ana wa Aru!",
                    Some("https://i.animepahe.pw/posters/one.jpg"),
                )])
            })
            .await
            .unwrap();

        assert_eq!(
            groups[0],
            LibraryAnime {
                anime_id: Some("anime-id".to_owned()),
                title: "Seitokai ni mo Ana wa Aru".to_owned(),
                poster: Some("https://i.animepahe.pw/posters/one.jpg".to_owned()),
                episode_count: 3,
            }
        );
        assert_eq!(
            shelf.library().groups()[0].poster.as_deref(),
            Some("https://i.animepahe.pw/posters/one.jpg")
        );
    }

    #[tokio::test]
    async fn a_failed_search_keeps_the_items_and_shows_the_placeholder() {
        let shelf = Shelf::new();
        shelf.touch(ONE);
        let library = shelf.library();
        library.refresh(&shelf.folder).unwrap();

        let groups = library
            .enrich(|_| async { Err("animepahe is unreachable".to_owned()) })
            .await
            .unwrap();

        assert_eq!(groups[0].title, "Seitokai ni mo Ana wa Aru");
        assert_eq!(groups[0].poster, None);
        assert_eq!(groups[0].episode_count, 1);
        assert!(library.enrich(|_| async { Ok(Vec::new()) }).await.is_ok());
    }

    #[tokio::test]
    async fn a_scan_after_enrichment_keeps_one_enriched_anime() {
        let shelf = Shelf::new();
        shelf.touch(ONE);
        let library = shelf.library();
        library.refresh(&shelf.folder).unwrap();
        library
            .enrich(|_| async {
                Ok(vec![search_result(
                    "Seitokai ni mo Ana wa Aru",
                    Some("https://i.animepahe.pw/posters/one.jpg"),
                )])
            })
            .await
            .unwrap();

        let groups = library.refresh(&shelf.folder).unwrap();

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].anime_id.as_deref(), Some("anime-id"));
        assert_eq!(
            groups[0].poster.as_deref(),
            Some("https://i.animepahe.pw/posters/one.jpg")
        );
    }

    #[tokio::test]
    async fn a_new_file_of_an_enriched_anime_inherits_its_metadata() {
        let shelf = Shelf::new();
        shelf.touch(ONE);
        let library = shelf.library();
        library.refresh(&shelf.folder).unwrap();
        library
            .enrich(|_| async {
                Ok(vec![search_result(
                    "Seitokai ni mo Ana wa Aru",
                    Some("https://i.animepahe.pw/posters/one.jpg"),
                )])
            })
            .await
            .unwrap();

        shelf.touch(TWO);
        let groups = library.refresh(&shelf.folder).unwrap();

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].episode_count, 2);
        assert_eq!(
            groups[0].poster.as_deref(),
            Some("https://i.animepahe.pw/posters/one.jpg")
        );
    }

    #[tokio::test]
    async fn falls_back_to_animepahes_ranking_when_titles_are_localised() {
        let shelf = Shelf::new();
        shelf.touch(ONE);
        let library = shelf.library();
        library.refresh(&shelf.folder).unwrap();

        let groups = library
            .enrich(|_| async {
                Ok(vec![
                    search_result(
                        "Even the Student Council Has Its Holes!",
                        Some("https://i.animepahe.pw/posters/localised.jpg"),
                    ),
                    search_result("Seitokai Yakuindomo", None),
                ])
            })
            .await
            .unwrap();

        assert_eq!(groups[0].anime_id.as_deref(), Some("anime-id"));
        assert_eq!(
            groups[0].poster.as_deref(),
            Some("https://i.animepahe.pw/posters/localised.jpg")
        );
        assert_eq!(groups[0].title, "Seitokai ni mo Ana wa Aru");
    }

    #[tokio::test]
    async fn an_empty_search_keeps_the_placeholder() {
        let shelf = Shelf::new();
        shelf.touch(ONE);
        let library = shelf.library();
        library.refresh(&shelf.folder).unwrap();

        let groups = library.enrich(|_| async { Ok(Vec::new()) }).await.unwrap();

        assert_eq!(groups[0].anime_id, None);
        assert_eq!(groups[0].poster, None);
    }
}
