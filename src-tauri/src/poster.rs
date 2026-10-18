use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

const FETCH_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_BYTES: u64 = 8 * 1024 * 1024;
const MAX_CACHED: usize = 512;

const ACCEPT: &str = "image/avif,image/webp,image/png,image/jpeg,*/*;q=0.8";

/// The Resolver's browser identity: the Base URL for the Referer, plus the
/// User-Agent and cookies that carry its Cloudflare clearance.
pub struct Browser<'a> {
    pub base_url: &'a str,
    pub user_agent: &'a str,
    pub cookie: Option<&'a str>,
}

/// Fetches poster bytes and hands them to the UI as data URLs, cached by URL.
pub struct Posters {
    cache: Mutex<HashMap<String, String>>,
}

impl Posters {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The poster at `url` as a data URL, reused from the cache when possible.
    ///
    /// `proxy` is the local CONNECT proxy to tunnel through, or `None` for a
    /// direct fetch on a clean resolver.
    pub async fn data_url(
        &self,
        url: &str,
        browser: Browser<'_>,
        proxy: Option<&str>,
    ) -> Result<String, String> {
        if let Some(cached) = self.cached(url) {
            return Ok(cached);
        }

        let data_url = fetch(url, &browser, proxy).await?;
        self.remember(url, &data_url);
        Ok(data_url)
    }

    /// A cached poster, if this URL was already fetched.
    pub fn cached(&self, url: &str) -> Option<String> {
        self.cache.lock().ok()?.get(url).cloned()
    }

    fn remember(&self, url: &str, data_url: &str) {
        if let Ok(mut cache) = self.cache.lock() {
            // Poster URLs are finite per session; this only guards a very long
            // browsing run from holding every image in memory forever.
            if cache.len() >= MAX_CACHED {
                cache.clear();
            }
            cache.insert(url.to_owned(), data_url.to_owned());
        }
    }
}

impl Default for Posters {
    fn default() -> Self {
        Self::new()
    }
}

/// Turn an image response into the data URL the UI renders.
pub fn data_url_of(content_type: &str, bytes: &[u8]) -> Result<String, String> {
    let content_type = content_type.split(';').next().unwrap_or_default().trim();
    if !content_type.starts_with("image/") {
        return Err("the poster host did not answer with an image.".to_owned());
    }
    if bytes.len() as u64 > MAX_BYTES {
        return Err("the poster is too large to display.".to_owned());
    }

    Ok(format!(
        "data:{content_type};base64,{}",
        STANDARD.encode(bytes)
    ))
}

async fn fetch(url: &str, browser: &Browser<'_>, proxy: Option<&str>) -> Result<String, String> {
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .timeout(FETCH_TIMEOUT)
        .user_agent(browser.user_agent);
    if let Some(proxy) = proxy {
        builder = builder.proxy(
            reqwest::Proxy::all(proxy)
                .map_err(|error| format!("could not use the local proxy: {error}"))?,
        );
    }
    let client = builder
        .build()
        .map_err(|error| format!("could not prepare the poster request: {error}"))?;

    let mut request = client
        .get(url)
        .header(reqwest::header::REFERER, browser.base_url)
        .header(reqwest::header::ACCEPT, ACCEPT);
    if let Some(cookie) = browser.cookie {
        request = request.header(reqwest::header::COOKIE, cookie);
    }

    let response = request
        .send()
        .await
        .map_err(|error| format!("could not reach the poster host: {error}"))?;

    if !response.status().is_success() {
        return Err(format!("the poster host answered {}", response.status()));
    }
    if response.content_length().is_some_and(|len| len > MAX_BYTES) {
        return Err("the poster is too large to display.".to_owned());
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();

    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("could not read the poster: {error}"))?;

    data_url_of(&content_type, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const BASE: &str = "https://animepahe.pw";
    const IMAGE: &[u8] = b"\xff\xd8\xff\xe0not-really-a-jpeg";

    struct TestServer {
        base: String,
        requests: Arc<Mutex<Vec<String>>>,
    }

    impl TestServer {
        fn url(&self, path: &str) -> String {
            format!("{}{path}", self.base)
        }

        fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    /// A single-purpose HTTP server: records each request head and answers
    /// with the same status, content type, and body every time.
    async fn serve(status: &str, content_type: &str, body: &[u8]) -> TestServer {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        let status = status.to_owned();
        let content_type = content_type.to_owned();
        let body = body.to_vec();

        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let mut head = Vec::new();
                let mut chunk = [0u8; 1024];
                loop {
                    let read = socket.read(&mut chunk).await.unwrap_or(0);
                    if read == 0 {
                        break;
                    }
                    head.extend_from_slice(&chunk[..read]);
                    if head.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                recorded
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&head).into_owned());

                let reply = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(reply.as_bytes()).await;
                let _ = socket.write_all(&body).await;
                let _ = socket.shutdown().await;
            }
        });

        TestServer {
            base: format!("http://{address}"),
            requests,
        }
    }

    async fn dead_address() -> SocketAddr {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        listener.local_addr().unwrap()
    }

    const TEST_UA: &str = "Mozilla/5.0 pahe-test";

    fn browser() -> Browser<'static> {
        Browser {
            base_url: BASE,
            user_agent: TEST_UA,
            cookie: None,
        }
    }

    #[tokio::test]
    async fn fetches_a_poster_directly_on_a_clean_resolver() {
        let server = serve("200 OK", "image/jpeg", IMAGE).await;
        let posters = Posters::new();

        let data_url = posters
            .data_url(&server.url("/posters/one.jpg"), browser(), None)
            .await
            .unwrap();

        assert_eq!(
            data_url,
            format!("data:image/jpeg;base64,{}", STANDARD.encode(IMAGE))
        );
    }

    #[tokio::test]
    async fn sends_a_browser_user_agent_and_the_base_url_as_referer() {
        let server = serve("200 OK", "image/png", IMAGE).await;
        let posters = Posters::new();

        posters
            .data_url(&server.url("/posters/two.png"), browser(), None)
            .await
            .unwrap();

        let head = server.requests()[0].to_lowercase();
        assert!(head.starts_with("get /posters/two.png http/1.1"));
        assert!(head.contains(&format!("referer: {BASE}")));
        assert!(head.contains(&format!("user-agent: {}", TEST_UA.to_lowercase())));
    }

    #[tokio::test]
    async fn sends_the_resolver_clearance_cookie() {
        let server = serve("200 OK", "image/jpeg", IMAGE).await;
        let posters = Posters::new();

        posters
            .data_url(
                &server.url("/posters/one.jpg"),
                Browser {
                    base_url: BASE,
                    user_agent: TEST_UA,
                    cookie: Some("cf_clearance=abc"),
                },
                None,
            )
            .await
            .unwrap();

        assert!(
            server.requests()[0]
                .to_lowercase()
                .contains("cookie: cf_clearance=abc")
        );
    }

    #[tokio::test]
    async fn fetches_through_the_proxy_on_a_poisoned_resolver() {
        let proxy = serve("200 OK", "image/webp", IMAGE).await;
        let posters = Posters::new();

        // .invalid never resolves, so success proves the request used the proxy.
        let data_url = posters
            .data_url(
                "http://poster.invalid/posters/three.webp",
                browser(),
                Some(&proxy.base),
            )
            .await
            .unwrap();

        assert_eq!(
            data_url,
            format!("data:image/webp;base64,{}", STANDARD.encode(IMAGE))
        );
        let head = proxy.requests()[0].to_lowercase();
        assert!(head.starts_with("get http://poster.invalid/posters/three.webp"));
    }

    #[tokio::test]
    async fn reuses_the_cached_poster_for_the_same_url() {
        let server = serve("200 OK", "image/jpeg", IMAGE).await;
        let posters = Posters::new();
        let url = server.url("/posters/one.jpg");

        let first = posters.data_url(&url, browser(), None).await.unwrap();
        let second = posters.data_url(&url, browser(), None).await.unwrap();

        assert_eq!(first, second);
        assert_eq!(server.requests().len(), 1);
    }

    #[tokio::test]
    async fn a_refused_poster_is_an_error_and_is_not_cached() {
        let server = serve("500 Internal Server Error", "text/html", b"nope").await;
        let posters = Posters::new();
        let url = server.url("/posters/one.jpg");

        assert!(posters.data_url(&url, browser(), None).await.is_err());
        assert!(posters.data_url(&url, browser(), None).await.is_err());
        assert_eq!(server.requests().len(), 2);
    }

    #[tokio::test]
    async fn a_challenge_page_is_not_served_as_a_poster() {
        let server = serve("200 OK", "text/html", b"<html>Just a moment...</html>").await;
        let posters = Posters::new();

        assert!(
            posters
                .data_url(&server.url("/posters/one.jpg"), browser(), None)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn an_unreachable_poster_host_is_an_error() {
        let address = dead_address().await;
        let posters = Posters::new();
        let url = format!("http://{address}/posters/one.jpg");

        assert!(posters.data_url(&url, browser(), None).await.is_err());
    }

    #[test]
    fn only_image_responses_become_data_urls() {
        assert_eq!(
            data_url_of("image/jpeg; charset=binary", IMAGE).unwrap(),
            format!("data:image/jpeg;base64,{}", STANDARD.encode(IMAGE))
        );
        assert!(data_url_of("text/html", IMAGE).is_err());
        assert!(data_url_of("", IMAGE).is_err());
    }
}
