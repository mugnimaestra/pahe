mod library;
mod poster;
mod probe;
mod proxy;
mod resolver;
mod settings;

use std::path::{Path, PathBuf};

use library::{Library, LibraryAnime};
use resolver::{AnimeDetail, Resolver, ResolverFailure, SearchResult};
use settings::{Settings, Store};
use tauri::Manager;

#[tauri::command]
fn get_settings(store: tauri::State<'_, Store>) -> Settings {
    store.get()
}

#[tauri::command]
async fn save_settings(
    store: tauri::State<'_, Store>,
    resolver: tauri::State<'_, Resolver>,
    settings: Settings,
) -> Result<Settings, String> {
    let saved = store.save(settings)?;
    resolver.base_url_changed(&saved.base_url).await;
    Ok(saved)
}

#[tauri::command]
async fn search_anime(
    store: tauri::State<'_, Store>,
    resolver: tauri::State<'_, Resolver>,
    query: String,
) -> Result<Vec<SearchResult>, ResolverFailure> {
    let base_url = store.get().base_url;
    resolver.search(&base_url, &query).await
}

#[tauri::command]
async fn load_anime(
    store: tauri::State<'_, Store>,
    resolver: tauri::State<'_, Resolver>,
    anime_id: String,
) -> Result<AnimeDetail, ResolverFailure> {
    let base_url = store.get().base_url;
    resolver.detail(&base_url, &anime_id).await
}

#[tauri::command]
async fn fetch_poster(
    store: tauri::State<'_, Store>,
    resolver: tauri::State<'_, Resolver>,
    url: String,
) -> Result<String, String> {
    let base_url = store.get().base_url;
    let result = resolver.poster(&base_url, &url).await;
    if let Err(error) = &result {
        eprintln!("poster {url}: {error}");
    }
    result
}

#[tauri::command]
async fn load_library(
    store: tauri::State<'_, Store>,
    library: tauri::State<'_, Library>,
) -> Result<Vec<LibraryAnime>, String> {
    library.refresh(Path::new(&store.get().library_folder))
}

#[tauri::command]
async fn enrich_library(
    store: tauri::State<'_, Store>,
    library: tauri::State<'_, Library>,
    resolver: tauri::State<'_, Resolver>,
) -> Result<Vec<LibraryAnime>, String> {
    let base_url = store.get().base_url;
    let resolver = resolver.inner();

    library
        .enrich(|query| {
            let base_url = base_url.clone();
            async move {
                resolver
                    .search(&base_url, &query)
                    .await
                    .map_err(|failure| failure.message)
            }
        })
        .await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();

    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }));
    }

    builder
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            let library_folder = app
                .path()
                .download_dir()
                .or_else(|_| app.path().home_dir().map(|home| home.join("Downloads")))
                .map(|downloads| downloads.join("animepahe"))
                .unwrap_or_else(|_| PathBuf::from("animepahe"));

            app.manage(Store::load(app_data_dir.clone(), library_folder));
            app.manage(Resolver::new(app.handle().clone()));
            app.manage(Library::open(app_data_dir.join(library::INDEX_FILE)));

            // The scan is local and fast; enrichment waits until the Library
            // view asks for it so startup never depends on the network.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let folder = handle.state::<Store>().get().library_folder;
                if let Err(error) = handle.state::<Library>().refresh(Path::new(&folder)) {
                    eprintln!("library scan: {error}");
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            search_anime,
            load_anime,
            fetch_poster,
            load_library,
            enrich_library
        ])
        .run(tauri::generate_context!())
        .expect("error while running pahe");
}
