mod poster;
mod probe;
mod proxy;
mod resolver;
mod settings;

use resolver::{Resolver, SearchFailure, SearchResult};
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
) -> Result<Vec<SearchResult>, SearchFailure> {
    let base_url = store.get().base_url;
    resolver.search(&base_url, &query).await
}

#[tauri::command]
async fn fetch_poster(
    store: tauri::State<'_, Store>,
    resolver: tauri::State<'_, Resolver>,
    url: String,
) -> Result<String, String> {
    let base_url = store.get().base_url;
    resolver.poster(&base_url, &url).await
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
            app.manage(Store::load(app.path().app_data_dir()?));
            app.manage(Resolver::new(app.handle().clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            search_anime,
            fetch_poster
        ])
        .run(tauri::generate_context!())
        .expect("error while running pahe");
}
