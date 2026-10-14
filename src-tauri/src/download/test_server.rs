//! Server HTTP locale minimale (`hyper`), usato **solo nei test** per
//! esercitare probe/downloader contro scenari reali di rete (§10 della
//! spec: "server di test locale con Range; senza Range; che dichiara Range
//! ma lo ignora; ...") senza dipendere da host esterni.
#![cfg(test)]

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub const TEST_ETAG: &str = "\"dualbond-test-etag\"";

#[derive(Clone)]
pub enum Scenario {
    /// 200 con corpo fisso, nessun supporto Range dichiarato.
    PlainOk(Vec<u8>),
    /// Risponde con l'header `Host` ricevuto come corpo (verifica che il
    /// client lo imposti correttamente).
    EchoHostHeader,
    /// Supporto Range reale: risponde 206 con `Content-Range`/`ETag` a una
    /// richiesta `Range`, 200 completo altrimenti.
    RangeSupported(Vec<u8>),
    /// Dichiara `Accept-Ranges: bytes` ma **ignora** l'header `Range` e
    /// risponde sempre 200 con il corpo intero (§0.3-C, trappola da
    /// intercettare: mai scrivere quel corpo all'offset del segmento).
    RangeDeclaredButIgnored(Vec<u8>),
    /// Redirect singolo verso un altro URL.
    Redirect(String),
    /// Come `RangeSupported`, ma le prime `fail_first` connessioni accettate
    /// vengono chiuse immediatamente senza rispondere (simula un reset di
    /// trasporto transitorio: connect riesce, l'invio della richiesta fallisce).
    /// Usato per verificare il retry loop su `DualBondError::Transport`.
    FlakyThenRangeSupported {
        fail_first: usize,
        body: Vec<u8>,
    },
    /// Le prime `limited_first` richieste ricevono `429 Too Many Requests`
    /// con `Retry-After: <retry_after_secs>`, poi si comporta come
    /// `RangeSupported` — usato per verificare che il worker rispetti il
    /// backoff invece di trattare il 429 come errore fatale (§4.7).
    RateLimitedThenRangeSupported {
        limited_first: usize,
        retry_after_secs: u64,
        body: Vec<u8>,
    },
}

pub struct TestServer {
    pub port: u16,
    handle: JoinHandle<()>,
}

impl TestServer {
    pub fn stop(self) {
        self.handle.abort();
    }
}

pub async fn spawn(scenario: Scenario) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind test server");
    let port = listener.local_addr().unwrap().port();
    let scenario = Arc::new(scenario);
    // Contatore condiviso: per `FlakyThenRangeSupported` conta le connessioni
    // accettate (decide se chiuderle subito), per `RateLimitedThenRangeSupported`
    // conta le richieste servite (decide se rispondere 429). Un solo campo
    // riusato perché le due varianti sono mutuamente esclusive per istanza.
    let request_count = Arc::new(AtomicUsize::new(0));

    let handle = tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(v) => v,
                Err(_) => break,
            };

            if let Scenario::FlakyThenRangeSupported { fail_first, .. } = scenario.as_ref() {
                if request_count.fetch_add(1, Ordering::SeqCst) < *fail_first {
                    // Chiude subito, senza rispondere: simula un reset di
                    // trasporto transitorio lato client.
                    drop(stream);
                    continue;
                }
            }

            let io = TokioIo::new(stream);
            let scenario = scenario.clone();
            let request_count = request_count.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req| {
                    let scenario = scenario.clone();
                    let request_count = request_count.clone();
                    async move { Ok::<_, Infallible>(handle_request(&scenario, &request_count, req)) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await;
            });
        }
    });

    TestServer { port, handle }
}

fn handle_request(
    scenario: &Scenario,
    request_count: &AtomicUsize,
    req: Request<Incoming>,
) -> Response<Full<Bytes>> {
    match scenario {
        Scenario::PlainOk(body) => Response::new(Full::new(Bytes::from(body.clone()))),
        Scenario::EchoHostHeader => {
            let host = req
                .headers()
                .get(hyper::header::HOST)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            Response::new(Full::new(Bytes::from(host)))
        }
        Scenario::RangeSupported(body) => match parse_range(&req, body.len()) {
            Some((start, end)) => Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header("Content-Range", format!("bytes {start}-{end}/{}", body.len()))
                .header("Content-Length", (end - start + 1).to_string())
                .header("Accept-Ranges", "bytes")
                .header("ETag", TEST_ETAG)
                .body(Full::new(Bytes::copy_from_slice(&body[start..=end])))
                .unwrap(),
            None => Response::builder()
                .status(StatusCode::OK)
                .header("Content-Length", body.len().to_string())
                .header("Accept-Ranges", "bytes")
                .header("ETag", TEST_ETAG)
                .body(Full::new(Bytes::from(body.clone())))
                .unwrap(),
        },
        Scenario::RangeDeclaredButIgnored(body) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Length", body.len().to_string())
            .header("Accept-Ranges", "bytes")
            .header("ETag", TEST_ETAG)
            .body(Full::new(Bytes::from(body.clone())))
            .unwrap(),
        Scenario::Redirect(to) => Response::builder()
            .status(StatusCode::FOUND)
            .header("Location", to.clone())
            .body(Full::new(Bytes::new()))
            .unwrap(),
        Scenario::RateLimitedThenRangeSupported {
            limited_first,
            retry_after_secs,
            body,
        } => {
            if request_count.fetch_add(1, Ordering::SeqCst) < *limited_first {
                return Response::builder()
                    .status(StatusCode::TOO_MANY_REQUESTS)
                    .header("Retry-After", retry_after_secs.to_string())
                    .body(Full::new(Bytes::new()))
                    .unwrap();
            }
            match parse_range(&req, body.len()) {
                Some((start, end)) => Response::builder()
                    .status(StatusCode::PARTIAL_CONTENT)
                    .header("Content-Range", format!("bytes {start}-{end}/{}", body.len()))
                    .header("Content-Length", (end - start + 1).to_string())
                    .header("Accept-Ranges", "bytes")
                    .header("ETag", TEST_ETAG)
                    .body(Full::new(Bytes::copy_from_slice(&body[start..=end])))
                    .unwrap(),
                None => Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Length", body.len().to_string())
                    .header("Accept-Ranges", "bytes")
                    .header("ETag", TEST_ETAG)
                    .body(Full::new(Bytes::from(body.clone())))
                    .unwrap(),
            }
        }
        Scenario::FlakyThenRangeSupported { body, .. } => match parse_range(&req, body.len()) {
            Some((start, end)) => Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header("Content-Range", format!("bytes {start}-{end}/{}", body.len()))
                .header("Content-Length", (end - start + 1).to_string())
                .header("Accept-Ranges", "bytes")
                .header("ETag", TEST_ETAG)
                .body(Full::new(Bytes::copy_from_slice(&body[start..=end])))
                .unwrap(),
            None => Response::builder()
                .status(StatusCode::OK)
                .header("Content-Length", body.len().to_string())
                .header("Accept-Ranges", "bytes")
                .header("ETag", TEST_ETAG)
                .body(Full::new(Bytes::from(body.clone())))
                .unwrap(),
        },
    }
}

/// Interpreta `Range: bytes=<start>-<end>` o `bytes=<start>-` (aperto fino
/// a fine corpo). Ritorna `None` se l'header manca o non è parsabile.
fn parse_range(req: &Request<Incoming>, body_len: usize) -> Option<(usize, usize)> {
    let raw = req.headers().get(hyper::header::RANGE)?.to_str().ok()?;
    let spec = raw.strip_prefix("bytes=")?;
    let (start_s, end_s) = spec.split_once('-')?;
    let start: usize = start_s.parse().ok()?;
    let end: usize = if end_s.is_empty() {
        body_len.saturating_sub(1)
    } else {
        end_s.parse().ok()?
    };
    if start > end || end >= body_len {
        return None;
    }
    Some((start, end))
}
