use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use url::Url;

const MAX_HEAD: usize = 8 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);
const CACHE_TTL: Duration = Duration::from_secs(60);

const BAD_REQUEST: &[u8] = b"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\n\r\n";
const METHOD_NOT_ALLOWED: &[u8] =
    b"HTTP/1.1 405 Method Not Allowed\r\nallow: CONNECT\r\ncontent-length: 0\r\n\r\n";
const BAD_GATEWAY: &[u8] = b"HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\n\r\n";
const ESTABLISHED: &[u8] = b"HTTP/1.1 200 Connection Established\r\n\r\n";

/// Resolves hostnames for tunnel targets.
pub trait Lookup {
    fn lookup(
        &self,
        host: &str,
    ) -> impl std::future::Future<Output = Result<Vec<IpAddr>, String>> + Send;
}

pub struct ProxyHandle {
    pub url: String,
}

/// Start an HTTP CONNECT proxy on an ephemeral loopback port.
///
/// The accept loop runs detached for the life of the process.
pub async fn start<L: Lookup + Send + Sync + 'static>(lookup: Arc<L>) -> io::Result<ProxyHandle> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let address = listener.local_addr()?;

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((client, _)) => {
                    let lookup = Arc::clone(&lookup);
                    tokio::spawn(async move {
                        let _ = serve(client, lookup).await;
                    });
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
    });

    Ok(ProxyHandle {
        url: format!("http://{address}"),
    })
}

async fn serve<L: Lookup>(mut client: TcpStream, lookup: Arc<L>) -> io::Result<()> {
    let Some(head) = read_head(&mut client).await? else {
        return Ok(());
    };
    let request = match parse_connect(&head) {
        Ok(request) => request,
        Err(reply) => {
            client.write_all(reply).await?;
            return Ok(());
        }
    };

    let addresses = match timeout(LOOKUP_TIMEOUT, lookup.lookup(&request.host)).await {
        Ok(Ok(addresses)) => addresses,
        _ => {
            client.write_all(BAD_GATEWAY).await?;
            return Ok(());
        }
    };

    let mut upstream = None;
    for address in addresses {
        let target = SocketAddr::new(address, request.port);
        if let Ok(Ok(stream)) = timeout(CONNECT_TIMEOUT, TcpStream::connect(target)).await {
            upstream = Some(stream);
            break;
        }
    }

    let Some(mut upstream) = upstream else {
        client.write_all(BAD_GATEWAY).await?;
        return Ok(());
    };

    client.write_all(ESTABLISHED).await?;
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
    Ok(())
}

async fn read_head(stream: &mut TcpStream) -> io::Result<Option<Vec<u8>>> {
    let mut head = Vec::new();
    let mut chunk = [0u8; 1024];

    loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(None);
        }
        head.extend_from_slice(&chunk[..read]);
        if head.windows(4).any(|window| window == b"\r\n\r\n") {
            return Ok(Some(head));
        }
        if head.len() > MAX_HEAD {
            return Ok(None);
        }
    }
}

struct ConnectRequest {
    host: String,
    port: u16,
}

fn parse_connect(head: &[u8]) -> Result<ConnectRequest, &'static [u8]> {
    let line_end = head
        .windows(2)
        .position(|window| window == b"\r\n")
        .unwrap_or(head.len());
    let Ok(line) = std::str::from_utf8(&head[..line_end]) else {
        return Err(BAD_REQUEST);
    };
    let mut parts = line.split_ascii_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    let version = parts.next().unwrap_or_default();

    if method != "CONNECT" {
        return Err(METHOD_NOT_ALLOWED);
    }
    if !matches!(version, "HTTP/1.0" | "HTTP/1.1") {
        return Err(BAD_REQUEST);
    }
    let Some((host, port)) = target.rsplit_once(':') else {
        return Err(BAD_REQUEST);
    };
    let host = host.trim_matches(['[', ']']);
    if host.is_empty() {
        return Err(BAD_REQUEST);
    }
    let Ok(port) = port.parse::<u16>() else {
        return Err(BAD_REQUEST);
    };

    Ok(ConnectRequest {
        host: host.to_owned(),
        port,
    })
}

pub struct DohLookup {
    client: reqwest::Client,
    primary: Url,
    fallback: Url,
    cache: Mutex<HashMap<String, (Instant, Vec<IpAddr>)>>,
}

impl DohLookup {
    pub fn new(client: reqwest::Client, primary: Url, fallback: Url) -> Self {
        Self {
            client,
            primary,
            fallback,
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn cloudflare() -> Self {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(LOOKUP_TIMEOUT)
            .build()
            .expect("could not build the DoH client");

        Self::new(
            client,
            Url::parse("https://cloudflare-dns.com/dns-query").expect("valid DoH URL"),
            Url::parse("https://1.1.1.1/dns-query").expect("valid DoH URL"),
        )
    }

    fn cached(&self, host: &str) -> Option<Vec<IpAddr>> {
        let cache = self.cache.lock().ok()?;
        let (stored, addresses) = cache.get(host)?;
        (stored.elapsed() < CACHE_TTL).then(|| addresses.clone())
    }

    async fn query(&self, endpoint: &Url, host: &str, record: &str) -> Result<Vec<IpAddr>, String> {
        let mut url = endpoint.clone();
        url.query_pairs_mut()
            .append_pair("name", host)
            .append_pair("type", record);

        let response = self
            .client
            .get(url)
            .header("accept", "application/dns-json")
            .send()
            .await
            .map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            return Err(format!("DoH answered {}", response.status()));
        }
        parse_doh(&response.text().await.map_err(|error| error.to_string())?)
    }
}

impl Lookup for DohLookup {
    async fn lookup(&self, host: &str) -> Result<Vec<IpAddr>, String> {
        if let Some(addresses) = self.cached(host) {
            return Ok(addresses);
        }

        for endpoint in [&self.primary, &self.fallback] {
            for record in ["A", "AAAA"] {
                if let Ok(addresses) = self.query(endpoint, host, record).await
                    && !addresses.is_empty()
                {
                    if let Ok(mut cache) = self.cache.lock() {
                        cache.insert(host.to_owned(), (Instant::now(), addresses.clone()));
                    }
                    return Ok(addresses);
                }
            }
        }

        Err(format!("could not resolve {host} over DoH"))
    }
}

#[derive(Deserialize)]
struct DohResponse {
    #[serde(rename = "Status")]
    status: u8,
    #[serde(rename = "Answer", default)]
    answers: Vec<DohAnswer>,
}

#[derive(Deserialize)]
struct DohAnswer {
    #[serde(rename = "type")]
    record_type: u16,
    data: String,
}

fn parse_doh(body: &str) -> Result<Vec<IpAddr>, String> {
    let response: DohResponse =
        serde_json::from_str(body).map_err(|error| format!("malformed DoH answer: {error}"))?;
    if response.status != 0 {
        return Err(format!("DoH returned status {}", response.status));
    }

    let addresses = response
        .answers
        .into_iter()
        .filter_map(|answer| match answer.record_type {
            1 => answer.data.parse::<Ipv4Addr>().ok().map(IpAddr::from),
            28 => answer.data.parse::<Ipv6Addr>().ok().map(IpAddr::from),
            _ => None,
        })
        .collect();

    Ok(addresses)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn connect(proxy: &ProxyHandle) -> TcpStream {
        let address: SocketAddr = proxy.url.trim_start_matches("http://").parse().unwrap();
        TcpStream::connect(address).await.unwrap()
    }

    async fn read_reply(stream: &mut TcpStream) -> String {
        let mut reply = vec![0u8; 1024];
        let read = stream.read(&mut reply).await.unwrap();
        String::from_utf8_lossy(&reply[..read]).into_owned()
    }

    struct RecordingLookup {
        addresses: Vec<IpAddr>,
        hosts: Mutex<Vec<String>>,
    }

    impl RecordingLookup {
        fn new(addresses: Vec<IpAddr>) -> Self {
            Self {
                addresses,
                hosts: Mutex::new(Vec::new()),
            }
        }
    }

    impl Lookup for RecordingLookup {
        async fn lookup(&self, host: &str) -> Result<Vec<IpAddr>, String> {
            self.hosts.lock().unwrap().push(host.to_owned());
            Ok(self.addresses.clone())
        }
    }

    struct FailingLookup;

    impl Lookup for FailingLookup {
        async fn lookup(&self, _host: &str) -> Result<Vec<IpAddr>, String> {
            Err("no address".to_owned())
        }
    }

    #[tokio::test]
    async fn tunnels_bytes_transparently_and_resolves_the_requested_host() {
        let echo = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let echo_address = echo.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = echo.accept().await.unwrap();
            let mut bytes = [0u8; 5];
            socket.read_exact(&mut bytes).await.unwrap();
            socket.write_all(&bytes).await.unwrap();
        });

        let lookup = Arc::new(RecordingLookup::new(vec![echo_address.ip()]));
        let proxy = start(Arc::clone(&lookup)).await.unwrap();
        let mut client = connect(&proxy).await;

        client
            .write_all(
                format!(
                    "CONNECT anime.example:{} HTTP/1.1\r\nhost: anime.example\r\n\r\n",
                    echo_address.port()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        assert!(read_reply(&mut client).await.starts_with("HTTP/1.1 200"));

        client.write_all(b"hello").await.unwrap();
        let mut echoed = [0u8; 5];
        client.read_exact(&mut echoed).await.unwrap();

        assert_eq!(&echoed, b"hello");
        assert_eq!(lookup.hosts.lock().unwrap().as_slice(), ["anime.example"]);
    }

    #[tokio::test]
    async fn rejects_requests_that_are_not_connect() {
        let proxy = start(Arc::new(RecordingLookup::new(vec![]))).await.unwrap();
        let mut client = connect(&proxy).await;

        client
            .write_all(b"GET / HTTP/1.1\r\nhost: anime.example\r\n\r\n")
            .await
            .unwrap();

        assert!(read_reply(&mut client).await.starts_with("HTTP/1.1 405"));
    }

    #[tokio::test]
    async fn answers_bad_gateway_when_the_host_does_not_resolve() {
        let proxy = start(Arc::new(FailingLookup)).await.unwrap();
        let mut client = connect(&proxy).await;

        client
            .write_all(b"CONNECT anime.example:443 HTTP/1.1\r\n\r\n")
            .await
            .unwrap();

        assert!(read_reply(&mut client).await.starts_with("HTTP/1.1 502"));
    }

    #[tokio::test]
    async fn parses_addresses_from_a_doh_answer() {
        let body = r#"{
            "Status": 0,
            "Answer": [
                {"name": "anime.example", "type": 5, "data": "cdn.example"},
                {"name": "anime.example", "type": 1, "data": "203.0.113.9"},
                {"name": "anime.example", "type": 28, "data": "2001:db8::9"}
            ]
        }"#;

        assert_eq!(
            parse_doh(body).unwrap(),
            vec![
                IpAddr::from([203, 0, 113, 9]),
                IpAddr::from("2001:db8::9".parse::<Ipv6Addr>().unwrap())
            ]
        );
    }

    #[tokio::test]
    async fn rejects_a_doh_answer_with_an_error_status() {
        assert!(parse_doh(r#"{"Status": 2}"#).is_err());
    }

    #[tokio::test]
    async fn falls_back_to_the_second_endpoint_and_then_caches() {
        let doh = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let doh_address = doh.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = doh.accept().await.unwrap();
            let mut request = [0u8; 2048];
            let _ = socket.read(&mut request).await.unwrap();
            let body = r#"{"Status":0,"Answer":[{"name":"anime.example","type":1,"TTL":60,"data":"203.0.113.9"}]}"#;
            let reply = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/dns-json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(reply.as_bytes()).await.unwrap();
        });

        let dead = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let dead_address = dead.local_addr().unwrap();
        drop(dead);

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let lookup = DohLookup::new(
            client,
            Url::parse(&format!("http://{dead_address}/dns-query")).unwrap(),
            Url::parse(&format!("http://{doh_address}/dns-query")).unwrap(),
        );

        let expected = vec![IpAddr::from([203, 0, 113, 9])];
        assert_eq!(lookup.lookup("anime.example").await.unwrap(), expected);
        // The server answered once; this reply can only come from the cache.
        assert_eq!(lookup.lookup("anime.example").await.unwrap(), expected);
    }
}
