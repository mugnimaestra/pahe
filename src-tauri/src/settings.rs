use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use url::Url;

pub const DEFAULT_BASE_URL: &str = "https://animepahe.pw";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub base_url: String,
    pub library_folder: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            library_folder: String::new(),
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
            base_url: DEFAULT_BASE_URL.to_owned(),
            library_folder: default_library_folder.into().display().to_string(),
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

fn normalize_base_url(value: &str) -> Result<String, String> {
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
            })
            .unwrap();

        let store = restart(&dir);
        assert_eq!(store.get().base_url, "https://mirror.example");
        assert_eq!(store.get().library_folder, "/media/anime");
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
