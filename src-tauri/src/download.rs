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
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub anime_id: String,
    pub title: String,
    pub episode: u32,
    pub session: String,
    pub quality: String,
}

/// A Download job, shaped so the queue can add more of them without changes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub anime_id: String,
    pub episode: u32,
    pub quality: String,
    pub state: JobState,
    pub progress: Option<Progress>,
    pub error: Option<String>,
}

impl Job {
    /// A fresh running job for `request`, before its Source is resolved.
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
            episode: request.episode,
            quality: request.quality.clone(),
            state: JobState::Running,
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

/// Resolves the Source, then streams it to `<name>.part` and renames on success.
///
/// Resolution is a closure so the engine can be exercised against a local
/// server without a webview. `report` receives the job on every change.
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
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Resolved, String>>,
    G: Fn(&Job),
{
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

    let source = match resolve().await {
        Ok(source) => source,
        Err(message) => return fail(job, message, &report),
    };
    job.quality = source.quality.clone();

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

    let total = match remote_size(&client, &source.cdn_url).await {
        Ok(total) => total,
        Err(message) => return fail(job, message, &report),
    };

    if let Some(total) = total {
        let available = match disk.available(folder) {
            Ok(available) => available,
            Err(error) => {
                return fail(
                    job,
                    format!("Could not check free disk space: {error}"),
                    &report,
                );
            }
        };
        if available < total.saturating_add(DISK_MARGIN) {
            return fail(
                job,
                format!(
                    "Not enough disk space: this Episode needs {} and only {} is free.",
                    megabytes(total),
                    megabytes(available)
                ),
                &report,
            );
        }
    }

    let fallback = fallback_filename(&request.title, request.episode, &job.quality, &source.group);
    let name = filename_for(&source.cdn_url, &fallback);
    let part = folder.join(format!("{name}.part"));
    let final_path = folder.join(&name);
    let job = stream(job, &client, &source.cdn_url, total, &part, &report).await;

    if job.state == JobState::Failed {
        return job;
    }

    // Windows will not rename onto an existing file; a re-download replaces it.
    if final_path.exists() {
        let _ = tokio::fs::remove_file(&final_path).await;
    }
    if let Err(error) = tokio::fs::rename(&part, &final_path).await {
        return fail(
            job,
            format!("Could not finish the download: {error}"),
            &report,
        );
    }

    let mut job = job;
    job.state = JobState::Completed;
    job.error = None;
    report(&job);
    job
}

async fn remote_size(client: &reqwest::Client, url: &str) -> Result<Option<u64>, String> {
    let response = client
        .head(url)
        .header(reqwest::header::REFERER, REFERER)
        .header(reqwest::header::ACCEPT, "*/*")
        .send()
        .await
        .map_err(|error| format!("The CDN could not be reached: {error}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "The CDN refused the download (HTTP {}).",
            response.status().as_u16()
        ));
    }

    // A HEAD response has no body, so reqwest reports no body length; the
    // Content-Length header is the CDN's promised size.
    Ok(response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok()))
}

async fn stream<G>(
    mut job: Job,
    client: &reqwest::Client,
    url: &str,
    total: Option<u64>,
    part: &Path,
    report: &G,
) -> Job
where
    G: Fn(&Job),
{
    let mut response = match client
        .get(url)
        .header(reqwest::header::REFERER, REFERER)
        .header(reqwest::header::ACCEPT, "*/*")
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return fail(
                job,
                format!("The download could not start: {error}"),
                report,
            );
        }
    };

    if !response.status().is_success() {
        return fail(
            job,
            format!(
                "The CDN refused the download (HTTP {}).",
                response.status().as_u16()
            ),
            report,
        );
    }

    let mut file = match tokio::fs::File::create(part).await {
        Ok(file) => file,
        Err(error) => {
            return fail(
                job,
                format!("Could not write the Episode to {}: {error}", part.display()),
                report,
            );
        }
    };

    let started = Instant::now();
    let mut written = 0u64;
    let mut reported_at = None;

    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if let Err(error) = file.write_all(&chunk).await {
                    return fail(
                        job,
                        format!("Could not write the Episode to {}: {error}", part.display()),
                        report,
                    );
                }
                written += chunk.len() as u64;

                if reported_at.is_none_or(|last: Instant| last.elapsed() >= PROGRESS_INTERVAL) {
                    reported_at = Some(Instant::now());
                    job.progress = Some(progress(written, total, started.elapsed()));
                    report(&job);
                }
            }
            Ok(None) => break,
            Err(error) => {
                return fail(
                    job,
                    format!("The download was interrupted: {error}"),
                    report,
                );
            }
        }
    }

    if let Err(error) = file.flush().await {
        return fail(
            job,
            format!("Could not write the Episode to {}: {error}", part.display()),
            report,
        );
    }
    drop(file);

    if let Some(total) = total
        && written != total
    {
        return fail(
            job,
            format!(
                "The download stopped after {} of {}.",
                megabytes(written),
                megabytes(total)
            ),
            report,
        );
    }

    job.progress = Some(progress(written, total, started.elapsed()));
    report(&job);
    job
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

fn title_slug(title: &str) -> String {
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
    use std::sync::Mutex;
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
        }
    }

    /// A one-request-at-a-time HTTP server. `body` may be shorter than
    /// `declared`, which simulates a connection that dropped mid-file.
    async fn serve(declared: u64, body: Vec<u8>, file_name: &str) -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let file_name = file_name.to_owned();

        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let body = body.clone();
                tokio::spawn(async move {
                    let mut buffer = [0u8; 2048];
                    let read = socket.read(&mut buffer).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buffer[..read]);
                    let head = format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {declared}\r\nconnection: close\r\n\r\n"
                    );
                    let _ = socket.write_all(head.as_bytes()).await;
                    if request.starts_with("GET") {
                        let _ = socket.write_all(&body).await;
                    }
                    let _ = socket.shutdown().await;
                });
            }
        });

        format!("http://{address}/mp4/abc?file={file_name}")
    }

    async fn run_job(url: String, folder: PathBuf, disk: FixedDisk) -> (Job, Vec<Job>) {
        let reports = Mutex::new(Vec::new());
        let job = Job::new(&request());
        let finished = run(
            job,
            &request(),
            &folder,
            &disk,
            || async move {
                Ok(Resolved {
                    cdn_url: url,
                    quality: "1080p".to_owned(),
                    group: "ADN".to_owned(),
                })
            },
            |job| reports.lock().unwrap().push(job.clone()),
        )
        .await;

        (finished, reports.into_inner().unwrap())
    }

    #[tokio::test]
    async fn streams_the_bytes_then_renames_the_part_file() {
        let body = b"the whole episode".to_vec();
        let url = serve(
            body.len() as u64,
            body.clone(),
            "AnimePahe_One_-_04_1080p_ADN.mp4",
        )
        .await;
        let dir = TempDir::new().unwrap();
        let folder = dir.path().join("animepahe");

        let (job, _) = run_job(url, folder.clone(), FixedDisk(u64::MAX)).await;

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
        let url = serve(
            body.len() as u64,
            body.clone(),
            "AnimePahe_One_-_04_1080p_ADN.mp4",
        )
        .await;
        let dir = TempDir::new().unwrap();

        let (job, reports) = run_job(url, dir.path().to_path_buf(), FixedDisk(u64::MAX)).await;

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
        let url = serve(body.len() as u64, body, "AnimePahe_One_-_04_1080p_ADN.mp4").await;
        let dir = TempDir::new().unwrap();
        let folder = dir.path().join("animepahe");

        let (job, _) = run_job(url, folder.clone(), FixedDisk(100 * 1024 * 1024)).await;

        assert_eq!(job.state, JobState::Failed);
        assert!(job.error.unwrap().contains("disk space"));
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn a_dropped_connection_keeps_the_part_file() {
        let url = serve(1_000, vec![3u8; 100], "AnimePahe_One_-_04_1080p_ADN.mp4").await;
        let dir = TempDir::new().unwrap();
        let folder = dir.path().to_path_buf();

        let (job, _) = run_job(url, folder.clone(), FixedDisk(u64::MAX)).await;

        assert_eq!(job.state, JobState::Failed);
        assert!(job.error.is_some());
        assert!(!folder.join("AnimePahe_One_-_04_1080p_ADN.mp4").exists());
        assert_eq!(
            std::fs::read(folder.join("AnimePahe_One_-_04_1080p_ADN.mp4.part"))
                .unwrap()
                .len(),
            100
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
