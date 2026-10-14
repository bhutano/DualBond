//! Worker di download per un singolo link (Milestone 4): preleva segmenti
//! dallo scheduler work-stealing, li scarica con `Range` sulla connessione
//! bindata al proprio link, applica la barriera di durabilità e riporta il
//! progresso. Quando la coda è vuota prova il tail-splitting; quando anche
//! quello non ha più nulla da offrire e lo scheduler è tutto `Done`, esce.
//!
//! **Semplificazione nota rispetto alla spec**: un intero segmento viene
//! scaricato in un'unica richiesta e la barriera di durabilità scatta a
//! fine segmento, non ogni 8 MiB/2s come da §4.5 — accettabile perché il
//! tetto di `chunk::segment_size_bytes` è 64 MiB (al più 64 MiB da
//! ri-scaricare dopo un crash a metà segmento, mai corruzione). Un
//! checkpoint infra-segmento è un miglioramento futuro, non un bug.

use crate::core::errors::DualBondError;
use crate::download::body_stream::stream_body_to_file;
use crate::download::circuit_breaker::CircuitBreaker;
use crate::download::http_client::{empty_body, empty_request, send_via_connector_with_retry};
use crate::download::scheduler::{Scheduler, SegmentState};
use crate::network::connector::LinkConnector;
use crate::storage::{FileWriter, StateDb};
use hyper::header::{ETAG, IF_RANGE, RANGE, RETRY_AFTER};
use hyper::{Method, StatusCode, Uri};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub struct WorkerContext {
    pub download_id: i64,
    pub link_key: String,
    pub connector: LinkConnector,
    pub uri: Uri,
    pub etag: Option<String>,
    pub scheduler: Arc<Mutex<Scheduler>>,
    pub db: Arc<Mutex<StateDb>>,
    pub writer: Arc<FileWriter>,
    /// Dimensione tipica dei segmenti di questo link: soglia per il
    /// tail-splitting (§4.4: "residuo > 2 × size(link_idle)").
    pub segment_size_for_stealing: u64,
    /// Condiviso tra tutti i worker dello stesso link (N connessioni,
    /// stesso `link_key`, vedi `orchestrator.rs`): un fallimento di
    /// trasporto su una connessione conta per tutto il link (§4.4).
    pub circuit_breaker: Arc<Mutex<CircuitBreaker>>,
    /// Condiviso tra **tutti** i worker del download, non solo quelli di
    /// un link (§6.2, pulsante Pausa): niente nuove lease finché è a
    /// `true`. I trasferimenti già in volo finiscono, non vengono
    /// interrotti a metà.
    pub paused: Arc<std::sync::atomic::AtomicBool>,
    /// Come `paused`, ma per il pulsante Annulla: interrompe anche connect,
    /// retry o body in corso e ritorna `Ok` (fermata pulita, non un errore).
    pub cancelled: Arc<std::sync::atomic::AtomicBool>,
    /// Controllo live del singolo link. Diversamente dalla pausa globale,
    /// quando passa a `false` interrompe anche la richiesta in volo e
    /// rimette il segmento in coda perché un altro link possa riprenderlo.
    pub enabled: Arc<std::sync::atomic::AtomicBool>,
}

pub async fn run_worker(ctx: WorkerContext) -> Result<(), DualBondError> {
    use std::sync::atomic::Ordering;
    loop {
        if ctx.cancelled.load(Ordering::Relaxed) {
            return Ok(());
        }
        if !ctx.enabled.load(Ordering::Acquire) {
            if let Some(result) = scheduler_terminal_result(&ctx) {
                return result;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            continue;
        }
        if ctx.paused.load(Ordering::Relaxed) {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            continue;
        }

        let breaker_available = ctx
            .circuit_breaker
            .lock()
            .unwrap()
            .is_available(Instant::now());
        if !breaker_available {
            // Link escluso dal circuit breaker (§4.4): niente nuove lease
            // finché il backoff non scade. Il segmento in questione resta
            // disponibile per altri link nel frattempo.
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            continue;
        }

        let work = {
            let mut sched = ctx.scheduler.lock().unwrap();
            if let Some(lease) = sched.lease_next(&ctx.link_key) {
                Some((lease, false))
            } else if let Some(steal) =
                sched.try_steal_tail(&ctx.link_key, ctx.segment_size_for_stealing)
            {
                let db = ctx.db.lock().unwrap();
                db.update_chunk_end(ctx.download_id, steal.victim_idx, steal.victim_new_end)?;
                db.insert_chunk(
                    ctx.download_id,
                    steal.lease.idx,
                    steal.lease.start,
                    steal.lease.end,
                    0,
                    "leased",
                )?;
                Some((steal.lease, true))
            } else {
                None
            }
        };

        let (lease, from_steal) = match work {
            Some(x) => x,
            None => {
                // Guardia isolata in un blocco proprio: deve essere fuori
                // scope PRIMA dell'.await sotto, altrimenti il future del
                // worker non è Send (MutexGuard non lo è) e tokio::spawn
                // non compila.
                let outcome = scheduler_terminal_result(&ctx);
                if let Some(result) = outcome {
                    return result;
                }
                // Coda vuota ma altri worker ancora in volo: nulla da rubare
                // in questo istante, ma potrebbe comparire progresso a
                // breve. Poll breve invece di un meccanismo di notifica
                // dedicato — la spec non impone un limite di latenza qui.
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                continue;
            }
        };

        if !from_steal {
            ctx.db
                .lock()
                .unwrap()
                .set_chunk_state(ctx.download_id, lease.idx, "leased")?;
        }

        let uri = ctx.uri.clone();
        let etag_for_req = ctx.etag.clone();
        let (resume_from, seg_end) = (lease.resume_from, lease.end);
        let build_req = move || {
            let mut builder = empty_request(Method::GET, &uri)?
                .header(RANGE, format!("bytes={resume_from}-{seg_end}"));
            if let Some(etag) = &etag_for_req {
                builder = builder.header(IF_RANGE, etag.as_str());
            }
            builder
                .body(empty_body())
                .map_err(|e| DualBondError::Http(e.to_string()))
        };

        let response = tokio::select! {
            result = send_via_connector_with_retry(&ctx.connector, &ctx.uri, build_req) => Some(result),
            _ = wait_for_live_stop(&ctx.cancelled, &ctx.enabled) => None,
        };
        let Some(response) = response else {
            requeue_segment(&ctx, lease.idx)?;
            if ctx.cancelled.load(Ordering::Acquire) {
                return Ok(());
            }
            continue;
        };

        let resp = match response {
            Ok(r) => r,
            Err(DualBondError::Transport(msg)) => {
                // §4.7: timeout/connection reset -> segmento rimesso in
                // coda, non abort dell'intero download. Un altro worker (o
                // questo, al giro dopo) lo riprende da `resume_from`. Conta
                // sia per il circuit breaker del link sia per il tetto di
                // tentativi totali del segmento (§4.4, gap chiuso dopo il
                // test con hardware reale — vedi docs/milestone4-notes.md).
                ctx.circuit_breaker
                    .lock()
                    .unwrap()
                    .record_failure(Instant::now());
                let outcome = ctx.scheduler.lock().unwrap().record_failure(lease.idx);
                ctx.db.lock().unwrap().set_chunk_state(
                    ctx.download_id,
                    lease.idx,
                    segment_state_str(outcome),
                )?;
                tracing::warn!(link = %ctx.link_key, segment = lease.idx, error = %msg, "segmento rimesso in coda dopo fallimento di trasporto");
                crate::core::events::push(
                    "warn",
                    format!("{}: segmento rimesso in coda ({msg})", ctx.link_key),
                );
                if outcome == SegmentState::Failed {
                    return Err(DualBondError::Http(format!(
                        "segmento {}-{} irraggiungibile dopo troppi tentativi su tutti i link",
                        lease.start, lease.end
                    )));
                }
                continue;
            }
            Err(e) => return Err(e),
        };

        if resp.status() == StatusCode::TOO_MANY_REQUESTS
            || resp.status() == StatusCode::SERVICE_UNAVAILABLE
        {
            // §4.7: rispettare Retry-After, mai trattarlo come errore
            // fatale — il segmento torna in coda, un altro worker (o questo
            // più tardi) lo riprende quando il server è pronto.
            let retry_after = header_str(resp.headers(), &RETRY_AFTER)
                .and_then(|v| v.parse::<u64>().ok())
                .map(std::time::Duration::from_secs)
                .unwrap_or(std::time::Duration::from_secs(5));
            requeue_segment(&ctx, lease.idx)?;
            tracing::warn!(
                link = %ctx.link_key,
                segment = lease.idx,
                status = %resp.status(),
                retry_after_secs = retry_after.as_secs(),
                "429/503: segmento rimesso in coda, back-off"
            );
            crate::core::events::push(
                "warn",
                format!(
                    "Il sito sta limitando i download su {}: rallento e riprovo",
                    ctx.link_key
                ),
            );
            tokio::time::sleep(retry_after).await;
            continue;
        }

        if resp.status() != StatusCode::PARTIAL_CONTENT {
            requeue_segment(&ctx, lease.idx)?;
            return Err(DualBondError::Http(format!(
                "atteso 206 per il segmento {}-{}, ricevuto {}: risorsa probabilmente cambiata",
                lease.start,
                lease.end,
                resp.status()
            )));
        }
        if let (Some(expected), Some(got)) = (&ctx.etag, header_str(resp.headers(), &ETAG)) {
            if expected != &got {
                // ETag divergente su QUESTO link: esclude il link dal
                // download corrente (§4.7), non annulla tutto il download.
                requeue_segment(&ctx, lease.idx)?;
                return Err(DualBondError::Http(format!(
                    "ETag divergente sul link {}: atteso {expected}, ricevuto {got}",
                    ctx.link_key
                )));
            }
        }

        let streamed = tokio::select! {
            result = stream_body_to_file(resp, &ctx.writer, lease.resume_from, |_| {}) => Some(result),
            _ = wait_for_live_stop(&ctx.cancelled, &ctx.enabled) => None,
        };
        let Some(streamed) = streamed else {
            // Il future di streaming viene eliminato qui: il body/il socket
            // HTTP sono chiusi e i byte parziali verranno sovrascritti alla
            // prossima lease, perché il watermark durevole non è avanzato.
            requeue_segment(&ctx, lease.idx)?;
            if ctx.cancelled.load(Ordering::Acquire) {
                return Ok(());
            }
            continue;
        };
        let written = streamed?;
        if ctx.cancelled.load(Ordering::Acquire) || !ctx.enabled.load(Ordering::Acquire) {
            requeue_segment(&ctx, lease.idx)?;
            if ctx.cancelled.load(Ordering::Acquire) {
                return Ok(());
            }
            continue;
        }
        let expected_len = lease.end - lease.resume_from + 1;
        if written != expected_len {
            requeue_segment(&ctx, lease.idx)?;
            return Err(DualBondError::Http(format!(
                "segmento {}-{} troncato: attesi {expected_len} byte, ricevuti {written}",
                lease.start, lease.end
            )));
        }

        ctx.writer.sync()?;
        let segment_len = lease.end - lease.start + 1;
        {
            let mut sched = ctx.scheduler.lock().unwrap();
            sched.report_progress(lease.idx, segment_len);
            sched.complete(lease.idx);
        }
        {
            let db = ctx.db.lock().unwrap();
            db.update_chunk_written(ctx.download_id, lease.idx, segment_len)?;
            db.mark_chunk_done(ctx.download_id, lease.idx)?;
        }
        ctx.circuit_breaker.lock().unwrap().record_success();
    }
}

fn scheduler_terminal_result(ctx: &WorkerContext) -> Option<Result<(), DualBondError>> {
    let sched = ctx.scheduler.lock().unwrap();
    if sched.all_done() {
        Some(Ok(()))
    } else if let Some(failed) = sched.any_failed() {
        Some(Err(DualBondError::Http(format!(
            "segmento {}-{} irraggiungibile dopo troppi tentativi su tutti i link",
            failed.start, failed.end
        ))))
    } else {
        None
    }
}

/// Completa quando il download viene annullato o questo specifico link
/// viene escluso. Il polling breve rende cancellabili anche connect/retry e
/// lettura del body, che altrimenti potrebbero restare sospesi a lungo.
async fn wait_for_live_stop(
    cancelled: &std::sync::atomic::AtomicBool,
    enabled: &std::sync::atomic::AtomicBool,
) {
    use std::sync::atomic::Ordering;
    loop {
        if cancelled.load(Ordering::Acquire) || !enabled.load(Ordering::Acquire) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

fn segment_state_str(state: SegmentState) -> &'static str {
    match state {
        SegmentState::Free => "free",
        SegmentState::Leased => "leased",
        SegmentState::Done => "done",
        SegmentState::Failed => "failed",
    }
}

fn requeue_segment(ctx: &WorkerContext, idx: i64) -> Result<(), DualBondError> {
    ctx.scheduler.lock().unwrap().release_lease(idx);
    ctx.db
        .lock()
        .unwrap()
        .set_chunk_state(ctx.download_id, idx, "free")
}

fn header_str(headers: &hyper::HeaderMap, name: &hyper::header::HeaderName) -> Option<String> {
    headers.get(name)?.to_str().ok().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::scheduler::Scheduler;
    use crate::download::test_server::{self, Scenario, TEST_ETAG};

    fn temp_path(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("dualbond-worker-{}-{}", std::process::id(), name));
        p
    }

    /// Le prime 2 richieste ricevono 429 con `Retry-After: 0` (il test non
    /// vuole aspettare sul serio): il worker deve rimettere il segmento in
    /// coda e ritentare, non abortire l'intero download (§4.7).
    #[tokio::test]
    async fn worker_absorbs_429_and_completes_after_retry_after() {
        let body: Vec<u8> = (0..50_000u32).map(|i| (i % 256) as u8).collect();
        let server = test_server::spawn(Scenario::RateLimitedThenRangeSupported {
            limited_first: 2,
            retry_after_secs: 0,
            body: body.clone(),
        })
        .await;
        let url = format!("http://127.0.0.1:{}/rl.bin", server.port);
        let total = body.len() as u64;
        let dest = temp_path("rl.bin");

        let db = Arc::new(Mutex::new(StateDb::open_in_memory().unwrap()));
        let download_id = db
            .lock()
            .unwrap()
            .create_download_with_segments(
                &url,
                &url,
                &dest.to_string_lossy(),
                total,
                Some(TEST_ETAG),
                None,
                None,
                &[(0, total - 1)],
            )
            .unwrap();
        let writer = Arc::new(FileWriter::create(&dest, total).unwrap());
        let scheduler = Arc::new(Mutex::new(Scheduler::from_segments(vec![
            crate::download::scheduler::Segment {
                idx: 0,
                start: 0,
                end: total - 1,
                written: 0,
                state: crate::download::scheduler::SegmentState::Free,
                lease_link: None,
                attempts: 0,
            },
        ])));

        let ctx = WorkerContext {
            download_id,
            link_key: "only".into(),
            connector: LinkConnector::Plain,
            uri: url.parse().unwrap(),
            etag: Some(TEST_ETAG.to_string()),
            scheduler: scheduler.clone(),
            db: db.clone(),
            writer: writer.clone(),
            segment_size_for_stealing: total,
            circuit_breaker: Arc::new(Mutex::new(CircuitBreaker::new())),
            paused: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            enabled: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        };

        run_worker(ctx)
            .await
            .unwrap_or_else(|e| panic!("il worker doveva assorbire i 429: {e}"));

        drop(writer);
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert!(scheduler.lock().unwrap().all_done());

        std::fs::remove_file(&dest).ok();
        server.stop();
    }

    #[tokio::test]
    async fn live_stop_reacts_quickly_to_link_disable() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let cancelled = AtomicBool::new(false);
        let enabled = AtomicBool::new(true);
        let disable = async {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            enabled.store(false, Ordering::Release);
        };

        tokio::time::timeout(std::time::Duration::from_millis(150), async {
            tokio::join!(wait_for_live_stop(&cancelled, &enabled), disable)
        })
        .await
        .expect("lo switch deve interrompere connect/body entro un breve polling");
    }

    #[tokio::test]
    async fn live_stop_reacts_quickly_to_download_cancel() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let cancelled = AtomicBool::new(false);
        let enabled = AtomicBool::new(true);
        let cancel = async {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            cancelled.store(true, Ordering::Release);
        };

        tokio::time::timeout(std::time::Duration::from_millis(150), async {
            tokio::join!(wait_for_live_stop(&cancelled, &enabled), cancel)
        })
        .await
        .expect("Annulla deve interrompere connect/body entro un breve polling");
    }

    #[tokio::test]
    async fn disabling_a_link_aborts_an_in_flight_request_and_requeues_its_segment() {
        use std::sync::atomic::{AtomicBool, Ordering};

        // Accetta TCP ma non invia mai una risposta HTTP: il worker resta
        // certamente dentro la richiesta finché lo switch non la annulla.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        });

        let url = format!("http://127.0.0.1:{port}/blocked.bin");
        let total = 1024u64;
        let dest = temp_path("disable-in-flight.bin");
        let db = Arc::new(Mutex::new(StateDb::open_in_memory().unwrap()));
        let download_id = db
            .lock()
            .unwrap()
            .create_download_with_segments(
                &url,
                &url,
                &dest.to_string_lossy(),
                total,
                None,
                None,
                None,
                &[(0, total - 1)],
            )
            .unwrap();
        let scheduler = Arc::new(Mutex::new(Scheduler::new(total, total)));
        let enabled = Arc::new(AtomicBool::new(true));
        let cancelled = Arc::new(AtomicBool::new(false));
        let ctx = WorkerContext {
            download_id,
            link_key: "switchable".into(),
            connector: LinkConnector::Plain,
            uri: url.parse().unwrap(),
            etag: None,
            scheduler: scheduler.clone(),
            db,
            writer: Arc::new(FileWriter::create(&dest, total).unwrap()),
            segment_size_for_stealing: total,
            circuit_breaker: Arc::new(Mutex::new(CircuitBreaker::new())),
            paused: Arc::new(AtomicBool::new(false)),
            cancelled: cancelled.clone(),
            enabled: enabled.clone(),
        };
        let worker = tokio::spawn(run_worker(ctx));

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if scheduler.lock().unwrap().segments()[0].state == SegmentState::Leased {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("il worker deve prendere il segmento prima del test dello switch");

        enabled.store(false, Ordering::Release);
        tokio::time::timeout(std::time::Duration::from_millis(250), async {
            loop {
                if scheduler.lock().unwrap().segments()[0].state == SegmentState::Free {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("disabilitare il link deve chiudere la richiesta e liberare il segmento");

        cancelled.store(true, Ordering::Release);
        tokio::time::timeout(std::time::Duration::from_millis(250), worker)
            .await
            .expect("il worker deve terminare dopo Annulla")
            .unwrap()
            .unwrap();
        server.abort();
        std::fs::remove_file(&dest).ok();
    }
}
