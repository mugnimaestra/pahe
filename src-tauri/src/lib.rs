mod settings;

use settings::{Settings, Store};
use tauri::Manager;

#[tauri::command]
fn get_settings(store: tauri::State<'_, Store>) -> Settings {
    store.get()
}

#[tauri::command]
fn save_settings(store: tauri::State<'_, Store>, settings: Settings) -> Result<Settings, String> {
    store.save(settings)
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_settings, save_settings])
        .run(tauri::generate_context!())
        .expect("error while running pahe");
}
