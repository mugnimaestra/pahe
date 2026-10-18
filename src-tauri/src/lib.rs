mod download;
mod library;
mod poster;
mod probe;
mod proxy;
mod queue;
mod resolver;
mod settings;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use download::Request as DownloadRequest;
use library::{Library, LibraryAnime, OwnedEpisode};
use queue::{Enqueued, Queue, QueueSnapshot};
use resolver::{AnimeDetail, Resolver, ResolverFailure, SearchResult};
use settings::{Settings, Store};
use tauri::{Emitter, Manager};

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

/// Starts one or more Download jobs. The Source is resolved inside each job,
/// never here, because a CDN URL expires before a queued job would run.
#[tauri::command]
async fn enqueue_downloads(
    queue: tauri::State<'_, Arc<Queue>>,
    library: tauri::State<'_, Library>,
    requests: Vec<DownloadRequest>,
) -> Result<Enqueued, String> {
    let requests = requests
        .into_iter()
        .map(normalize_request)
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

fn normalize_request(mut request: DownloadRequest) -> Result<DownloadRequest, String> {
    if request.anime_id.trim().is_empty()
        || request.session.trim().is_empty()
        || request.title.trim().is_empty()
    {
        return Err("Open the Anime again and pick an Episode.".to_owned());
    }
    if request.quality.trim().is_empty() {
        request.quality = "1080p".to_owned();
    }

    Ok(request)
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
            let resolver = Resolver::new(app.handle().clone());
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
                                            .map_err(|failure| failure.message)
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
            search_anime,
            load_anime,
            fetch_poster,
            load_library,
            enrich_library,
            enqueue_downloads,
            load_queue,
            start_queue,
            resume_download,
            owned_episodes
        ])
        .run(tauri::generate_context!())
        .expect("error while running pahe");
}
