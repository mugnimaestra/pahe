use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use url::Url;

pub const DEFAULT_BASE_URL: &str = "https://animepahe.pw";
pub const DEFAULT_QUALITY: &str = "1080p";

/// When the Resolver tunnels through pahe's local DoH proxy. `Auto` probes the
/// Base URL and uses the proxy only when the resolver is Poisoned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProxyPolicy {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub base_url: String,
    pub library_folder: String,
    pub default_quality: String,
    pub proxy: ProxyPolicy,
    /// What the Resolver webview says it is. Empty keeps the system default.
    pub resolver_user_agent: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            library_folder: String::new(),
            default_quality: DEFAULT_QUALITY.to_owned(),
            proxy: ProxyPolicy::Auto,
            resolver_user_agent: String::new(),
        }
    }
}

const SETTINGS_FILE: &str = "settings.json";

pub struct Store {
    path: PathBuf,
    current: Mutex<Settings>,
}

impl Store {
    pub fn load(
        app_data_dir: impl Into<PathBuf>,
        default_library_folder: impl Into<PathBuf>,
    ) -> Self {
        let path = app_data_dir.into().join(SETTINGS_FILE);
        let default = Settings {
            library_folder: default_library_folder.into().display().to_string(),
            ..Settings::default()
        };
        let mut current = fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_else(|| default.clone());

        // Settings written before the Library folder existed keep working.
        if current.library_folder.trim().is_empty() {
            current.library_folder = default.library_folder;
        }

        Self {
            path,
            current: Mutex::new(current),
        }
    }

    pub fn get(&self) -> Settings {
        self.current.lock().expect("settings lock poisoned").clone()
    }

    pub fn save(&self, settings: Settings) -> Result<Settings, String> {
        let settings = Settings {
            base_url: normalize_base_url(&settings.base_url)?,
            library_folder: normalize_library_folder(&settings.library_folder)?,
            default_quality: normalize_default_quality(&settings.default_quality)?,
            proxy: settings.proxy,
            resolver_user_agent: settings.resolver_user_agent.trim().to_owned(),
        };
        let mut current = self.current.lock().expect("settings lock poisoned");

        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)
                .map_err(|e| format!("could not create settings folder: {e}"))?;
        }
        let json = serde_json::to_vec_pretty(&settings)
            .map_err(|e| format!("could not encode settings: {e}"))?;
        let temp = self.path.with_extension("json.tmp");
        fs::write(&temp, json).map_err(|e| format!("could not save settings: {e}"))?;
        fs::rename(&temp, &self.path).map_err(|e| format!("could not save settings: {e}"))?;

        *current = settings.clone();
        Ok(settings)
    }
}

/// Trims and validates a Base URL, returning the form pahe stores.
pub fn normalize_base_url(value: &str) -> Result<String, String> {
    let base_url = value.trim().trim_end_matches('/');
    let url =
        Url::parse(base_url).map_err(|_| "Base URL must be a valid http(s) URL".to_owned())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("Base URL must be a valid http(s) URL".to_owned());
    }

    Ok(base_url.to_owned())
}

fn normalize_library_folder(value: &str) -> Result<String, String> {
    let folder = value.trim();
    if folder.is_empty() {
        return Err("Library folder must not be empty".to_owned());
    }

    Ok(folder.to_owned())
}

fn normalize_default_quality(value: &str) -> Result<String, String> {
    let quality = value.trim();
    if quality.is_empty() {
        return Err("Default Quality must not be empty".to_owned());
    }

    Ok(quality.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn downloads(dir: &TempDir) -> PathBuf {
        dir.path().join("Downloads").join("animepahe")
    }

    fn restart(dir: &TempDir) -> Store {
        Store::load(dir.path(), downloads(dir))
    }

    fn settings(base_url: &str) -> Settings {
        Settings {
            base_url: base_url.to_owned(),
            library_folder: "/library".to_owned(),
            ..Settings::default()
        }
    }

    #[test]
    fn defaults_base_url_and_library_folder_when_no_settings_file_exists() {
        let dir = TempDir::new().unwrap();

        let store = restart(&dir);

        assert_eq!(store.get().base_url, DEFAULT_BASE_URL);
        assert_eq!(
            store.get().library_folder,
            downloads(&dir).display().to_string()
        );
    }

    #[test]
    fn saved_settings_survive_a_restart() {
        let dir = TempDir::new().unwrap();
        restart(&dir)
            .save(Settings {
                base_url: "https://mirror.example".to_owned(),
                library_folder: "/media/anime".to_owned(),
                default_quality: "720p".to_owned(),
                proxy: ProxyPolicy::Never,
                resolver_user_agent: "Mozilla/5.0 (Macintosh) Chrome/141.0.0.0".to_owned(),
            })
            .unwrap();

        let store = restart(&dir);
        assert_eq!(store.get().base_url, "https://mirror.example");
        assert_eq!(store.get().library_folder, "/media/anime");
        assert_eq!(store.get().default_quality, "720p");
        assert_eq!(store.get().proxy, ProxyPolicy::Never);
        assert_eq!(
            store.get().resolver_user_agent,
            "Mozilla/5.0 (Macintosh) Chrome/141.0.0.0"
        );
    }

    #[test]
    fn save_trims_the_resolver_user_agent() {
        let dir = TempDir::new().unwrap();
        let store = restart(&dir);

        let saved = store
            .save(Settings {
                resolver_user_agent: "  Mozilla/5.0 (Macintosh)  ".to_owned(),
                ..settings(DEFAULT_BASE_URL)
            })
            .unwrap();

        assert_eq!(saved.resolver_user_agent, "Mozilla/5.0 (Macintosh)");
    }

    #[test]
    fn defaults_the_quality_and_proxy_policy_when_no_settings_file_exists() {
        let dir = TempDir::new().unwrap();

        let store = restart(&dir);

        assert_eq!(store.get().default_quality, DEFAULT_QUALITY);
        assert_eq!(store.get().proxy, ProxyPolicy::Auto);
        assert_eq!(
            store.get().resolver_user_agent,
            "",
            "an empty User-Agent keeps the system default"
        );
    }

    #[test]
    fn settings_written_before_the_quality_and_proxy_policy_get_defaults() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("settings.json"),
            r#"{"baseUrl": "https://mirror.example", "libraryFolder": "/media/anime"}"#,
        )
        .unwrap();

        let store = restart(&dir);

        assert_eq!(store.get().default_quality, DEFAULT_QUALITY);
        assert_eq!(store.get().proxy, ProxyPolicy::Auto);
        assert_eq!(store.get().resolver_user_agent, "");
    }

    #[test]
    fn an_empty_default_quality_is_rejected_and_not_persisted() {
        let dir = TempDir::new().unwrap();
        let store = restart(&dir);

        assert!(
            store
                .save(Settings {
                    default_quality: "  ".to_owned(),
                    ..settings(DEFAULT_BASE_URL)
                })
                .is_err()
        );

        assert_eq!(store.get().default_quality, DEFAULT_QUALITY);
        assert_eq!(restart(&dir).get().default_quality, DEFAULT_QUALITY);
    }

    #[test]
    fn save_trims_the_default_quality() {
        let dir = TempDir::new().unwrap();
        let store = restart(&dir);

        let saved = store
            .save(Settings {
                default_quality: "  720p ".to_owned(),
                ..settings(DEFAULT_BASE_URL)
            })
            .unwrap();

        assert_eq!(saved.default_quality, "720p");
    }

    #[test]
    fn settings_written_before_the_library_folder_get_the_default() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("settings.json"),
            r#"{"baseUrl": "https://mirror.example"}"#,
        )
        .unwrap();

        let store = restart(&dir);

        assert_eq!(store.get().base_url, "https://mirror.example");
        assert_eq!(
            store.get().library_folder,
            downloads(&dir).display().to_string()
        );
    }

    #[test]
    fn trailing_slash_is_removed_from_the_base_url() {
        let dir = TempDir::new().unwrap();
        let store = restart(&dir);

        let saved = store
            .save(Settings {
                base_url: "https://animepahe.pw/".to_owned(),
                library_folder: "/library".to_owned(),
                ..Settings::default()
            })
            .unwrap();

        assert_eq!(saved.base_url, "https://animepahe.pw");
        assert_eq!(restart(&dir).get().base_url, "https://animepahe.pw");
    }

    #[test]
    fn corrupt_settings_file_falls_back_to_defaults() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("settings.json"), "{ not json").unwrap();

        let store = restart(&dir);
        assert_eq!(store.get().base_url, DEFAULT_BASE_URL);
        assert_eq!(
            store.get().library_folder,
            downloads(&dir).display().to_string()
        );
    }

    #[test]
    fn invalid_base_url_is_rejected_and_not_persisted() {
        let dir = TempDir::new().unwrap();
        let store = restart(&dir);

        for invalid in ["", "not a url", "ftp://animepahe.pw", "https://"] {
            assert!(
                store
                    .save(Settings {
                        base_url: invalid.to_owned(),
                        library_folder: "/library".to_owned(),
                        ..Settings::default()
                    })
                    .is_err(),
                "{invalid:?} should be rejected"
            );
        }

        assert_eq!(store.get().base_url, DEFAULT_BASE_URL);
        assert_eq!(restart(&dir).get().base_url, DEFAULT_BASE_URL);
    }

    #[test]
    fn an_empty_library_folder_is_rejected_and_not_persisted() {
        let dir = TempDir::new().unwrap();
        let store = restart(&dir);

        assert!(
            store
                .save(Settings {
                    base_url: "https://mirror.example".to_owned(),
                    library_folder: "  ".to_owned(),
                    ..Settings::default()
                })
                .is_err()
        );

        assert_eq!(
            store.get().library_folder,
            downloads(&dir).display().to_string()
        );
        assert_eq!(
            restart(&dir).get().library_folder,
            downloads(&dir).display().to_string()
        );
    }

    #[test]
    fn save_trims_the_library_folder() {
        let dir = TempDir::new().unwrap();
        let store = restart(&dir);

        let saved = store
            .save(Settings {
                base_url: "https://animepahe.pw".to_owned(),
                library_folder: "  /media/anime  ".to_owned(),
                ..Settings::default()
            })
            .unwrap();

        assert_eq!(saved.library_folder, "/media/anime");
    }

    #[test]
    fn save_creates_the_app_data_directory() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("pahe");

        Store::load(&nested, downloads(&dir))
            .save(settings("https://mirror.example"))
            .unwrap();

        assert!(nested.join("settings.json").is_file());
    }
}
