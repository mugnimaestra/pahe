use std::future::Future;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use url::Url;

pub const EVENT: &str = "download://job";

const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
const REFERER: &str = "https://kwik.cx/";
/// Space kept free after a Download job's own bytes.
const DISK_MARGIN: u64 = 500 * 1024 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobState {
    Queued,
    Running,
    Completed,
    Failed,
    /// Restored after the app died mid-download; waits for an explicit Resume.
    Interrupted,
    /// Waiting while a Challenge blocks the Resolver; its bytes are kept.
    Paused,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub bytes: u64,
    pub total: Option<u64>,
    /// Bytes per second.
    pub speed: Option<f64>,
    pub eta_seconds: Option<u64>,
}

/// A request to fetch one Episode at one Quality.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub anime_id: String,
    pub title: String,
    pub episode: u32,
    pub session: String,
    pub quality: String,
    /// Fetches again even though the Library owns this Episode and Quality.
    #[serde(default)]
    pub redownload: bool,
}

/// A Download job, shaped so the queue can add more of them without changes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub anime_id: String,
    pub title: String,
    pub episode: u32,
    pub quality: String,
    pub state: JobState,
    pub progress: Option<Progress>,
    pub error: Option<String>,
}

impl Job {
    /// A fresh queued job for `request`, before it reaches the engine.
    pub fn new(request: &Request) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let counter = COUNTER.fetch_add(1, Ordering::SeqCst);

        Self {
            id: format!("{stamp}-{counter}"),
            anime_id: request.anime_id.clone(),
            title: request.title.clone(),
            episode: request.episode,
            quality: request.quality.clone(),
            state: JobState::Queued,
            progress: None,
            error: None,
        }
    }
}

/// The Resolver's answer for a Download job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub cdn_url: String,
    pub quality: String,
    pub group: String,
}

/// Free bytes on the volume holding a folder.
pub trait FreeSpace {
    fn available(&self, path: &Path) -> io::Result<u64>;
}

pub struct Disk;

impl FreeSpace for Disk {
    fn available(&self, path: &Path) -> io::Result<u64> {
        fs2::available_space(path)
    }
}

/// Why one download attempt did not finish.
enum TransferError {
    /// The CDN rejected the URL; a fresh resolution may heal it.
    Expired,
    Failed(String),
}

/// Resolves the Source, then streams it to `<name>.part` and renames on success.
///
/// Resolution is a closure so the engine can be exercised against a local
/// server without a webview. A 403 asks it once more, because a CDN URL can
/// expire between resolution and the request. `report` receives the job on
/// every change.
pub async fn run<D, F, Fut, G>(
    mut job: Job,
    request: &Request,
    folder: &Path,
    disk: &D,
    resolve: F,
    report: G,
) -> Job
where
    D: FreeSpace,
    F: Fn() -> Fut,
    Fut: Future<Output = Result<Resolved, String>>,
    G: Fn(&Job),
{
    job.state = JobState::Running;
    report(&job);

    if let Err(error) = tokio::fs::create_dir_all(folder).await {
        return fail(
            job,
            format!(
                "Could not create the Library folder {}: {error}",
                folder.display()
            ),
            &report,
        );
    }

    let client = match reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .no_proxy()
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return fail(
                job,
                format!("Could not prepare the download: {error}"),
                &report,
            );
        }
    };

    // Two attempts at most: the first resolution, then exactly one more when
    // the CDN says the URL expired.
    let mut expired = false;
    loop {
        let source = match resolve().await {
            Ok(source) => source,
            Err(message) => return fail(job, message, &report),
        };
        job.quality = source.quality.clone();

        match transfer(&mut job, request, folder, disk, &client, &source, &report).await {
            Ok(()) => {
                job.state = JobState::Completed;
                job.error = None;
                report(&job);
                return job;
            }
            Err(TransferError::Expired) if !expired => {
                expired = true;
                report(&job);
            }
            Err(TransferError::Expired) => {
                return fail(
                    job,
                    "The CDN link expired again after a fresh resolution. Retry the Download job or pick another Quality.".to_owned(),
                    &report,
                );
            }
            Err(TransferError::Failed(message)) => return fail(job, message, &report),
        }
    }
}

/// HEADs for the size, checks disk space, streams to `.part`, and renames.
async fn transfer<D, G>(
    job: &mut Job,
    request: &Request,
    folder: &Path,
    disk: &D,
    client: &reqwest::Client,
    source: &Resolved,
    report: &G,
) -> Result<(), TransferError>
where
    D: FreeSpace,
    G: Fn(&Job),
{
    let total = remote_size(client, &source.cdn_url).await?;

    if let Some(total) = total {
        let available = disk.available(folder).map_err(|error| {
            TransferError::Failed(format!("Could not check free disk space: {error}"))
        })?;
        if available < total.saturating_add(DISK_MARGIN) {
            return Err(TransferError::Failed(format!(
                "Not enough disk space: this Episode needs {} and only {} is free.",
                megabytes(total),
                megabytes(available)
            )));
        }
    }

    let fallback = fallback_filename(&request.title, request.episode, &job.quality, &source.group);
    let name = filename_for(&source.cdn_url, &fallback);
    let part = folder.join(format!("{name}.part"));
    let final_path = folder.join(&name);

    stream(job, client, &source.cdn_url, total, &part, report).await?;

    // Windows will not rename onto an existing file; a re-download replaces it.
    if final_path.exists() {
        let _ = tokio::fs::remove_file(&final_path).await;
    }
    tokio::fs::rename(&part, &final_path)
        .await
        .map_err(|error| {
            TransferError::Failed(format!("Could not finish the download: {error}"))
        })?;

    Ok(())
}

async fn remote_size(client: &reqwest::Client, url: &str) -> Result<Option<u64>, TransferError> {
    let response = client
        .head(url)
        .header(reqwest::header::REFERER, REFERER)
        .header(reqwest::header::ACCEPT, "*/*")
        .send()
        .await
        .map_err(|error| TransferError::Failed(format!("The CDN could not be reached: {error}")))?;

    if response.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(TransferError::Expired);
    }
    if !response.status().is_success() {
        return Err(TransferError::Failed(format!(
            "The CDN refused the download (HTTP {}).",
            response.status().as_u16()
        )));
    }

    // A HEAD response has no body, so reqwest reports no body length; the
    // Content-Length header is the CDN's promised size.
    Ok(response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok()))
}

/// Streams into `part`, appending when the partial file and the CDN agree on a
/// Range. A server that ignores Range restarts the file instead of corrupting it.
async fn stream<G>(
    job: &mut Job,
    client: &reqwest::Client,
    url: &str,
    total_hint: Option<u64>,
    part: &Path,
    report: &G,
) -> Result<(), TransferError>
where
    G: Fn(&Job),
{
    let mut resume_from = partial_size(part).await;

    // One retry covers a 416 or a Content-Range that names another offset.
    for _ in 0..2 {
        let mut request = client
            .get(url)
            .header(reqwest::header::REFERER, REFERER)
            .header(reqwest::header::ACCEPT, "*/*");
        if resume_from > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={resume_from}-"));
        }

        let mut response = request.send().await.map_err(|error| {
            TransferError::Failed(format!("The download could not start: {error}"))
        })?;

        let status = response.status();
        if status == reqwest::StatusCode::FORBIDDEN {
            return Err(TransferError::Expired);
        }

        if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            // A part that already holds every byte is done; anything else is
            // untrustworthy and starts again from an empty file.
            if total_hint.is_some_and(|total| total == resume_from) {
                report_finished(job, total_hint, resume_from, report);
                return Ok(());
            }
            let _ = tokio::fs::remove_file(part).await;
            resume_from = 0;
            continue;
        }

        if !status.is_success() {
            return Err(TransferError::Failed(format!(
                "The CDN refused the download (HTTP {}).",
                status.as_u16()
            )));
        }

        let partial = status == reqwest::StatusCode::PARTIAL_CONTENT && resume_from > 0;
        let total = if partial {
            let (start, total) = content_range(&response).ok_or_else(|| {
                TransferError::Failed("The CDN sent an incomplete Range response.".to_owned())
            })?;
            if start != resume_from {
                let _ = tokio::fs::remove_file(part).await;
                resume_from = 0;
                continue;
            }
            total.or(total_hint)
        } else {
            total_hint.or_else(|| response_length(&response))
        };

        // A 200 answer to a Range request means the CDN ignored it: replace the
        // file rather than appending a second copy of its start.
        let mut file = if partial {
            tokio::fs::OpenOptions::new()
                .append(true)
                .open(part)
                .await
                .map_err(|error| {
                    TransferError::Failed(format!(
                        "Could not resume the Episode at {}: {error}",
                        part.display()
                    ))
                })?
        } else {
            tokio::fs::File::create(part).await.map_err(|error| {
                TransferError::Failed(format!(
                    "Could not write the Episode to {}: {error}",
                    part.display()
                ))
            })?
        };

        let started = Instant::now();
        let mut written = if partial { resume_from } else { 0 };
        let mut reported_at = None;

        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    if let Err(error) = file.write_all(&chunk).await {
                        return Err(TransferError::Failed(format!(
                            "Could not write the Episode to {}: {error}",
                            part.display()
                        )));
                    }
                    written += chunk.len() as u64;

                    if reported_at.is_none_or(|last: Instant| last.elapsed() >= PROGRESS_INTERVAL) {
                        reported_at = Some(Instant::now());
                        job.progress = Some(progress(written, total, started.elapsed()));
                        report(job);
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    return Err(TransferError::Failed(format!(
                        "The download was interrupted: {error}"
                    )));
                }
            }
        }

        if let Err(error) = file.flush().await {
            return Err(TransferError::Failed(format!(
                "Could not write the Episode to {}: {error}",
                part.display()
            )));
        }
        drop(file);

        if let Some(total) = total {
            if written != total {
                return Err(TransferError::Failed(format!(
                    "The download stopped after {} of {}.",
                    megabytes(written),
                    megabytes(total)
                )));
            }
            // Trust the bytes on disk, not the loop's counter.
            let size = partial_size(part).await;
            if size != total {
                return Err(TransferError::Failed(format!(
                    "The Episode file is {} but the CDN promised {}. Retry the Download job.",
                    megabytes(size),
                    megabytes(total)
                )));
            }
        }

        report_finished(job, total, written, report);
        return Ok(());
    }

    Err(TransferError::Failed(
        "The CDN would not send the whole Episode. Retry the Download job.".to_owned(),
    ))
}

async fn partial_size(part: &Path) -> u64 {
    tokio::fs::metadata(part)
        .await
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn response_length(response: &reqwest::Response) -> Option<u64> {
    response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
}

/// `<start>` and the total from `Content-Range: bytes <start>-<end>/<total>`.
fn content_range(response: &reqwest::Response) -> Option<(u64, Option<u64>)> {
    let value = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)?
        .to_str()
        .ok()?;
    let range = value.strip_prefix("bytes ")?;
    let (range, total) = range.split_once('/')?;
    let (start, _) = range.split_once('-')?;
    let total = total.trim().parse().ok();

    Some((start.trim().parse().ok()?, total))
}

fn report_finished<G>(job: &mut Job, total: Option<u64>, written: u64, report: &G)
where
    G: Fn(&Job),
{
    let previous = job.progress.as_ref();
    job.progress = Some(Progress {
        bytes: written,
        total,
        speed: previous.and_then(|progress| progress.speed),
        eta_seconds: previous.and_then(|progress| progress.eta_seconds),
    });
    report(job);
}

fn progress(bytes: u64, total: Option<u64>, elapsed: Duration) -> Progress {
    let seconds = elapsed.as_secs_f64();
    let speed = (seconds > 0.0).then(|| bytes as f64 / seconds);
    let eta_seconds = match (speed, total) {
        (Some(speed), Some(total)) if speed > 0.0 => {
            Some(((total.saturating_sub(bytes)) as f64 / speed).round() as u64)
        }
        _ => None,
    };

    Progress {
        bytes,
        total,
        speed,
        eta_seconds,
    }
}

fn fail<G>(mut job: Job, message: String, report: &G) -> Job
where
    G: Fn(&Job),
{
    job.state = JobState::Failed;
    job.error = Some(message);
    report(&job);
    job
}

/// The final filename: the CDN's `?file=` name, sanitised like the CLI.
pub fn filename_for(cdn_url: &str, fallback: &str) -> String {
    let name = file_param(cdn_url)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| fallback.to_owned());

    sanitize_filename(&name)
}

/// `AnimePahe_<title_slug>_-_<NN>_<quality>_<group>.mp4`, matching the CLI.
pub fn fallback_filename(title: &str, episode: u32, quality: &str, group: &str) -> String {
    format!(
        "AnimePahe_{}_-_{episode:02}_{}_{}.mp4",
        title_slug(title),
        sanitize_component(quality),
        sanitize_component(group)
    )
}

fn file_param(cdn_url: &str) -> Option<String> {
    let query = Url::parse(cdn_url).ok()?.query()?.to_owned();
    let raw = query
        .split('&')
        .rev()
        .find_map(|pair| pair.strip_prefix("file="))?;

    Some(percent_decode_str(raw).decode_utf8_lossy().into_owned())
}

fn sanitize_filename(name: &str) -> String {
    let mut cleaned: String = name
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || matches!(ch, '.' | '-' | ' ') {
                ch
            } else {
                '_'
            }
        })
        .collect();

    if cleaned.trim().is_empty() {
        cleaned = "episode.mp4".to_owned();
    }
    if !cleaned.to_ascii_lowercase().ends_with(".mp4") {
        cleaned.push_str(".mp4");
    }

    cleaned
}

/// `Even the Student Council Has Its Holes!` -> `Even_the_Student_Council_Has_Its_Holes`.
pub fn title_slug(title: &str) -> String {
    let words: Vec<String> = title
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect();

    if words.is_empty() {
        "Episode".to_owned()
    } else {
        words.join("_")
    }
}

fn sanitize_component(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|ch| if ch.is_alphanumeric() { ch } else { '_' })
        .collect();
    let cleaned = cleaned.trim_matches('_').to_owned();

    if cleaned.is_empty() {
        "animepahe".to_owned()
    } else {
        cleaned
    }
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    struct FixedDisk(u64);

    impl FreeSpace for FixedDisk {
        fn available(&self, _path: &Path) -> io::Result<u64> {
            Ok(self.0)
        }
    }

    fn request() -> Request {
        Request {
            anime_id: "anime-id".to_owned(),
            title: "Even the Student Council Has Its Holes!".to_owned(),
            episode: 4,
            session: "play-session".to_owned(),
            quality: "1080p".to_owned(),
            redownload: false,
        }
    }

    /// A one-request-at-a-time HTTP server. The plan decides whether Range is
    /// honoured, whether every request is refused, and whether a GET drops
    /// mid-body. `declared` overrides the size HEAD promises.
    #[derive(Clone, Copy, Default)]
    struct Plan {
        honor_range: bool,
        forbid: bool,
        drop_after: Option<usize>,
        declared: Option<u64>,
    }

    struct TestServer {
        url: String,
        requests: Arc<Mutex<Vec<String>>>,
    }

    async fn serve(body: Vec<u8>, file_name: &str, plan: Plan) -> TestServer {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let file_name = file_name.to_owned();
        let requests = Arc::new(Mutex::new(Vec::new()));

        let recorded = Arc::clone(&requests);
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let body = body.clone();
                let recorded = Arc::clone(&recorded);
                tokio::spawn(async move {
                    // A single read can stop mid-request line, so read until the
                    // blank line that ends the headers before answering.
                    let mut buffer = Vec::new();
                    loop {
                        let mut chunk = [0u8; 1024];
                        match socket.read(&mut chunk).await {
                            Ok(0) | Err(_) => break,
                            Ok(read) => {
                                buffer.extend_from_slice(&chunk[..read]);
                                if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
                                    break;
                                }
                            }
                        }
                    }
                    let head_end = buffer
                        .windows(4)
                        .position(|window| window == b"\r\n\r\n")
                        .map(|index| index + 4)
                        .unwrap_or(buffer.len());
                    let head = String::from_utf8_lossy(&buffer[..head_end]).to_ascii_lowercase();
                    recorded.lock().unwrap().push(head.clone());
                    let get = head.starts_with("get ");
                    eprintln!(
                        "serve: raw={:?} get={get} forbid={} drop={:?} range={} declared={:?}",
                        String::from_utf8_lossy(&buffer),
                        plan.forbid,
                        plan.drop_after,
                        plan.honor_range,
                        plan.declared
                    );

                    let response = if plan.forbid {
                        b"HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                            .to_vec()
                    } else if !get {
                        format!(
                            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                            plan.declared.unwrap_or(body.len() as u64)
                        )
                        .into_bytes()
                    } else if let Some(limit) = plan.drop_after {
                        let limited = &body[..limit.min(body.len())];
                        let mut response =
                            b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n"
                                .to_vec();
                        response.extend_from_slice(format!("{:x}\r\n", limited.len()).as_bytes());
                        response.extend_from_slice(limited);
                        response.extend_from_slice(b"\r\n");
                        response
                    } else if let Some(start) =
                        plan.honor_range.then(|| range_start(&head)).flatten()
                    {
                        if start >= body.len() as u64 {
                            b"HTTP/1.1 416 Range Not Satisfiable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                                .to_vec()
                        } else {
                            let partial = &body[start as usize..];
                            let mut response = format!(
                                "HTTP/1.1 206 Partial Content\r\ncontent-length: {}\r\ncontent-range: bytes {start}-{}/{}\r\nconnection: close\r\n\r\n",
                                partial.len(),
                                body.len() - 1,
                                body.len()
                            )
                            .into_bytes();
                            response.extend_from_slice(partial);
                            response
                        }
                    } else {
                        let mut response = format!(
                            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                            body.len()
                        )
                        .into_bytes();
                        response.extend_from_slice(&body);
                        response
                    };

                    let wrote = socket.write_all(&response).await;
                    let shut = socket.shutdown().await;
                    eprintln!(
                        "serve: sent={} write={:?} shutdown={:?}",
                        response.len(),
                        wrote.map_err(|error| error.to_string()),
                        shut.map_err(|error| error.to_string())
                    );
                });
            }
        });

        TestServer {
            url: format!("http://{address}/mp4/abc?file={file_name}"),
            requests,
        }
    }

    fn range_start(head: &str) -> Option<u64> {
        head.lines()
            .find_map(|line| line.strip_prefix("range: bytes="))
            .and_then(|rest| rest.split('-').next())
            .and_then(|start| start.trim().parse().ok())
    }

    async fn run_job(url: String, folder: PathBuf, disk: FixedDisk) -> (Job, Vec<Job>) {
        let reports = Mutex::new(Vec::new());
        let job = Job::new(&request());
        let finished = run(
            job,
            &request(),
            &folder,
            &disk,
            || {
                let url = url.clone();
                async move {
                    Ok(Resolved {
                        cdn_url: url,
                        quality: "1080p".to_owned(),
                        group: "ADN".to_owned(),
                    })
                }
            },
            |job| reports.lock().unwrap().push(job.clone()),
        )
        .await;

        (finished, reports.into_inner().unwrap())
    }

    #[tokio::test]
    async fn streams_the_bytes_then_renames_the_part_file() {
        let body = b"the whole episode".to_vec();
        let server = serve(
            body.clone(),
            "AnimePahe_One_-_04_1080p_ADN.mp4",
            Plan::default(),
        )
        .await;
        let dir = TempDir::new().unwrap();
        let folder = dir.path().join("animepahe");

        let (job, _) = run_job(server.url, folder.clone(), FixedDisk(u64::MAX)).await;

        assert_eq!(job.state, JobState::Completed, "{:?}", job.error);
        assert_eq!(job.error, None);
        assert_eq!(
            std::fs::read(folder.join("AnimePahe_One_-_04_1080p_ADN.mp4")).unwrap(),
            body
        );
        assert!(
            !folder
                .join("AnimePahe_One_-_04_1080p_ADN.mp4.part")
                .exists()
        );
    }

    #[tokio::test]
    async fn reports_progress_with_bytes_and_total() {
        let body = vec![7u8; 64 * 1024];
        let server = serve(
            body.clone(),
            "AnimePahe_One_-_04_1080p_ADN.mp4",
            Plan::default(),
        )
        .await;
        let dir = TempDir::new().unwrap();

        let (job, reports) =
            run_job(server.url, dir.path().to_path_buf(), FixedDisk(u64::MAX)).await;

        assert_eq!(job.state, JobState::Completed);
        assert_eq!(reports[0].progress, None);
        let finished = reports.last().unwrap();
        assert_eq!(finished.state, JobState::Completed);
        let progress = finished.progress.as_ref().unwrap();
        assert_eq!(progress.bytes, body.len() as u64);
        assert_eq!(progress.total, Some(body.len() as u64));
        assert!(progress.speed.is_some());
        assert!(progress.eta_seconds.is_some());
        assert!(
            reports
                .iter()
                .any(|report| report.state == JobState::Running
                    && report.progress.as_ref().is_some_and(|p| p.bytes > 0)),
            "a running report should carry live progress"
        );
    }

    #[tokio::test]
    async fn insufficient_disk_space_fails_before_writing() {
        let body = vec![1u8; 8 * 1024 * 1024];
        let server = serve(
            body.clone(),
            "AnimePahe_One_-_04_1080p_ADN.mp4",
            Plan::default(),
        )
        .await;
        let dir = TempDir::new().unwrap();
        let folder = dir.path().join("animepahe");

        let (job, _) = run_job(server.url, folder.clone(), FixedDisk(100 * 1024 * 1024)).await;

        assert_eq!(job.state, JobState::Failed);
        assert!(job.error.unwrap().contains("disk space"));
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn a_dropped_connection_keeps_the_part_file() {
        let server = serve(
            vec![3u8; 100],
            "AnimePahe_One_-_04_1080p_ADN.mp4",
            Plan {
                declared: Some(1_000),
                drop_after: Some(100),
                ..Plan::default()
            },
        )
        .await;
        let dir = TempDir::new().unwrap();
        let folder = dir.path().to_path_buf();

        let (job, _) = run_job(server.url, folder.clone(), FixedDisk(u64::MAX)).await;

        assert_eq!(job.state, JobState::Failed);
        assert!(job.error.is_some());
        assert!(!folder.join("AnimePahe_One_-_04_1080p_ADN.mp4").exists());
        assert_eq!(
            std::fs::read(folder.join("AnimePahe_One_-_04_1080p_ADN.mp4.part"))
                .unwrap()
                .len(),
            100,
            "error={:?} requests={:?}",
            job.error,
            server.requests.lock().unwrap()
        );
    }

    #[tokio::test]
    async fn a_part_file_resumes_from_its_size_with_a_range_request() {
        let body: Vec<u8> = (0..2_000).map(|byte| byte as u8).collect();
        let name = "AnimePahe_One_-_04_1080p_ADN.mp4";
        let server = serve(
            body.clone(),
            name,
            Plan {
                honor_range: true,
                ..Plan::default()
            },
        )
        .await;
        let dir = TempDir::new().unwrap();
        let folder = dir.path().to_path_buf();
        std::fs::write(folder.join(format!("{name}.part")), &body[..700]).unwrap();

        let (job, _) = run_job(server.url, folder.clone(), FixedDisk(u64::MAX)).await;

        assert_eq!(job.state, JobState::Completed, "{:?}", job.error);
        assert_eq!(std::fs::read(folder.join(name)).unwrap(), body);
        assert!(
            server
                .requests
                .lock()
                .unwrap()
                .iter()
                .any(|head| head.starts_with("get ") && head.contains("range: bytes=700-")),
            "the download should ask for the bytes it is missing"
        );
    }

    #[tokio::test]
    async fn a_server_that_ignores_range_restarts_the_file_cleanly() {
        let body = vec![9u8; 2_000];
        let name = "AnimePahe_One_-_04_1080p_ADN.mp4";
        let server = serve(body.clone(), name, Plan::default()).await;
        let dir = TempDir::new().unwrap();
        let folder = dir.path().to_path_buf();
        std::fs::write(folder.join(format!("{name}.part")), vec![0xEEu8; 700]).unwrap();

        let (job, _) = run_job(server.url, folder.clone(), FixedDisk(u64::MAX)).await;

        assert_eq!(job.state, JobState::Completed, "{:?}", job.error);
        assert_eq!(std::fs::read(folder.join(name)).unwrap(), body);
        assert!(!folder.join(format!("{name}.part")).exists());
        assert!(
            server
                .requests
                .lock()
                .unwrap()
                .iter()
                .any(|head| head.starts_with("get ") && head.contains("range: bytes=700-")),
            "the download should still try to resume"
        );
    }

    #[tokio::test]
    async fn a_403_re_resolves_once_then_fails_with_retry_available() {
        let name = "AnimePahe_One_-_04_1080p_ADN.mp4";
        let server = serve(
            vec![1u8; 100],
            name,
            Plan {
                forbid: true,
                ..Plan::default()
            },
        )
        .await;
        let dir = TempDir::new().unwrap();
        let folder = dir.path().to_path_buf();
        let resolutions = Arc::new(AtomicUsize::new(0));

        let reports = Mutex::new(Vec::new());
        let job = Job::new(&request());
        let finished = run(
            job,
            &request(),
            &folder,
            &FixedDisk(u64::MAX),
            || {
                resolutions.fetch_add(1, Ordering::SeqCst);
                let url = server.url.clone();
                async move {
                    Ok(Resolved {
                        cdn_url: url,
                        quality: "1080p".to_owned(),
                        group: "ADN".to_owned(),
                    })
                }
            },
            |job| reports.lock().unwrap().push(job.clone()),
        )
        .await;

        assert_eq!(resolutions.load(Ordering::SeqCst), 2);
        assert_eq!(finished.state, JobState::Failed);
        assert!(
            finished
                .error
                .as_deref()
                .is_some_and(|error| error.contains("expired again"))
        );
    }

    #[tokio::test]
    async fn a_403_re_resolves_and_downloads_on_the_second_try() {
        let body = vec![4u8; 1_500];
        let name = "AnimePahe_One_-_04_1080p_ADN.mp4";
        let expired = serve(
            body.clone(),
            name,
            Plan {
                forbid: true,
                ..Plan::default()
            },
        )
        .await;
        let fresh = serve(body.clone(), name, Plan::default()).await;
        let dir = TempDir::new().unwrap();
        let folder = dir.path().to_path_buf();
        let resolutions = Arc::new(AtomicUsize::new(0));

        let reports = Mutex::new(Vec::new());
        let job = Job::new(&request());
        let finished = run(
            job,
            &request(),
            &folder,
            &FixedDisk(u64::MAX),
            || {
                let attempt = resolutions.fetch_add(1, Ordering::SeqCst);
                let url = if attempt == 0 {
                    expired.url.clone()
                } else {
                    fresh.url.clone()
                };
                async move {
                    Ok(Resolved {
                        cdn_url: url,
                        quality: "1080p".to_owned(),
                        group: "ADN".to_owned(),
                    })
                }
            },
            |job| reports.lock().unwrap().push(job.clone()),
        )
        .await;

        assert_eq!(resolutions.load(Ordering::SeqCst), 2);
        assert_eq!(finished.state, JobState::Completed, "{:?}", finished.error);
        assert_eq!(std::fs::read(folder.join(name)).unwrap(), body);
    }

    #[tokio::test]
    async fn a_retry_resumes_the_part_left_by_a_failed_attempt() {
        let body: Vec<u8> = (0..3_000).map(|byte| byte as u8).collect();
        let name = "AnimePahe_One_-_04_1080p_ADN.mp4";
        let dropped = serve(
            body.clone(),
            name,
            Plan {
                declared: Some(body.len() as u64),
                drop_after: Some(900),
                ..Plan::default()
            },
        )
        .await;
        let dir = TempDir::new().unwrap();
        let folder = dir.path().to_path_buf();

        let (failed, _) = run_job(dropped.url, folder.clone(), FixedDisk(u64::MAX)).await;
        assert_eq!(failed.state, JobState::Failed);
        assert_eq!(
            std::fs::read(folder.join(format!("{name}.part")))
                .unwrap()
                .len(),
            900,
            "error={:?} requests={:?}",
            failed.error,
            dropped.requests.lock().unwrap()
        );

        let fixed = serve(
            body.clone(),
            name,
            Plan {
                honor_range: true,
                ..Plan::default()
            },
        )
        .await;
        let (retried, _) = run_job(fixed.url, folder.clone(), FixedDisk(u64::MAX)).await;

        assert_eq!(retried.state, JobState::Completed, "{:?}", retried.error);
        assert_eq!(std::fs::read(folder.join(name)).unwrap(), body);
        assert!(
            fixed
                .requests
                .lock()
                .unwrap()
                .iter()
                .any(|head| head.starts_with("get ") && head.contains("range: bytes=900-")),
            "the retry should resume from the part's size"
        );
    }

    #[test]
    fn takes_the_filename_from_the_cdn_file_parameter() {
        let url = "https://vault-99.owocdn.top/mp4/99/02/abc?file=AnimePahe_Seitokai_ni_mo_Ana_wa_Aru_-_01v2_1080p_ADN.mp4";

        assert_eq!(
            filename_for(url, "fallback.mp4"),
            "AnimePahe_Seitokai_ni_mo_Ana_wa_Aru_-_01v2_1080p_ADN.mp4"
        );
    }

    #[test]
    fn url_decodes_the_filename_before_sanitising_it() {
        assert_eq!(
            filename_for(
                "https://cdn.example/mp4/x?file=AnimePahe_One%20Two_-_01_1080p_ADN.mp4",
                "fallback.mp4"
            ),
            "AnimePahe_One Two_-_01_1080p_ADN.mp4"
        );
        assert_eq!(
            filename_for(
                "https://cdn.example/mp4/x?file=Episode%5B1%5D:%20Name",
                "fallback.mp4"
            ),
            "Episode_1__ Name.mp4"
        );
    }

    #[test]
    fn a_filename_cannot_escape_the_library_folder() {
        assert_eq!(
            filename_for(
                "https://cdn.example/mp4/x?file=../../etc/passwd",
                "fallback.mp4"
            ),
            ".._.._etc_passwd.mp4"
        );
    }

    #[test]
    fn falls_back_when_the_cdn_has_no_file_parameter() {
        assert_eq!(
            fallback_filename("Even the Student Council Has Its Holes!", 4, "1080p", "ADN"),
            "AnimePahe_Even_the_Student_Council_Has_Its_Holes_-_04_1080p_ADN.mp4"
        );
        assert_eq!(
            filename_for("https://cdn.example/mp4/x", "fallback.mp4"),
            "fallback.mp4"
        );
        assert_eq!(
            filename_for("https://cdn.example/mp4/x?file=", "fallback.mp4"),
            "fallback.mp4"
        );
    }

    #[test]
    fn an_extensionless_filename_gets_an_mp4_suffix() {
        assert_eq!(
            filename_for("https://cdn.example/mp4/x?file=episode", "fallback.mp4"),
            "episode.mp4"
        );
    }
}
