use std::net::IpAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde::Serialize;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use url::Url;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// The verdict of a TLS probe against the Base URL.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProbeOutcome {
    Clean,
    Poisoned,
    Unreachable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TlsOutcome {
    Trusted,
    CertificateMismatch,
    Failed,
}

pub trait DnsResolver {
    fn resolve(
        &self,
        host: &str,
        port: u16,
    ) -> impl std::future::Future<Output = Result<Vec<IpAddr>, String>> + Send;
}

pub trait TlsChecker {
    fn check(
        &self,
        address: IpAddr,
        host: &str,
        port: u16,
    ) -> impl std::future::Future<Output = TlsOutcome> + Send;
}

/// Resolves the Base URL through the system resolver, then validates each
/// address with a TLS handshake carrying the real SNI.
pub async fn probe<D: DnsResolver, T: TlsChecker>(
    base_url: &Url,
    dns: &D,
    tls: &T,
) -> ProbeOutcome {
    let Some(host) = base_url.host_str() else {
        return ProbeOutcome::Unreachable;
    };
    if base_url.scheme() != "https" {
        return ProbeOutcome::Clean;
    }
    let port = base_url.port_or_known_default().unwrap_or(443);

    let Ok(mut addresses) = dns.resolve(host, port).await else {
        return ProbeOutcome::Unreachable;
    };
    addresses.dedup();
    if addresses.is_empty() {
        return ProbeOutcome::Unreachable;
    }

    let mut mismatch = false;
    for address in addresses {
        match tls.check(address, host, port).await {
            TlsOutcome::Trusted => return ProbeOutcome::Clean,
            TlsOutcome::CertificateMismatch => mismatch = true,
            TlsOutcome::Failed => {}
        }
    }

    if mismatch {
        ProbeOutcome::Poisoned
    } else {
        ProbeOutcome::Unreachable
    }
}

/// Probe the Base URL with the real system resolver and TLS stack.
pub async fn check(base_url: &str) -> ProbeOutcome {
    let Ok(url) = Url::parse(base_url) else {
        return ProbeOutcome::Unreachable;
    };
    probe(&url, &SystemDns, &RustlsTls).await
}

pub struct SystemDns;

impl DnsResolver for SystemDns {
    async fn resolve(&self, host: &str, port: u16) -> Result<Vec<IpAddr>, String> {
        let addresses = tokio::net::lookup_host((host, port))
            .await
            .map_err(|error| error.to_string())?;
        Ok(addresses.map(|address| address.ip()).collect())
    }
}

pub struct RustlsTls;

impl TlsChecker for RustlsTls {
    async fn check(&self, address: IpAddr, host: &str, port: u16) -> TlsOutcome {
        let Ok(server_name) = rustls::pki_types::ServerName::try_from(host.to_owned()) else {
            return TlsOutcome::Failed;
        };

        let Ok(Ok(stream)) = timeout(CONNECT_TIMEOUT, TcpStream::connect((address, port))).await
        else {
            return TlsOutcome::Failed;
        };

        let connector = TlsConnector::from(tls_config());
        match timeout(HANDSHAKE_TIMEOUT, connector.connect(server_name, stream)).await {
            Ok(Ok(_)) => TlsOutcome::Trusted,
            Ok(Err(error)) => classify(&error),
            Err(_) => TlsOutcome::Failed,
        }
    }
}

fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            Arc::new(
                rustls::ClientConfig::builder()
                    .with_root_certificates(roots)
                    .with_no_client_auth(),
            )
        })
        .clone()
}

fn classify(error: &std::io::Error) -> TlsOutcome {
    use rustls::{CertificateError, Error};

    // tokio-rustls wraps rustls errors in an io::Error; the inner error lives
    // in get_ref(), not in the io::Error's source chain.
    let mut source = error
        .get_ref()
        .map(|inner| inner as &(dyn std::error::Error + 'static));
    while let Some(current) = source {
        if let Some(Error::InvalidCertificate(
            CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
        )) = current.downcast_ref::<Error>()
        {
            return TlsOutcome::CertificateMismatch;
        }
        source = current.source();
    }

    TlsOutcome::Failed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct FakeDns(Result<Vec<IpAddr>, String>);

    impl DnsResolver for FakeDns {
        async fn resolve(&self, _host: &str, _port: u16) -> Result<Vec<IpAddr>, String> {
            self.0.clone()
        }
    }

    struct FakeTls(HashMap<IpAddr, TlsOutcome>);

    impl TlsChecker for FakeTls {
        async fn check(&self, address: IpAddr, _host: &str, _port: u16) -> TlsOutcome {
            self.0.get(&address).copied().unwrap_or(TlsOutcome::Failed)
        }
    }

    fn ip(last: u8) -> IpAddr {
        IpAddr::from([203, 0, 113, last])
    }

    fn url(value: &str) -> Url {
        Url::parse(value).unwrap()
    }

    fn tls(outcomes: &[(u8, TlsOutcome)]) -> FakeTls {
        FakeTls(
            outcomes
                .iter()
                .map(|(last, outcome)| (ip(*last), *outcome))
                .collect(),
        )
    }

    #[tokio::test]
    async fn a_trusted_handshake_is_a_clean_resolver() {
        let dns = FakeDns(Ok(vec![ip(1)]));
        let tls = tls(&[(1, TlsOutcome::Trusted)]);

        assert_eq!(
            probe(&url("https://animepahe.pw"), &dns, &tls).await,
            ProbeOutcome::Clean
        );
    }

    #[tokio::test]
    async fn a_certificate_name_mismatch_is_a_poisoned_resolver() {
        let dns = FakeDns(Ok(vec![ip(1), ip(2)]));
        let tls = tls(&[
            (1, TlsOutcome::CertificateMismatch),
            (2, TlsOutcome::CertificateMismatch),
        ]);

        assert_eq!(
            probe(&url("https://animepahe.pw"), &dns, &tls).await,
            ProbeOutcome::Poisoned
        );
    }

    #[tokio::test]
    async fn one_trusted_address_wins_over_a_poisoned_one() {
        let dns = FakeDns(Ok(vec![ip(1), ip(2)]));
        let tls = tls(&[
            (1, TlsOutcome::CertificateMismatch),
            (2, TlsOutcome::Trusted),
        ]);

        assert_eq!(
            probe(&url("https://animepahe.pw"), &dns, &tls).await,
            ProbeOutcome::Clean
        );
    }

    #[tokio::test]
    async fn a_failed_lookup_is_unreachable() {
        let dns = FakeDns(Err("no such host".to_owned()));
        let tls = tls(&[]);

        assert_eq!(
            probe(&url("https://animepahe.pw"), &dns, &tls).await,
            ProbeOutcome::Unreachable
        );
    }

    #[tokio::test]
    async fn an_empty_answer_is_unreachable() {
        let dns = FakeDns(Ok(vec![]));
        let tls = tls(&[]);

        assert_eq!(
            probe(&url("https://animepahe.pw"), &dns, &tls).await,
            ProbeOutcome::Unreachable
        );
    }

    #[tokio::test]
    async fn a_failed_handshake_is_unreachable() {
        let dns = FakeDns(Ok(vec![ip(1)]));
        let tls = tls(&[(1, TlsOutcome::Failed)]);

        assert_eq!(
            probe(&url("https://animepahe.pw"), &dns, &tls).await,
            ProbeOutcome::Unreachable
        );
    }

    #[tokio::test]
    async fn an_http_base_url_needs_no_tls_probe() {
        let dns = FakeDns(Err("should not be called".to_owned()));
        let tls = tls(&[]);

        assert_eq!(
            probe(&url("http://animepahe.pw"), &dns, &tls).await,
            ProbeOutcome::Clean
        );
    }
}
