//! A loopback relay that lets the player's `<video>` play a CDN URL.
//!
//! The CDN wants kwik's Referer and a browser User-Agent, and a `<video>`
//! element cannot send either. The relay adds them and passes `Range`
//! through, so the webview can seek before the Episode is downloaded.

use std::collections::HashMap;
use std::hash::{BuildHasher, RandomState};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

use crate::download::{REFERER, USER_AGENT};
use crate::proxy::read_head;

const NOT_FOUND: &[u8] =
    b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
const BAD_GATEWAY: &[u8] =
    b"HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
const METHOD_NOT_ALLOWED: &[u8] = b"HTTP/1.1 405 Method Not Allowed\r\nallow: GET, HEAD\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";

/// The CDN headers the webview needs to see to play and seek.
const PASSED_HEADERS: [&str; 4] = [
    "content-length",
    "content-range",
    "accept-ranges",
    "last-modified",
];

/// Registered CDN URLs behind tokens on a loopback port.
///
/// ponytail: a token lives until the app exits, one per Stream started. They
/// are a few hundred bytes each; expire them by age if that ever matters.
pub struct Relay {
    port: u16,
    sources: Arc<Mutex<HashMap<String, String>>>,
}

impl Relay {
    /// Binds an ephemeral loopback port. The accept loop runs detached for the
    /// life of the process.
    pub async fn start() -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let sources = Arc::new(Mutex::new(HashMap::new()));
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .no_proxy()
            .build()
            .map_err(io::Error::other)?;

        tokio::spawn({
            let sources = Arc::clone(&sources);
            async move {
                loop {
                    match listener.accept().await {
                        Ok((viewer, _)) => {
                            let sources = Arc::clone(&sources);
                            let client = client.clone();
                            tokio::spawn(async move {
                                let _ = serve(viewer, &sources, &client).await;
                            });
                        }
                        Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
                    }
                }
            }
        });

        Ok(Self { port, sources })
    }

    /// Puts `cdn_url` behind a token that cannot be guessed and returns the
    /// loopback URL the player loads.
    pub fn register(&self, cdn_url: String) -> String {
        let token = format!("{:016x}", RandomState::new().hash_one(&cdn_url));
        self.sources
            .lock()
            .expect("relay lock poisoned")
            .insert(token.clone(), cdn_url);

        format!("http://127.0.0.1:{}/{token}", self.port)
    }
}

async fn serve(
    mut viewer: TcpStream,
    sources: &Mutex<HashMap<String, String>>,
    client: &reqwest::Client,
) -> io::Result<()> {
    let Some(head) = read_head(&mut viewer).await? else {
        return Ok(());
    };
    let head = String::from_utf8_lossy(&head);
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next().unwrap_or_default().split_ascii_whitespace();
    let method = request_line.next().unwrap_or_default();
    let token = request_line
        .next()
        .unwrap_or_default()
        .trim_start_matches('/');
    let range = lines.find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("range")
            .then(|| value.trim().to_owned())
    });

    let head_only = match method {
        "GET" => false,
        "HEAD" => true,
        _ => return viewer.write_all(METHOD_NOT_ALLOWED).await,
    };
    let cdn_url = sources
        .lock()
        .expect("relay lock poisoned")
        .get(token)
        .cloned();
    let Some(cdn_url) = cdn_url else {
        return viewer.write_all(NOT_FOUND).await;
    };

    let mut request = if head_only {
        client.head(&cdn_url)
    } else {
        client.get(&cdn_url)
    }
    .header(reqwest::header::REFERER, REFERER)
    .header(reqwest::header::ACCEPT, "*/*");
    if let Some(range) = range {
        request = request.header(reqwest::header::RANGE, range);
    }
    let Ok(mut response) = request.send().await else {
        return viewer.write_all(BAD_GATEWAY).await;
    };

    // A 403 passes through: the CDN URL expired and the player says so.
    let status = response.status();
    let mut reply = format!(
        "HTTP/1.1 {} {}\r\nconnection: close\r\n",
        status.as_u16(),
        status.canonical_reason().unwrap_or("")
    );
    // Episodes are mp4; some CDN answers call them octet-stream, which a
    // webview will not play.
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.starts_with("video/"))
        .unwrap_or("video/mp4");
    reply.push_str(&format!("content-type: {content_type}\r\n"));
    for name in PASSED_HEADERS {
        if let Some(value) = response.headers().get(name).and_then(|v| v.to_str().ok()) {
            reply.push_str(&format!("{name}: {value}\r\n"));
        }
    }
    reply.push_str("\r\n");
    viewer.write_all(reply.as_bytes()).await?;

    if !head_only {
        // The webview drops a connection when it seeks; that ends this copy.
        while let Ok(Some(chunk)) = response.chunk().await {
            viewer.write_all(&chunk).await?;
        }
    }
    viewer.shutdown().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    const BODY: &[u8] = b"0123456789abcdefghij";

    /// A CDN stand-in that answers ranges, but only with kwik's Referer.
    async fn cdn() -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    let head = read_head(&mut socket).await.unwrap().unwrap();
                    let head = String::from_utf8_lossy(&head).to_lowercase();
                    let referer = format!("referer: {}", REFERER.to_lowercase());
                    let reply = if !head.contains(&referer) || head.contains("?expired") {
                        b"HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\n\r\n".to_vec()
                    } else if head.contains("range: bytes=10-") {
                        let mut reply = b"HTTP/1.1 206 Partial Content\r\ncontent-type: application/octet-stream\r\ncontent-length: 10\r\ncontent-range: bytes 10-19/20\r\naccept-ranges: bytes\r\n\r\n".to_vec();
                        reply.extend_from_slice(&BODY[10..]);
                        reply
                    } else {
                        let mut reply = b"HTTP/1.1 200 OK\r\ncontent-type: video/mp4\r\ncontent-length: 20\r\naccept-ranges: bytes\r\n\r\n".to_vec();
                        reply.extend_from_slice(BODY);
                        reply
                    };
                    let _ = socket.write_all(&reply).await;
                });
            }
        });
        format!("http://{address}/file.mp4")
    }

    async fn fetch(url: &str, extra: &str) -> String {
        let url = url::Url::parse(url).unwrap();
        let mut socket = TcpStream::connect(("127.0.0.1", url.port().unwrap()))
            .await
            .unwrap();
        let request = format!(
            "GET {} HTTP/1.1\r\nhost: 127.0.0.1\r\n{extra}\r\n",
            url.path()
        );
        socket.write_all(request.as_bytes()).await.unwrap();
        let mut reply = Vec::new();
        socket.read_to_end(&mut reply).await.unwrap();
        String::from_utf8_lossy(&reply).into_owned()
    }

    #[tokio::test]
    async fn relays_the_whole_episode_with_the_cdn_headers() {
        let relay = Relay::start().await.unwrap();
        let url = relay.register(cdn().await);

        let reply = fetch(&url, "").await;

        assert!(reply.starts_with("HTTP/1.1 200 OK\r\n"), "{reply}");
        assert!(reply.contains("content-type: video/mp4\r\n"));
        assert!(reply.contains("content-length: 20\r\n"));
        assert!(reply.ends_with("\r\n\r\n0123456789abcdefghij"), "{reply}");
    }

    #[tokio::test]
    async fn passes_a_range_through_so_the_player_can_seek() {
        let relay = Relay::start().await.unwrap();
        let url = relay.register(cdn().await);

        let reply = fetch(&url, "Range: bytes=10-\r\n").await;

        assert!(
            reply.starts_with("HTTP/1.1 206 Partial Content\r\n"),
            "{reply}"
        );
        assert!(reply.contains("content-range: bytes 10-19/20\r\n"));
        assert!(reply.contains("content-type: video/mp4\r\n"), "{reply}");
        assert!(reply.ends_with("\r\n\r\nabcdefghij"), "{reply}");
    }

    #[tokio::test]
    async fn refuses_a_token_it_never_issued() {
        let relay = Relay::start().await.unwrap();
        let url = relay.register(cdn().await);
        let other = format!("{}0", url);

        let reply = fetch(&other, "").await;

        assert!(reply.starts_with("HTTP/1.1 404 Not Found\r\n"), "{reply}");
    }

    #[tokio::test]
    async fn passes_an_expired_cdn_answer_through() {
        let relay = Relay::start().await.unwrap();
        let url = relay.register(format!("{}?expired", cdn().await));

        let reply = fetch(&url, "").await;

        assert!(reply.starts_with("HTTP/1.1 403 Forbidden\r\n"), "{reply}");
    }

    #[tokio::test]
    async fn answers_bad_gateway_when_the_cdn_cannot_be_reached() {
        let relay = Relay::start().await.unwrap();
        let url = relay.register("http://127.0.0.1:9/unreachable.mp4".to_owned());

        let reply = fetch(&url, "").await;

        assert!(reply.starts_with("HTTP/1.1 502 Bad Gateway\r\n"), "{reply}");
    }
}
