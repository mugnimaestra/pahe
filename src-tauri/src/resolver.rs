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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchFailureKind {
    Unreachable,
    ChallengeTimedOut,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchFailure {
    pub kind: SearchFailureKind,
    pub message: String,
}

impl SearchFailure {
    fn failed(message: impl Into<String>) -> Self {
        Self {
            kind: SearchFailureKind::Failed,
            message: message.into(),
        }
    }

    fn unreachable(base_url: &str) -> Self {
        Self {
            kind: SearchFailureKind::Unreachable,
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
        }
    }

    pub async fn search(
        &self,
        base_url: &str,
        query: &str,
    ) -> Result<Vec<SearchResult>, SearchFailure> {
        let query = query.trim();
        if query.is_empty() {
            return Err(SearchFailure::failed("Type a title to search."));
        }

        let mut session = self.session.lock().await;
        let mut rebuilt = false;

        loop {
            let window = self.ensure_window(&mut session, base_url).await?;
            match self.run_search(&window, base_url, query).await {
                Err(failure) if failure.kind == SearchFailureKind::Unreachable && !rebuilt => {
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
    ) -> Result<WebviewWindow, SearchFailure> {
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
            return Err(SearchFailure::unreachable(base_url));
        }

        let proxy = match outcome {
            ProbeOutcome::Poisoned => Some(self.ensure_proxy().await?),
            _ => None,
        };
        let url = Url::parse(base_url)
            .map_err(|_| SearchFailure::failed("The Base URL is not a valid URL."))?;

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
                .map_err(|_| SearchFailure::failed("Could not start the local proxy."))?;
            builder = builder.proxy_url(proxy_url);
        }

        let window = builder.build().map_err(|error| {
            SearchFailure::failed(format!("Could not open the Resolver: {error}"))
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

    async fn ensure_proxy(&self) -> Result<String, SearchFailure> {
        let mut proxy = self.proxy.lock().await;
        if let Some(handle) = proxy.as_ref() {
            return Ok(handle.url.clone());
        }

        let handle = proxy::start(Arc::new(DohLookup::cloudflare()))
            .await
            .map_err(|error| {
                SearchFailure::failed(format!("Could not start the local proxy: {error}"))
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
    ) -> Result<Vec<SearchResult>, SearchFailure> {
        let url = search_url(base_url, query)?;
        let mut surfaced = false;
        let mut failures = 0;
        let mut challenge_deadline = None;
        let mut attempt = 0;

        let outcome = loop {
            attempt += 1;
            let fetched = self.fetch_once(window, &url, attempt).await;

            match fetched {
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
                        break Err(SearchFailure {
                            kind: SearchFailureKind::ChallengeTimedOut,
                            message: "Cloudflare asked for a human check and the Resolver window was not cleared in time. Try the search again.".to_owned(),
                        });
                    }
                    sleep(Duration::from_secs(2)).await;
                }
                Ok(result) if (200..300).contains(&result.status) => {
                    break parse_search(&result.body, base_url);
                }
                Ok(_) | Err(_) => {
                    failures += 1;
                    if failures >= MAX_FAILURES {
                        break Err(SearchFailure::unreachable(base_url));
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
        attempt: u32,
    ) -> Result<FetchResult, String> {
        let id = format!(
            "search-{attempt}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        window
            .eval(fetch_script(url.as_str(), &id))
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

fn search_url(base_url: &str, query: &str) -> Result<Url, SearchFailure> {
    let mut url = Url::parse(base_url)
        .map_err(|_| SearchFailure::failed("The Base URL is not a valid URL."))?;
    url.set_path("/api");
    url.query_pairs_mut()
        .clear()
        .append_pair("m", "search")
        .append_pair("q", query);
    Ok(url)
}

fn fetch_script(url: &str, id: &str) -> String {
    let url = serde_json::to_string(url).expect("URL is valid JSON");
    let id = serde_json::to_string(id).expect("id is valid JSON");

    format!(
        r#"window.__paheFetch = {{ id: {id}, result: null }};
fetch({url}, {{ credentials: "include", cache: "no-store", headers: {{ accept: "application/json" }} }})
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

pub fn parse_search(body: &str, base_url: &str) -> Result<Vec<SearchResult>, SearchFailure> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| SearchFailure::failed("animepahe sent an unexpected search response."))?;
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| SearchFailure::failed("animepahe sent an unexpected search response."))?;
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
}
