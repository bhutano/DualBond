use crate::core::errors::DualBondError;
use crate::network::connector::LinkConnector;
use bytes::Bytes;
use http_body_util::Empty;
use hyper::body::Incoming;
use hyper::client::conn::http1;
use hyper::{Request, Response, Uri};
use hyper_util::rt::TokioIo;
use std::sync::{Arc, Once};
use tokio::net::TcpStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::TlsConnector;

/// `rustls` 0.23 richiede un `CryptoProvider` di processo. Con una sola
/// feature di crypto abilitata (qui `aws-lc-rs`, default della crate)
/// basterebbe la scelta implicita, ma installarlo esplicitamente una sola
/// volta rende il comportamento deterministico anche se in futuro si
/// abilitasse anche `ring`.
fn ensure_crypto_provider() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

fn tls_connector() -> Result<TlsConnector, DualBondError> {
    let mut root_store = rustls::RootCertStore::empty();
    let certs = rustls_native_certs::load_native_certs().map_err(DualBondError::from)?;
    for cert in certs {
        root_store
            .add(cert)
            .map_err(|e| DualBondError::Http(format!("certificato radice non valido: {e}")))?;
    }
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(config)))
}

/// Apre una connessione nuova (nessun pooling: il pooling per-link arriva
/// con lo scheduler multi-link in Milestone 4, §4.2) e invia una singola
/// richiesta HTTP/1.1, restituendo la risposta con il body ancora da
/// consumare (streaming).
pub async fn send_once(
    uri: &Uri,
    req: Request<Empty<Bytes>>,
) -> Result<Response<Incoming>, DualBondError> {
    ensure_crypto_provider();

    let (host, port, is_https) = host_port_https(uri)?;

    let tcp = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        TcpStream::connect((host.as_str(), port)),
    )
    .await
    .map_err(|_| DualBondError::Transport(format!("timeout di connessione a {host}:{port}")))?
    .map_err(|e| DualBondError::Transport(format!("connessione fallita a {host}:{port}: {e}")))?;

    send_over_tcp(tcp, &host, is_https, req).await
}

/// Estrae host, porta (con default 80/443) e flag HTTPS da un `Uri`.
/// Condiviso tra `send_once` e i chiamanti che si connettono da soli
/// (`network::connector`, Milestone 4) per non duplicare la logica di
/// default-port.
pub fn host_port_https(uri: &Uri) -> Result<(String, u16, bool), DualBondError> {
    let host = uri
        .host()
        .ok_or_else(|| DualBondError::Http(format!("URL senza host: {uri}")))?
        .to_string();
    let is_https = uri.scheme_str() == Some("https");
    let port = uri.port_u16().unwrap_or(if is_https { 443 } else { 80 });
    Ok((host, port, is_https))
}

/// Come `send_once`, ma su un `TcpStream` già connesso — usato dal worker
/// multi-link (Milestone 4) che si connette tramite `network::connector`
/// (socket bindato al link con `IP_UNICAST_IF`) invece della route di
/// default usata da `send_once`.
pub async fn send_over_tcp(
    tcp: TcpStream,
    host: &str,
    is_https: bool,
    req: Request<Empty<Bytes>>,
) -> Result<Response<Incoming>, DualBondError> {
    ensure_crypto_provider();
    tcp.set_nodelay(true).ok();

    if is_https {
        let connector = tls_connector()?;
        let server_name = ServerName::try_from(host.to_string())
            .map_err(|_| DualBondError::Http(format!("hostname TLS non valido: {host}")))?
            .to_owned();
        let tls_stream = connector
            .connect(server_name, tcp)
            .await
            .map_err(|e| DualBondError::Transport(format!("handshake TLS fallito con {host}: {e}")))?;
        send_over(TokioIo::new(tls_stream), req).await
    } else {
        send_over(TokioIo::new(tcp), req).await
    }
}

async fn send_over<S>(
    io: TokioIo<S>,
    req: Request<Empty<Bytes>>,
) -> Result<Response<Incoming>, DualBondError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, conn) = http1::handshake(io)
        .await
        .map_err(|e| DualBondError::Transport(format!("handshake HTTP fallito: {e}")))?;

    tokio::spawn(async move {
        let _ = conn.await;
    });

    sender
        .send_request(req)
        .await
        .map_err(|e| DualBondError::Transport(format!("richiesta HTTP fallita: {e}")))
}

/// Costruisce una richiesta con body vuoto e `Host` impostato esplicitamente
/// (obbligatorio in HTTP/1.1, `hyper::client::conn` non lo aggiunge da solo).
pub fn empty_request(
    method: hyper::Method,
    uri: &Uri,
) -> Result<hyper::http::request::Builder, DualBondError> {
    let authority = uri
        .authority()
        .ok_or_else(|| DualBondError::Http(format!("URL senza authority: {uri}")))?
        .as_str()
        .to_string();
    Ok(Request::builder()
        .method(method)
        .uri(uri.clone())
        .header(hyper::header::HOST, authority))
}

pub fn empty_body() -> Empty<Bytes> {
    Empty::new()
}

/// Ritenta `send_once` fino a 3 tentativi, ma solo su
/// `DualBondError::Transport` (fallimenti di connessione/handshake/invio,
/// transitori per natura — §4.7 della spec: "Timeout / connection reset →
/// segmento in coda"). Mai su `DualBondError::Http`, che segnala una
/// decisione di protocollo (status inatteso, ETag divergente) e non va
/// ritentata ciecamente. Ogni tentativo ricostruisce la richiesta da zero:
/// `Request` non è clonabile e un tentativo fallito ne consuma il body.
pub async fn send_once_with_retry(
    uri: &Uri,
    build_req: impl Fn() -> Result<Request<Empty<Bytes>>, DualBondError>,
) -> Result<Response<Incoming>, DualBondError> {
    const MAX_ATTEMPTS: u32 = 3;
    let mut attempt = 0;
    loop {
        attempt += 1;
        let req = build_req()?;
        match send_once(uri, req).await {
            Ok(resp) => return Ok(resp),
            Err(DualBondError::Transport(_)) if attempt < MAX_ATTEMPTS => {
                tokio::time::sleep(full_jitter_backoff(attempt - 1)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Backoff esponenziale con *full jitter* (§4.7 della spec):
/// `delay = random(0, min(60s, 0,5s × 2^n))`. Il jitter evita che tutti i
/// worker riprovino in sincrono dopo un evento di rete comune (es. il link
/// che cade e torna). `n` è l'indice del tentativo fallito, 0-based.
fn full_jitter_backoff(n: u32) -> std::time::Duration {
    const CAP_MS: u64 = 60_000;
    const BASE_MS: u64 = 500;
    let max_delay_ms = BASE_MS.saturating_mul(1u64 << n.min(20)).min(CAP_MS);
    let jitter_seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    let delay_ms = jitter_seed % (max_delay_ms + 1);
    std::time::Duration::from_millis(delay_ms)
}

/// Come `send_once_with_retry`, ma la connessione passa da un
/// `LinkConnector` (Milestone 4, worker multi-link) invece che dalla route
/// di default: stessa politica di retry, solo su `DualBondError::Transport`.
pub async fn send_via_connector_with_retry(
    connector: &LinkConnector,
    uri: &Uri,
    build_req: impl Fn() -> Result<Request<Empty<Bytes>>, DualBondError>,
) -> Result<Response<Incoming>, DualBondError> {
    const MAX_ATTEMPTS: u32 = 3;
    let (host, port, is_https) = host_port_https(uri)?;
    let mut attempt = 0;
    loop {
        attempt += 1;
        let req = build_req()?;
        let attempt_result: Result<Response<Incoming>, DualBondError> = async {
            let tcp = connector.connect(&host, port).await?;
            send_over_tcp(tcp, &host, is_https, req).await
        }
        .await;
        match attempt_result {
            Ok(resp) => return Ok(resp),
            Err(DualBondError::Transport(_)) if attempt < MAX_ATTEMPTS => {
                tokio::time::sleep(full_jitter_backoff(attempt - 1)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::test_server;
    use hyper::Method;

    #[tokio::test]
    async fn sends_get_and_reads_status_and_body_over_plain_http() {
        let server = test_server::spawn(test_server::Scenario::PlainOk(b"hello dualbond".to_vec())).await;
        let uri: Uri = format!("http://127.0.0.1:{}/file", server.port).parse().unwrap();

        let req = empty_request(Method::GET, &uri)
            .unwrap()
            .body(empty_body())
            .unwrap();
        let resp = send_once(&uri, req).await.unwrap();

        assert_eq!(resp.status(), hyper::StatusCode::OK);
        let body = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        assert_eq!(&body[..], b"hello dualbond");

        server.stop();
    }

    #[tokio::test]
    async fn connect_failure_is_transport_error_not_http() {
        // Bind per ottenere una porta libera, poi chiude subito: nulla
        // ascolta più su quella porta, la connect() fallisce (ECONNREFUSED).
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let uri: Uri = format!("http://127.0.0.1:{port}/x").parse().unwrap();
        let req = empty_request(Method::GET, &uri)
            .unwrap()
            .body(empty_body())
            .unwrap();
        let err = send_once(&uri, req).await.unwrap_err();

        assert!(
            matches!(err, DualBondError::Transport(_)),
            "atteso DualBondError::Transport, ottenuto {err:?}"
        );
    }

    #[test]
    fn full_jitter_backoff_never_exceeds_the_exponential_cap() {
        for n in 0..8 {
            let expected_cap_ms = (500u64 * (1u64 << n)).min(60_000);
            for _ in 0..20 {
                let delay = full_jitter_backoff(n);
                assert!(
                    delay.as_millis() as u64 <= expected_cap_ms,
                    "n={n}: delay {delay:?} supera il cap atteso {expected_cap_ms}ms"
                );
            }
        }
    }

    #[test]
    fn full_jitter_backoff_saturates_at_60_seconds_for_large_n() {
        for _ in 0..20 {
            assert!(full_jitter_backoff(30).as_millis() as u64 <= 60_000);
        }
    }

    #[tokio::test]
    async fn send_via_connector_with_retry_works_over_plain_connector() {
        let server = test_server::spawn(test_server::Scenario::PlainOk(b"via connector".to_vec())).await;
        let uri: Uri = format!("http://127.0.0.1:{}/f", server.port).parse().unwrap();

        let resp = send_via_connector_with_retry(&LinkConnector::Plain, &uri, || {
            empty_request(Method::GET, &uri)?.body(empty_body()).map_err(|e| DualBondError::Http(e.to_string()))
        })
        .await
        .unwrap();

        assert_eq!(resp.status(), hyper::StatusCode::OK);
        server.stop();
    }

    #[tokio::test]
    async fn send_via_connector_with_retry_absorbs_transient_transport_failures() {
        let server = test_server::spawn(test_server::Scenario::FlakyThenRangeSupported {
            fail_first: 2,
            body: b"resiliente".to_vec(),
        })
        .await;
        let uri: Uri = format!("http://127.0.0.1:{}/f", server.port).parse().unwrap();

        let resp = send_via_connector_with_retry(&LinkConnector::Plain, &uri, || {
            empty_request(Method::GET, &uri)?.body(empty_body()).map_err(|e| DualBondError::Http(e.to_string()))
        })
        .await
        .unwrap_or_else(|e| panic!("il retry doveva assorbire il fallimento transitorio: {e}"));

        assert_eq!(resp.status(), hyper::StatusCode::OK);
        server.stop();
    }

    #[tokio::test]
    async fn host_header_is_set_from_authority() {
        let server = test_server::spawn(test_server::Scenario::EchoHostHeader).await;
        let uri: Uri = format!("http://127.0.0.1:{}/whoami", server.port).parse().unwrap();

        let req = empty_request(Method::GET, &uri)
            .unwrap()
            .body(empty_body())
            .unwrap();
        let resp = send_once(&uri, req).await.unwrap();
        let body = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();

        assert_eq!(&body[..], format!("127.0.0.1:{}", server.port).as_bytes());

        server.stop();
    }
}
