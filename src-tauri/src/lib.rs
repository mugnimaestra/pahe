mod download;
mod library;
mod player;
mod poster;
mod probe;
mod proxy;
mod queue;
mod resolver;
mod settings;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use download::{Failure as DownloadFailure, FailureReason, Request as DownloadRequest};
use library::{Library, LibraryAnime, LibraryItem, OwnedEpisode};
use probe::ProbeOutcome;
use queue::{Enqueued, Queue, QueueSnapshot};
use resolver::{AnimeDetail, Resolver, ResolverFailure, SearchResult};
use settings::{Settings, Store};
use tauri::{Emitter, Manager};

/// Allows the Library folder through the asset protocol, so the player can
/// load a Library item's file in the main window. The scope matches real
/// paths, so a symlinked folder is allowed as its target.
fn allow_library<R: tauri::Runtime>(manager: &impl Manager<R>, folder: &Path) {
    let folder = fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
    if let Err(error) = manager.asset_protocol_scope().allow_directory(folder, true) {
        eprintln!("asset scope: {error}");
    }
}

#[tauri::command]
fn get_settings(store: tauri::State<'_, Store>) -> Settings {
    store.get()
}

#[tauri::command]
async fn save_settings(
    app: tauri::AppHandle,
    store: tauri::State<'_, Store>,
    resolver: tauri::State<'_, Resolver>,
    library: tauri::State<'_, Library>,
    settings: Settings,
) -> Result<Settings, String> {
    let previous = store.get();
    let saved = store.save(settings)?;

    // A new Base URL clears the Resolver's cookies and probe cache; a new proxy
    // policy rebuilds the Resolver window; a new Library folder re-scans.
    resolver.base_url_changed(&saved.base_url).await;
    resolver.set_proxy_policy(saved.proxy).await;
    allow_library(&app, Path::new(&saved.library_folder));
    if saved.library_folder != previous.library_folder
        && let Err(error) = library.refresh(Path::new(&saved.library_folder))
    {
        eprintln!("library scan: {error}");
    }

    Ok(saved)
}

/// Probes a Base URL the user is considering, so Settings can tell a dead
/// mirror from a Poisoned resolver before it is saved.
#[tauri::command]
async fn test_base_url(base_url: String) -> Result<ProbeOutcome, String> {
    let base_url = settings::normalize_base_url(&base_url)?;

    Ok(probe::check(&base_url).await)
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

/// Starts one or more Download jobs. The Source is resolved inside each job,
/// never here, because a CDN URL expires before a queued job would run.
#[tauri::command]
async fn enqueue_downloads(
    store: tauri::State<'_, Store>,
    queue: tauri::State<'_, Arc<Queue>>,
    library: tauri::State<'_, Library>,
    requests: Vec<DownloadRequest>,
) -> Result<Enqueued, String> {
    let default_quality = store.get().default_quality;
    let requests = requests
        .into_iter()
        .map(|request| request.normalized(&default_quality))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(queue.enqueue(requests, |request| {
        library.owns(
            &request.anime_id,
            &request.title,
            request.episode,
            &request.quality,
        )
    }))
}

/// Keeps a Resolver failure's state on the Download job, so the UI can name it.
fn download_failure(failure: ResolverFailure) -> DownloadFailure {
    let reason = match failure.kind {
        resolver::ResolverFailureKind::Unreachable => FailureReason::Unreachable,
        resolver::ResolverFailureKind::DeadSource => FailureReason::DeadSource,
        resolver::ResolverFailureKind::ChallengeTimedOut => FailureReason::ChallengeTimedOut,
        resolver::ResolverFailureKind::Failed => FailureReason::Failed,
    };

    DownloadFailure {
        reason,
        message: failure.message,
    }
}

#[tauri::command]
fn load_queue(queue: tauri::State<'_, Arc<Queue>>) -> QueueSnapshot {
    queue.snapshot()
}

/// Starts the worker for jobs that were restored after a restart.
#[tauri::command]
fn start_queue(queue: tauri::State<'_, Arc<Queue>>) -> QueueSnapshot {
    queue.start()
}

/// Resumes an interrupted or paused job, or retries a failed one.
#[tauri::command]
fn resume_download(
    queue: tauri::State<'_, Arc<Queue>>,
    job_id: String,
) -> Result<QueueSnapshot, String> {
    queue.resume(&job_id)?;
    Ok(queue.snapshot())
}

#[tauri::command]
fn owned_episodes(
    library: tauri::State<'_, Library>,
    anime_id: String,
    title: String,
) -> Vec<OwnedEpisode> {
    library.owned(&anime_id, &title)
}

/// Every Library item of an Anime, for the Library's Episode list.
#[tauri::command]
fn library_items(
    library: tauri::State<'_, Library>,
    anime_id: Option<String>,
    title: String,
) -> Vec<LibraryItem> {
    library.items(anime_id.as_deref().unwrap_or_default(), &title)
}

/// Rejects with a plain-language message when a Library item's file is gone.
#[tauri::command]
fn check_library_file(library: tauri::State<'_, Library>, path: String) -> Result<(), String> {
    library.file(&path).map(|_| ())
}

/// Hands a Library item's file to the OS default player.
#[tauri::command]
fn open_externally(library: tauri::State<'_, Library>, path: String) -> Result<(), String> {
    player::open_externally(&library.file(&path)?)
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
            let folder = app.state::<Store>().get().library_folder;
            allow_library(app, Path::new(&folder));
            let resolver = Resolver::new(app.handle().clone(), app.state::<Store>().get().proxy);
            let mut challenge = resolver.challenge_watch();
            app.manage(resolver);
            app.manage(Library::open(app_data_dir.join(library::INDEX_FILE)));

            let queue = Arc::new(Queue::open(app_data_dir.join(queue::QUEUE_FILE), {
                let handle = app.handle().clone();
                move |job| {
                    let _ = handle.emit(download::EVENT, job);
                }
            }));
            app.manage(Arc::clone(&queue));

            // A Challenge pauses the queue while the user clears it; clearance
            // resumes the running job and lets later jobs start.
            tauri::async_runtime::spawn({
                let queue = Arc::clone(&queue);
                async move {
                    while challenge.changed().await.is_ok() {
                        queue.set_challenge(*challenge.borrow_and_update());
                    }
                }
            });

            // The scan is local and fast; enrichment waits until the Library
            // view asks for it so startup never depends on the network.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let folder = handle.state::<Store>().get().library_folder;
                if let Err(error) = handle.state::<Library>().refresh(Path::new(&folder)) {
                    eprintln!("library scan: {error}");
                }
            });

            // One worker drives the serial queue; jobs publish their own state.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                queue
                    .serve(move |job, request| {
                        let handle = handle.clone();
                        async move {
                            let settings = handle.state::<Store>().get();
                            let folder = PathBuf::from(settings.library_folder);
                            let base_url = settings.base_url;
                            let quality = request.quality.clone();
                            let anime_id = request.anime_id.clone();
                            let session = request.session.clone();
                            let queue = handle.state::<Arc<Queue>>().inner().clone();

                            let finished = download::run(
                                job,
                                &request,
                                &folder,
                                &download::Disk,
                                || {
                                    let handle = handle.clone();
                                    let base_url = base_url.clone();
                                    let anime_id = anime_id.clone();
                                    let session = session.clone();
                                    let quality = quality.clone();
                                    async move {
                                        handle
                                            .state::<Resolver>()
                                            .resolve_source(
                                                &base_url, &anime_id, &session, &quality,
                                            )
                                            .await
                                            .map(|source| download::Resolved {
                                                cdn_url: source.cdn_url,
                                                quality: source.quality,
                                                group: source.group,
                                            })
                                            .map_err(download_failure)
                                    }
                                },
                                |job| queue.report(job),
                            )
                            .await;

                            if finished.state == download::JobState::Completed {
                                let _ = handle.state::<Library>().refresh(&folder);
                            }
                            finished
                        }
                    })
                    .await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            test_base_url,
            search_anime,
            load_anime,
            fetch_poster,
            load_library,
            enrich_library,
            enqueue_downloads,
            load_queue,
            start_queue,
            resume_download,
            owned_episodes,
            library_items,
            check_library_file,
            open_externally
        ])
        .run(tauri::generate_context!())
        .expect("error while running pahe");
}

#[cfg(test)]
mod tests {
    use super::*;
    use resolver::ResolverFailureKind;

    #[test]
    fn a_resolver_failure_keeps_its_state_on_the_download_job() {
        for (kind, reason) in [
            (ResolverFailureKind::Unreachable, FailureReason::Unreachable),
            (ResolverFailureKind::DeadSource, FailureReason::DeadSource),
            (
                ResolverFailureKind::ChallengeTimedOut,
                FailureReason::ChallengeTimedOut,
            ),
            (ResolverFailureKind::Failed, FailureReason::Failed),
        ] {
            let failure = download_failure(ResolverFailure {
                kind,
                message: "why".to_owned(),
            });

            assert_eq!(failure.reason, reason);
            assert_eq!(failure.message, "why");
        }
    }
}
