use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::utils::config::BackgroundThrottlingPolicy;
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tokio::sync::{Mutex, mpsc, watch};
use tokio::time::{Instant, sleep, timeout};
use url::Url;

use crate::poster::{self, Posters};
use crate::probe::{self, ProbeOutcome};
use crate::proxy::{self, DohLookup, ProxyHandle};

pub const RESOLVER_LABEL: &str = "resolver";
pub const STATUS_EVENT: &str = "resolver://status";

/// A stable data store keeps Cloudflare clearance across launches (macOS 14+).
const DATA_STORE: [u8; 16] = *b"pahe-resolver-01";
const CHALLENGE_TIMEOUT: Duration = Duration::from_secs(600);
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const PAGE_LOAD_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_FAILURES: u32 = 6;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub anime_id: String,
    pub title: String,
    pub year: Option<u16>,
    pub poster: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnimeDetail {
    pub synopsis: Option<String>,
    pub episodes: Vec<Episode>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Episode {
    pub number: u32,
    pub air_date: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ResolverFailureKind {
    Unreachable,
    ChallengeTimedOut,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolverFailure {
    pub kind: ResolverFailureKind,
    pub message: String,
}

impl ResolverFailure {
    fn failed(message: impl Into<String>) -> Self {
        Self {
            kind: ResolverFailureKind::Failed,
            message: message.into(),
        }
    }

    fn unreachable(base_url: &str) -> Self {
        Self {
            kind: ResolverFailureKind::Unreachable,
            message: format!("Could not reach {base_url}. Check your connection or the Base URL."),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ResolverPhase {
    Idle,
    Challenge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolverStatus {
    pub state: ResolverPhase,
}

struct Session {
    base_url: String,
}

/// Owns the one persistent Resolver webview and serializes all resolution work.
pub struct Resolver {
    app: AppHandle,
    session: Mutex<Session>,
    probe: Mutex<Option<(String, ProbeOutcome)>>,
    proxy: Mutex<Option<ProxyHandle>>,
    posters: Posters,
    user_agent: Mutex<Option<String>>,
}

impl Resolver {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            session: Mutex::new(Session {
                base_url: String::new(),
            }),
            probe: Mutex::new(None),
            proxy: Mutex::new(None),
            posters: Posters::new(),
            user_agent: Mutex::new(None),
        }
    }

    pub async fn search(
        &self,
        base_url: &str,
        query: &str,
    ) -> Result<Vec<SearchResult>, ResolverFailure> {
        let query = query.trim();
        if query.is_empty() {
            return Err(ResolverFailure::failed("Type a title to search."));
        }

        let mut session = self.session.lock().await;
        let mut rebuilt = false;

        loop {
            let window = self.ensure_window(&mut session, base_url).await?;
            match self.run_search(&window, base_url, query).await {
                Err(failure) if failure.kind == ResolverFailureKind::Unreachable && !rebuilt => {
                    // Repeated connection failures may mean the network changed;
                    // drop the cached probe and the window, then try once more.
                    rebuilt = true;
                    self.forget_probe().await;
                    self.destroy_window().await;
                }
                outcome => return outcome,
            }
        }
    }

    /// The synopsis and every Episode of one Anime. The release API pages are
    /// followed to `last_page`, so a long-running Anime lists all Episodes.
    pub async fn detail(
        &self,
        base_url: &str,
        anime_id: &str,
    ) -> Result<AnimeDetail, ResolverFailure> {
        let anime_id = anime_id.trim();
        if anime_id.is_empty() {
            return Err(ResolverFailure::failed(
                "Open an Anime from the search results.",
            ));
        }

        let mut session = self.session.lock().await;
        let mut rebuilt = false;

        loop {
            let window = self.ensure_window(&mut session, base_url).await?;
            match self.run_detail(&window, base_url, anime_id).await {
                Err(failure) if failure.kind == ResolverFailureKind::Unreachable && !rebuilt => {
                    rebuilt = true;
                    self.forget_probe().await;
                    self.destroy_window().await;
                }
                outcome => return outcome,
            }
        }
    }

    /// The poster at `url` as a data URL, fetched through the local proxy when
    /// the probe reports a Poisoned resolver and directly when it is Clean.
    ///
    /// The poster host answers a plain HTTP client with a Cloudflare Challenge,
    /// so the fetch borrows the Resolver's User-Agent and clearance cookies.
    pub async fn poster(&self, base_url: &str, url: &str) -> Result<String, String> {
        if let Some(cached) = self.posters.cached(url) {
            return Ok(cached);
        }

        let proxy = match self.probe_outcome(base_url).await {
            ProbeOutcome::Poisoned => Some(
                self.ensure_proxy()
                    .await
                    .map_err(|failure| failure.message)?,
            ),
            _ => None,
        };

        let window = {
            let mut session = self.session.lock().await;
            self.ensure_window(&mut session, base_url)
                .await
                .map_err(|failure| failure.message)?
        };
        let user_agent = self.user_agent(&window).await?;
        // The poster host is a subdomain of the Base URL, so clearance lives on
        // the parent domain. Wry's cookies_for_url compares domains exactly and
        // misses those, so read the store and match the domain ourselves.
        let cookie = cookie_header(&window, url)?;

        self.posters
            .data_url(
                url,
                poster::Browser {
                    base_url,
                    user_agent: &user_agent,
                    cookie: cookie.as_deref(),
                },
                proxy.as_deref(),
            )
            .await
    }

    async fn user_agent(&self, window: &WebviewWindow) -> Result<String, String> {
        if let Some(user_agent) = self.user_agent.lock().await.clone() {
            return Ok(user_agent);
        }

        window
            .eval("window.__paheUa = navigator.userAgent")
            .map_err(|error| error.to_string())?;

        let deadline = Instant::now() + Duration::from_secs(2);
        let poll = "JSON.stringify(window.__paheUa || null)";

        while Instant::now() < deadline {
            let (tx, mut rx) = mpsc::unbounded_channel();
            window
                .eval_with_callback(poll, move |value| {
                    let _ = tx.send(value);
                })
                .map_err(|error| error.to_string())?;

            if let Ok(Some(raw)) = timeout(Duration::from_millis(250), rx.recv()).await
                && let Some(user_agent) = decode_user_agent(&raw)
            {
                *self.user_agent.lock().await = Some(user_agent.clone());
                return Ok(user_agent);
            }
            sleep(Duration::from_millis(100)).await;
        }

        Err("The Resolver did not report its User-Agent.".to_owned())
    }

    /// Called when the Base URL changes: clears the probe cache, the Resolver
    /// window, and its browsing data so stale clearance does not leak across
    /// origins.
    pub async fn base_url_changed(&self, base_url: &str) {
        let mut session = self.session.lock().await;
        if session.base_url == base_url {
            return;
        }
        if let Some(window) = self.app.get_webview_window(RESOLVER_LABEL) {
            let _ = window.clear_all_browsing_data();
        }
        session.base_url.clear();
        self.forget_probe().await;
        self.destroy_window().await;
    }

    async fn forget_probe(&self) {
        *self.probe.lock().await = None;
    }

    async fn destroy_window(&self) {
        if let Some(window) = self.app.get_webview_window(RESOLVER_LABEL) {
            let _ = window.destroy();
        }
    }

    async fn ensure_window(
        &self,
        session: &mut Session,
        base_url: &str,
    ) -> Result<WebviewWindow, ResolverFailure> {
        if session.base_url == base_url {
            if let Some(window) = self.app.get_webview_window(RESOLVER_LABEL) {
                return Ok(window);
            }
        } else {
            self.destroy_window().await;
            session.base_url = base_url.to_owned();
        }

        let outcome = self.probe_outcome(base_url).await;
        if outcome == ProbeOutcome::Unreachable {
            return Err(ResolverFailure::unreachable(base_url));
        }

        let proxy = match outcome {
            ProbeOutcome::Poisoned => Some(self.ensure_proxy().await?),
            _ => None,
        };
        let url = Url::parse(base_url)
            .map_err(|_| ResolverFailure::failed("The Base URL is not a valid URL."))?;

        let (ready_tx, mut ready_rx) = watch::channel(false);
        let mut builder =
            WebviewWindowBuilder::new(&self.app, RESOLVER_LABEL, WebviewUrl::External(url))
                .title("animepahe")
                .inner_size(1000.0, 760.0)
                .center()
                .visible(false)
                .skip_taskbar(true)
                .background_throttling(BackgroundThrottlingPolicy::Disabled)
                .data_store_identifier(DATA_STORE)
                .on_page_load(move |_window, payload| {
                    if payload.event() == PageLoadEvent::Finished {
                        let _ = ready_tx.send(true);
                    }
                });

        if let Some(proxy) = proxy {
            let proxy_url = Url::parse(&proxy)
                .map_err(|_| ResolverFailure::failed("Could not start the local proxy."))?;
            builder = builder.proxy_url(proxy_url);
        }

        let window = builder.build().map_err(|error| {
            ResolverFailure::failed(format!("Could not open the Resolver: {error}"))
        })?;

        // Best effort: an early script is retried by the search loop anyway.
        let _ = timeout(PAGE_LOAD_TIMEOUT, ready_rx.wait_for(|ready| *ready)).await;
        Ok(window)
    }

    async fn probe_outcome(&self, base_url: &str) -> ProbeOutcome {
        let mut cached = self.probe.lock().await;
        if let Some((url, outcome)) = cached.as_ref()
            && url == base_url
        {
            return *outcome;
        }

        let outcome = probe::check(base_url).await;
        // Unreachable is transient: let the next search probe again.
        if outcome != ProbeOutcome::Unreachable {
            *cached = Some((base_url.to_owned(), outcome));
        }
        outcome
    }

    async fn ensure_proxy(&self) -> Result<String, ResolverFailure> {
        let mut proxy = self.proxy.lock().await;
        if let Some(handle) = proxy.as_ref() {
            return Ok(handle.url.clone());
        }

        let handle = proxy::start(Arc::new(DohLookup::cloudflare()))
            .await
            .map_err(|error| {
                ResolverFailure::failed(format!("Could not start the local proxy: {error}"))
            })?;
        let url = handle.url.clone();
        *proxy = Some(handle);
        Ok(url)
    }

    async fn run_search(
        &self,
        window: &WebviewWindow,
        base_url: &str,
        query: &str,
    ) -> Result<Vec<SearchResult>, ResolverFailure> {
        let url = search_url(base_url, query)?;
        let result = self
            .fetch_page(window, base_url, &url, "application/json")
            .await?;
        parse_search(&result.body, base_url)
    }

    async fn run_detail(
        &self,
        window: &WebviewWindow,
        base_url: &str,
        anime_id: &str,
    ) -> Result<AnimeDetail, ResolverFailure> {
        let episodes =
            collect_releases(|page| self.fetch_release_page(window, base_url, anime_id, page))
                .await?;
        let synopsis = self.anime_synopsis(window, base_url, anime_id).await;

        Ok(AnimeDetail { synopsis, episodes })
    }

    async fn fetch_release_page(
        &self,
        window: &WebviewWindow,
        base_url: &str,
        anime_id: &str,
        page: u32,
    ) -> Result<ReleasePage, ResolverFailure> {
        let url = release_url(base_url, anime_id, page)?;
        let result = self
            .fetch_page(window, base_url, &url, "application/json")
            .await?;
        parse_release_page(&result.body)
    }

    /// The Anime page's synopsis, or None when it cannot be read. It is
    /// optional, so one attempt is made and no Challenge is surfaced for it.
    async fn anime_synopsis(
        &self,
        window: &WebviewWindow,
        base_url: &str,
        anime_id: &str,
    ) -> Option<String> {
        let url = anime_url(base_url, anime_id).ok()?;
        let result = self.fetch_once(window, &url, "text/html").await.ok()?;
        if !(200..300).contains(&result.status) {
            return None;
        }
        parse_synopsis(&result.body)
    }

    /// Fetches `url` in the Resolver, walking the user through a Challenge and
    /// retrying after transient connection failures.
    async fn fetch_page(
        &self,
        window: &WebviewWindow,
        base_url: &str,
        url: &Url,
        accept: &str,
    ) -> Result<FetchResult, ResolverFailure> {
        let mut surfaced = false;
        let mut failures = 0;
        let mut challenge_deadline = None;

        let outcome = loop {
            match self.fetch_once(window, url, accept).await {
                Ok(result) if is_challenge(&result) => {
                    if !surfaced {
                        surfaced = true;
                        let _ = window.show();
                        let _ = window.unminimize();
                        let _ = window.set_focus();
                        let _ = self.app.emit(
                            STATUS_EVENT,
                            ResolverStatus {
                                state: ResolverPhase::Challenge,
                            },
                        );
                    }

                    let deadline = *challenge_deadline
                        .get_or_insert_with(|| Instant::now() + CHALLENGE_TIMEOUT);
                    if Instant::now() >= deadline {
                        break Err(ResolverFailure {
                            kind: ResolverFailureKind::ChallengeTimedOut,
                            message: "Cloudflare asked for a human check and the Resolver window was not cleared in time. Try again.".to_owned(),
                        });
                    }
                    sleep(Duration::from_secs(2)).await;
                }
                Ok(result) if (200..300).contains(&result.status) => break Ok(result),
                Ok(_) | Err(_) => {
                    failures += 1;
                    if failures >= MAX_FAILURES {
                        break Err(ResolverFailure::unreachable(base_url));
                    }
                    sleep(Duration::from_secs(1)).await;
                }
            }
        };

        if surfaced {
            let _ = window.hide();
            let _ = self.app.emit(
                STATUS_EVENT,
                ResolverStatus {
                    state: ResolverPhase::Idle,
                },
            );
        }

        outcome
    }

    async fn fetch_once(
        &self,
        window: &WebviewWindow,
        url: &Url,
        accept: &str,
    ) -> Result<FetchResult, String> {
        let id = format!(
            "fetch-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        window
            .eval(fetch_script(url.as_str(), &id, accept))
            .map_err(|error| error.to_string())?;

        let deadline = Instant::now() + FETCH_TIMEOUT;
        let poll = "JSON.stringify(window.__paheFetch || null)";

        while Instant::now() < deadline {
            let (tx, mut rx) = mpsc::unbounded_channel();
            window
                .eval_with_callback(poll, move |value| {
                    let _ = tx.send(value);
                })
                .map_err(|error| error.to_string())?;

            if let Ok(Some(raw)) = timeout(Duration::from_millis(250), rx.recv()).await
                && let Some(result) = decode_poll(&raw, &id)
            {
                if let Some(error) = result.error {
                    return Err(error);
                }
                return Ok(result);
            }
            sleep(Duration::from_millis(100)).await;
        }

        Err("The Resolver did not answer in time.".to_owned())
    }
}

fn search_url(base_url: &str, query: &str) -> Result<Url, ResolverFailure> {
    let mut url = Url::parse(base_url)
        .map_err(|_| ResolverFailure::failed("The Base URL is not a valid URL."))?;
    url.set_path("/api");
    url.query_pairs_mut()
        .clear()
        .append_pair("m", "search")
        .append_pair("q", query);
    Ok(url)
}

fn anime_url(base_url: &str, anime_id: &str) -> Result<Url, ResolverFailure> {
    let mut url = Url::parse(base_url)
        .map_err(|_| ResolverFailure::failed("The Base URL is not a valid URL."))?;
    url.set_path("/anime");
    url.path_segments_mut()
        .map_err(|_| ResolverFailure::failed("The Base URL cannot hold an Anime path."))?
        .push(anime_id);
    Ok(url)
}

fn release_url(base_url: &str, anime_id: &str, page: u32) -> Result<Url, ResolverFailure> {
    let mut url = Url::parse(base_url)
        .map_err(|_| ResolverFailure::failed("The Base URL is not a valid URL."))?;
    url.set_path("/api");
    url.query_pairs_mut()
        .clear()
        .append_pair("m", "release")
        .append_pair("id", anime_id)
        .append_pair("sort", "episode_asc")
        .append_pair("page", &page.to_string());
    Ok(url)
}

fn fetch_script(url: &str, id: &str, accept: &str) -> String {
    let url = serde_json::to_string(url).expect("URL is valid JSON");
    let id = serde_json::to_string(id).expect("id is valid JSON");
    let accept = serde_json::to_string(accept).expect("accept is valid JSON");

    format!(
        r#"window.__paheFetch = {{ id: {id}, result: null }};
fetch({url}, {{ credentials: "include", cache: "no-store", headers: {{ accept: {accept} }} }})
  .then(async (response) => {{
    const body = await response.text();
    window.__paheFetch = {{ id: {id}, result: {{ status: response.status, contentType: response.headers.get("content-type") || "", mitigated: response.headers.get("cf-mitigated") || "", body }} }};
  }})
  .catch((error) => {{
    window.__paheFetch = {{ id: {id}, result: {{ status: 0, contentType: "", mitigated: "", body: "", error: String(error) }} }};
  }});"#
    )
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct FetchResult {
    status: u16,
    content_type: String,
    mitigated: String,
    body: String,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct FetchEnvelope {
    id: String,
    result: Option<FetchResult>,
}

fn decode_poll(raw: &str, id: &str) -> Option<FetchResult> {
    let inner = serde_json::from_str::<String>(raw.trim()).ok()?;
    if inner == "null" {
        return None;
    }

    let envelope = serde_json::from_str::<FetchEnvelope>(&inner).ok()?;
    if envelope.id != id {
        return None;
    }
    envelope.result
}

fn is_challenge(result: &FetchResult) -> bool {
    result.mitigated.eq_ignore_ascii_case("challenge")
        || (result.content_type.contains("text/html")
            && (result.body.contains("Just a moment")
                || result.body.contains("challenge-platform")))
}

/// The cookies the Resolver holds for the poster URL, as a Cookie header.
///
/// Wry's `cookies_for_url` requires the cookie domain to equal the URL host,
/// which drops the `animepahe.pw` clearance cookie for `i.animepahe.pw`, so the
/// full jar is filtered here instead.
fn cookie_header(window: &WebviewWindow, url: &str) -> Result<Option<String>, String> {
    let url = Url::parse(url).map_err(|_| "The poster URL is not valid.".to_owned())?;
    let host = url.host_str().unwrap_or_default();

    let cookies = window
        .cookies()
        .map_err(|error| format!("could not read the Resolver cookies: {error}"))?;

    let header = cookies
        .iter()
        .filter(|cookie| domain_matches(cookie.domain(), host))
        .filter(|cookie| url.scheme() == "https" || !cookie.secure().unwrap_or(false))
        .filter(|cookie| url.path().starts_with(cookie.path().unwrap_or("/")))
        .map(|cookie| format!("{}={}", cookie.name(), cookie.value()))
        .collect::<Vec<_>>()
        .join("; ");

    Ok((!header.is_empty()).then_some(header))
}

fn domain_matches(domain: Option<&str>, host: &str) -> bool {
    let Some(domain) = domain else {
        return false;
    };
    let domain = domain.trim_start_matches('.');

    !domain.is_empty() && (host == domain || host.ends_with(&format!(".{domain}")))
}

fn decode_user_agent(raw: &str) -> Option<String> {
    let inner = serde_json::from_str::<String>(raw.trim()).ok()?;
    if inner == "null" {
        return None;
    }
    serde_json::from_str::<String>(&inner).ok()
}

pub fn parse_search(body: &str, base_url: &str) -> Result<Vec<SearchResult>, ResolverFailure> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| ResolverFailure::failed("animepahe sent an unexpected search response."))?;
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| ResolverFailure::failed("animepahe sent an unexpected search response."))?;
    let base = Url::parse(base_url).ok();

    Ok(data
        .iter()
        .filter_map(|item| {
            let title = item.get("title")?.as_str()?.trim();
            if title.is_empty() {
                return None;
            }
            let anime_id = item
                .get("session")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| item.get("id").and_then(as_text))?;
            let year = item
                .get("year")
                .and_then(Value::as_u64)
                .and_then(|year| u16::try_from(year).ok());
            let poster = item
                .get("poster")
                .and_then(Value::as_str)
                .and_then(|poster| absolute_url(base.as_ref(), poster));

            Some(SearchResult {
                anime_id,
                title: title.to_owned(),
                year,
                poster,
            })
        })
        .collect())
}

/// One page of the release API: the parsed Episodes plus its `last_page`.
struct ReleasePage {
    episodes: Vec<Episode>,
    last_page: u32,
}

/// Animepahe paginates releases 30 per page. Every page up to `last_page` is
/// fetched so a long-running Anime lists all of its Episodes.
const MAX_RELEASE_PAGES: u32 = 200;

async fn collect_releases<F, Fut>(mut fetch: F) -> Result<Vec<Episode>, ResolverFailure>
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<ReleasePage, ResolverFailure>>,
{
    let mut episodes = Vec::new();
    let mut page = 1;

    loop {
        let fetched = fetch(page).await?;
        episodes.extend(fetched.episodes);

        if page >= fetched.last_page {
            return Ok(episodes);
        }
        if page >= MAX_RELEASE_PAGES {
            return Err(ResolverFailure::failed(
                "animepahe reported more release pages than pahe will load.",
            ));
        }
        page += 1;
    }
}

fn parse_release_page(body: &str) -> Result<ReleasePage, ResolverFailure> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| ResolverFailure::failed("animepahe sent an unexpected release response."))?;

    let last_page = value
        .get("last_page")
        .and_then(Value::as_u64)
        .and_then(|page| u32::try_from(page).ok())
        .unwrap_or(1)
        .max(1);

    let empty = Vec::new();
    let rows = match value.get("data") {
        Some(Value::Array(rows)) => rows,
        None => &empty,
        Some(_) => {
            return Err(ResolverFailure::failed(
                "animepahe sent an unexpected release response.",
            ));
        }
    };

    let episodes = rows
        .iter()
        .filter_map(|row| {
            Some(Episode {
                number: u32::try_from(row.get("episode")?.as_u64()?).ok()?,
                air_date: air_date(row.get("created_at")),
            })
        })
        .collect();

    Ok(ReleasePage {
        episodes,
        last_page,
    })
}

/// The release API's `created_at` is the only date it carries; its date part is
/// the air date. Missing or malformed dates stay absent rather than guessed.
fn air_date(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    let date = text.split_whitespace().next()?;
    let bytes = date.as_bytes();

    let shaped = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit());

    shaped.then(|| date.to_owned())
}

/// The Anime page's `<meta name="description">` holds the synopsis.
fn parse_synopsis(html: &str) -> Option<String> {
    let text = decode_entities(meta_content(html, "description")?.trim());

    (!text.is_empty()).then_some(text)
}

fn meta_content(html: &str, name: &str) -> Option<String> {
    let mut rest = html;

    while let Some(start) = rest.find("<meta") {
        let tag = &rest[start..];
        let end = tag.find('>')?;
        let attributes = &tag[..end];

        if attribute(attributes, "name").as_deref() == Some(name) {
            return attribute(attributes, "content");
        }
        rest = &tag[end + 1..];
    }
    None
}

/// Reads one quoted attribute from a tag, tolerating either attribute order.
fn attribute(tag: &str, key: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let pattern = format!("{key}=");

    for (start, _) in lower.match_indices(&pattern) {
        let before = lower[..start].chars().next_back();
        if before.is_some_and(|ch| !ch.is_whitespace()) {
            continue;
        }

        let value = tag[start + pattern.len()..].trim_start();
        let mut chars = value.chars();
        let quote = chars.next()?;
        if quote != '"' && quote != '\'' {
            continue;
        }
        let value = chars.as_str();
        let end = value.find(quote)?;
        return Some(value[..end].to_owned());
    }
    None
}

fn decode_entities(value: &str) -> String {
    let mut decoded = String::with_capacity(value.len());
    let mut rest = value;

    while let Some(index) = rest.find('&') {
        decoded.push_str(&rest[..index]);
        let entity = &rest[index + 1..];

        let Some(end) = entity.find(';') else {
            decoded.push_str(rest);
            return decoded;
        };

        match decode_entity(&entity[..end]) {
            Some(ch) => {
                decoded.push(ch);
                rest = &entity[end + 1..];
            }
            None => {
                decoded.push('&');
                rest = entity;
            }
        }
    }

    decoded.push_str(rest);
    decoded
}

fn decode_entity(entity: &str) -> Option<char> {
    let named = match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => None,
    };
    if named.is_some() {
        return named;
    }

    let number = entity.strip_prefix('#')?;
    let code = match number.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => number.parse().ok()?,
    };
    char::from_u32(code)
}

fn as_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn absolute_url(base: Option<&Url>, value: &str) -> Option<String> {
    if let Ok(absolute) = Url::parse(value) {
        return Some(absolute.to_string());
    }
    base?.join(value).ok().map(|url| url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://animepahe.pw";

    fn fixture() -> &'static str {
        r#"{
            "total": 2,
            "current_page": 1,
            "data": [
                {
                    "id": 6556,
                    "title": "Seitokai ni mo Ana wa Aru!",
                    "year": 2025,
                    "session": "0b9c9a2e-6d1f-4b3c-9c4e-1a2b3c4d5e6f",
                    "poster": "https://i.animepahe.si/posters/example.jpg"
                },
                {
                    "id": 12,
                    "title": "Another Anime",
                    "year": 2024,
                    "poster": "/posters/relative.jpg"
                }
            ]
        }"#
    }

    #[test]
    fn maps_search_results_to_posters_titles_and_years() {
        let results = parse_search(fixture(), BASE).unwrap();

        assert_eq!(
            results[0],
            SearchResult {
                anime_id: "0b9c9a2e-6d1f-4b3c-9c4e-1a2b3c4d5e6f".to_owned(),
                title: "Seitokai ni mo Ana wa Aru!".to_owned(),
                year: Some(2025),
                poster: Some("https://i.animepahe.si/posters/example.jpg".to_owned()),
            }
        );
        assert_eq!(results[1].anime_id, "12");
        assert_eq!(
            results[1].poster.as_deref(),
            Some("https://animepahe.pw/posters/relative.jpg")
        );
    }

    #[test]
    fn an_empty_result_set_is_not_an_error() {
        let body = r#"{"total": 0, "data": []}"#;

        assert!(parse_search(body, BASE).unwrap().is_empty());
    }

    #[test]
    fn rejects_a_response_without_a_data_array() {
        assert!(parse_search(r#"{"error": "nope"}"#, BASE).is_err());
        assert!(parse_search("<html>Just a moment...</html>", BASE).is_err());
    }

    #[test]
    fn search_url_encodes_the_query() {
        let url = search_url(BASE, "seitokai ni mo ana & more").unwrap();

        assert_eq!(url.path(), "/api");
        assert_eq!(url.query(), Some("m=search&q=seitokai+ni+mo+ana+%26+more"));
    }

    #[test]
    fn detects_a_cloudflare_challenge_page() {
        let challenge = FetchResult {
            status: 403,
            content_type: "text/html; charset=UTF-8".to_owned(),
            mitigated: "challenge".to_owned(),
            body: "<html><title>Just a moment...</title></html>".to_owned(),
            error: None,
        };
        let answer = FetchResult {
            status: 200,
            content_type: "application/json".to_owned(),
            mitigated: String::new(),
            body: fixture().to_owned(),
            error: None,
        };

        assert!(is_challenge(&challenge));
        assert!(!is_challenge(&answer));
    }

    #[test]
    fn ignores_poll_results_from_an_older_request() {
        let raw = serde_json::to_string(
            &serde_json::to_string(&FetchEnvelope {
                id: "old".to_owned(),
                result: None,
            })
            .unwrap(),
        )
        .unwrap();

        assert!(decode_poll(&raw, "new").is_none());
        assert!(decode_poll("null", "new").is_none());
    }

    #[test]
    fn reads_the_browser_user_agent_from_an_eval_callback() {
        let raw =
            serde_json::to_string(&serde_json::to_string("Mozilla/5.0 (Macintosh) pahe").unwrap())
                .unwrap();

        assert_eq!(
            decode_user_agent(&raw).as_deref(),
            Some("Mozilla/5.0 (Macintosh) pahe")
        );
        assert!(decode_user_agent("null").is_none());
        assert!(decode_user_agent("not json").is_none());
    }

    #[test]
    fn matches_a_cookie_domain_to_its_subdomains() {
        assert!(domain_matches(Some("animepahe.pw"), "i.animepahe.pw"));
        assert!(domain_matches(Some(".animepahe.pw"), "i.animepahe.pw"));
        assert!(domain_matches(Some("i.animepahe.pw"), "i.animepahe.pw"));
        assert!(!domain_matches(Some("animepahe.pw"), "notanimepahe.pw"));
        assert!(!domain_matches(Some("other.example"), "i.animepahe.pw"));
        assert!(!domain_matches(None, "i.animepahe.pw"));
    }

    const RELEASE_PAGE: &str = r#"{
        "total": 175,
        "per_page": 30,
        "current_page": 1,
        "last_page": 6,
        "data": [
            {
                "id": 4387,
                "episode": 1,
                "session": "1c600e41110dd17d00bc96db2208bf0e6c1c044d528246ac229d80f5aa480170",
                "created_at": "2018-06-11 16:14:41"
            },
            {
                "id": 4388,
                "episode": 2,
                "session": "f0e1d2c3b4a5968778695a4b3c2d1e0f",
                "created_at": null
            },
            {
                "id": 4389,
                "episode": 3,
                "created_at": "not a date"
            }
        ]
    }"#;

    fn episode(number: u32) -> Episode {
        Episode {
            number,
            air_date: None,
        }
    }

    #[test]
    fn parses_a_release_page_with_episode_numbers_and_air_dates() {
        let page = parse_release_page(RELEASE_PAGE).unwrap();

        assert_eq!(page.last_page, 6);
        assert_eq!(
            page.episodes,
            vec![
                Episode {
                    number: 1,
                    air_date: Some("2018-06-11".to_owned()),
                },
                Episode {
                    number: 2,
                    air_date: None,
                },
                Episode {
                    number: 3,
                    air_date: None,
                },
            ]
        );
    }

    #[test]
    fn an_empty_release_response_lists_no_episodes() {
        let page = parse_release_page(r#"{"message": ""}"#).unwrap();

        assert_eq!(page.last_page, 1);
        assert!(page.episodes.is_empty());
    }

    #[test]
    fn rejects_a_release_response_that_is_not_a_page() {
        assert!(parse_release_page("<html>Just a moment...</html>").is_err());
        assert!(parse_release_page(r#"{"data": {}}"#).is_err());
    }

    #[tokio::test]
    async fn follows_every_release_page_in_order() {
        let requested = std::cell::RefCell::new(Vec::new());

        let episodes = collect_releases(|page| {
            requested.borrow_mut().push(page);
            let (episodes, last_page) = match page {
                1 => (vec![episode(1), episode(2)], 3),
                2 => (vec![episode(3)], 3),
                _ => (vec![episode(4), episode(5)], 3),
            };
            async move {
                Ok(ReleasePage {
                    episodes,
                    last_page,
                })
            }
        })
        .await
        .unwrap();

        assert_eq!(
            episodes.iter().map(|item| item.number).collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5]
        );
        assert_eq!(*requested.borrow(), vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn an_empty_release_page_lists_no_episodes() {
        let episodes = collect_releases(|_| async {
            Ok(ReleasePage {
                episodes: Vec::new(),
                last_page: 1,
            })
        })
        .await
        .unwrap();

        assert!(episodes.is_empty());
    }

    #[tokio::test]
    async fn a_failed_release_page_fails_the_whole_list() {
        let result = collect_releases(|page| async move {
            if page == 2 {
                Err(ResolverFailure::failed("page 2 failed"))
            } else {
                Ok(ReleasePage {
                    episodes: vec![episode(1)],
                    last_page: 3,
                })
            }
        })
        .await;

        assert_eq!(result.unwrap_err().kind, ResolverFailureKind::Failed);
    }

    #[test]
    fn release_url_asks_for_ascending_episode_pages() {
        let url = release_url(BASE, "7da6a559-70e7-5ce1-4f29-3732c1fea142", 2).unwrap();

        assert_eq!(url.path(), "/api");
        assert_eq!(
            url.query(),
            Some("m=release&id=7da6a559-70e7-5ce1-4f29-3732c1fea142&sort=episode_asc&page=2")
        );
    }

    #[test]
    fn anime_url_points_at_the_anime_page() {
        let url = anime_url(BASE, "7da6a559-70e7-5ce1-4f29-3732c1fea142").unwrap();

        assert_eq!(
            url.as_str(),
            "https://animepahe.pw/anime/7da6a559-70e7-5ce1-4f29-3732c1fea142"
        );
    }

    #[test]
    fn reads_the_synopsis_from_the_description_meta_tag() {
        let html = r#"<html><head>
            <meta property="og:description" content="not this one">
            <meta name="description" content="In the enchanted Kingdom of Fiore, the &quot;Salamander&quot; &amp; friends &#39;live&#39; here.">
        </head></html>"#;

        assert_eq!(
            parse_synopsis(html).as_deref(),
            Some("In the enchanted Kingdom of Fiore, the \"Salamander\" & friends 'live' here.")
        );
    }

    #[test]
    fn reads_a_description_tag_with_the_content_attribute_first() {
        let html = r#"<meta content="Folded &amp; simple" name="description">"#;

        assert_eq!(parse_synopsis(html).as_deref(), Some("Folded & simple"));
    }

    #[test]
    fn an_anime_page_without_a_description_has_no_synopsis() {
        assert!(parse_synopsis(r#"<meta property="og:description" content="nope">"#).is_none());
        assert!(parse_synopsis(r#"<meta name="description" content="  ">"#).is_none());
        assert!(parse_synopsis("<html></html>").is_none());
    }

    #[test]
    fn a_date_that_is_not_an_air_date_stays_absent() {
        assert_eq!(
            air_date(Some(&Value::String("2018-06-11 16:14:41".to_owned()))).as_deref(),
            Some("2018-06-11")
        );
        assert!(air_date(Some(&Value::String("2018/06/11".to_owned()))).is_none());
        assert!(air_date(Some(&Value::String(String::new()))).is_none());
        assert!(air_date(Some(&Value::Null)).is_none());
        assert!(air_date(None).is_none());
    }
}
