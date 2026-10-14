use crate::core::errors::DualBondError;
use crate::download::http_client::{empty_body, empty_request, send_once};
use hyper::header::{CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, LAST_MODIFIED, LOCATION, RANGE};
use hyper::{Method, StatusCode, Uri};

const MAX_REDIRECTS: u32 = 5;

/// Esito del probe iniziale (§4.6 della spec): URL finale dopo i redirect,
/// e conferma **reale** — non dedotta da `Accept-Ranges` — del supporto ai
/// Range parziali, ottenuta con una `GET Range: bytes=0-0` effettiva.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResult {
    pub final_url: String,
    pub total_size: Option<u64>,
    pub supports_ranges: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub content_type: Option<String>,
}

/// Esegue il probe: `GET Range: bytes=0-0` sull'URL, seguendo fino a 5
/// redirect e fissando l'URL finale (i worker di download non seguono
/// redirect, § 4.6 punto 3). Un `200` in risposta a `Range` significa che
/// il server la ignora (§0.3-C): niente stallo, si passa a
/// `supports_ranges = false` e il chiamante userà una sola connessione.
pub async fn probe(url: &str) -> Result<ProbeResult, DualBondError> {
    let mut current = url.to_string();

    for _ in 0..=MAX_REDIRECTS {
        let uri: Uri = current
            .parse()
            .map_err(|e| DualBondError::Http(format!("URL non valido '{current}': {e}")))?;

        let req = empty_request(Method::GET, &uri)?
            .header(RANGE, "bytes=0-0")
            .body(empty_body())
            .map_err(|e| DualBondError::Http(e.to_string()))?;

        let resp = send_once(&uri, req).await?;
        let status = resp.status();

        if status.is_redirection() {
            let location = resp
                .headers()
                .get(LOCATION)
                .ok_or_else(|| DualBondError::Http(format!("redirect {status} senza Location")))?
                .to_str()
                .map_err(|e| DualBondError::Http(format!("Location non valido: {e}")))?;
            current = resolve_redirect(&uri, location)?;
            continue;
        }

        let headers = resp.headers();
        let etag = header_str(headers, &ETAG);
        let last_modified = header_str(headers, &LAST_MODIFIED);
        let content_type = header_str(headers, &CONTENT_TYPE);

        return match status {
            StatusCode::PARTIAL_CONTENT => {
                let total_size = header_str(headers, &CONTENT_RANGE)
                    .as_deref()
                    .and_then(parse_content_range_total);
                Ok(ProbeResult {
                    final_url: current,
                    total_size,
                    supports_ranges: true,
                    etag,
                    last_modified,
                    content_type,
                })
            }
            StatusCode::OK => {
                let total_size = header_str(headers, &CONTENT_LENGTH)
                    .and_then(|s| s.parse::<u64>().ok());
                Ok(ProbeResult {
                    final_url: current,
                    total_size,
                    supports_ranges: false,
                    etag,
                    last_modified,
                    content_type,
                })
            }
            other => Err(DualBondError::Http(format!(
                "status inatteso durante il probe: {other}"
            ))),
        };
    }

    Err(DualBondError::Http("troppi redirect nel probe (>5)".into()))
}

fn header_str(headers: &hyper::HeaderMap, name: &hyper::header::HeaderName) -> Option<String> {
    headers.get(name)?.to_str().ok().map(str::to_string)
}

/// `Content-Range: bytes 0-0/12345` → `12345`. `*` (dimensione sconosciuta)
/// → `None`, coerente con `total_size: Option<u64>`.
fn parse_content_range_total(value: &str) -> Option<u64> {
    let after_slash = value.rsplit_once('/')?.1;
    if after_slash == "*" {
        return None;
    }
    after_slash.parse().ok()
}

fn resolve_redirect(base: &Uri, location: &str) -> Result<String, DualBondError> {
    if location.starts_with("http://") || location.starts_with("https://") {
        return Ok(location.to_string());
    }
    // Redirect relativo: ricompone scheme+authority di `base` con il path
    // nuovo (caso comune per redirect stessa-origine).
    let scheme = base.scheme_str().unwrap_or("http");
    let authority = base
        .authority()
        .ok_or_else(|| DualBondError::Http("redirect relativo senza authority di base".into()))?;
    Ok(format!("{scheme}://{authority}{location}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::test_server::{self, Scenario, TEST_ETAG};

    #[tokio::test]
    async fn confirms_real_range_support_via_206() {
        let body = vec![7u8; 1024];
        let server = test_server::spawn(Scenario::RangeSupported(body.clone())).await;
        let url = format!("http://127.0.0.1:{}/file.bin", server.port);

        let result = probe(&url).await.unwrap();

        assert!(result.supports_ranges);
        assert_eq!(result.total_size, Some(1024));
        assert_eq!(result.etag.as_deref(), Some(TEST_ETAG));
        assert_eq!(result.final_url, url);

        server.stop();
    }

    #[tokio::test]
    async fn detects_server_that_declares_range_but_ignores_it() {
        let body = vec![9u8; 512];
        let server = test_server::spawn(Scenario::RangeDeclaredButIgnored(body.clone())).await;
        let url = format!("http://127.0.0.1:{}/ignores-range.bin", server.port);

        let result = probe(&url).await.unwrap();

        assert!(
            !result.supports_ranges,
            "un 200 in risposta a Range deve risultare supports_ranges=false, mai fidarsi di Accept-Ranges"
        );
        assert_eq!(result.total_size, Some(512));

        server.stop();
    }

    #[tokio::test]
    async fn server_without_range_support_at_all() {
        let body = vec![1u8; 256];
        let server = test_server::spawn(Scenario::PlainOk(body.clone())).await;
        let url = format!("http://127.0.0.1:{}/plain.bin", server.port);

        let result = probe(&url).await.unwrap();

        assert!(!result.supports_ranges);
        assert_eq!(result.total_size, Some(256));

        server.stop();
    }

    #[tokio::test]
    async fn follows_redirect_and_fixes_final_url() {
        let body = vec![3u8; 128];
        let target = test_server::spawn(Scenario::RangeSupported(body.clone())).await;
        let target_url = format!("http://127.0.0.1:{}/real-file.bin", target.port);

        let redirector = test_server::spawn(Scenario::Redirect(target_url.clone())).await;
        let start_url = format!("http://127.0.0.1:{}/short-link", redirector.port);

        let result = probe(&start_url).await.unwrap();

        assert_eq!(result.final_url, target_url);
        assert!(result.supports_ranges);

        redirector.stop();
        target.stop();
    }

    #[test]
    fn parses_content_range_total() {
        assert_eq!(parse_content_range_total("bytes 0-0/12345"), Some(12345));
        assert_eq!(parse_content_range_total("bytes 0-0/*"), None);
    }
}
