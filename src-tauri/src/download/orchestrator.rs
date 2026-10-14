//! Coordina il downloader multi-link (Milestone 4): probe, scheduler,
//! worker uno per link, verifica finale dell'hash. Punto d'ingresso
//! pubblico per l'integrazione futura con CLI/IPC (non ancora cablato lì —
//! resta un `pub async fn` libero, come `resumable_download` prima di
//! esserlo).
//!
//! **Semplificazioni note rispetto alla spec, documentate qui invece che
//! scoperte in silenzio**:
//! - Nessun resume da riavvio: ogni chiamata crea un nuovo `download` (il
//!   resume multi-segmento su DB esistente richiede rivalidazione ETag per
//!   segmento, non ancora implementata — `resumable_download`, Milestone 3,
//!   resta la via da usare per il resume single-link).
//! - Un solo worker per link, non `AimdController` che ne fa partire/salire
//!   N: il controller AIMD (`concurrency.rs`) esiste ma non è ancora
//!   cablato al ciclo di vita del worker pool.
//! - Dimensionamento dei segmenti fissato all'avvio da valori di bootstrap
//!   (nessuna storia EWMA ancora raccolta), non ricalcolato in corsa.
//! - DNS non pinnato per link (vedi `network::connector`).

use crate::core::errors::DualBondError;
use crate::download::chunk::segment_size_bytes;
use crate::download::circuit_breaker::CircuitBreaker;
use crate::download::probe::probe;
use crate::download::scheduler::Scheduler;
use crate::download::worker::{run_worker, WorkerContext};
use crate::network::connector::LinkConnector;
use crate::storage::{sha256_of_file, FileWriter, StateDb};
use hyper::Uri;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Bootstrap prima di avere una storia EWMA reale per nessun link (§4.4:
/// il dimensionamento adattivo si basa su `ewma_bps`/RTT osservati, che
/// all'avvio non esistono ancora). Valori scelti conservativi: 20 Mbps,
/// 30 ms — producono segmenti da qualche MB, non ai due estremi del clamp.
const BOOTSTRAP_EWMA_BPS: f64 = 20_000_000.0;
const BOOTSTRAP_RTT_MS: u64 = 30;

pub struct LinkSpec {
    pub key: String,
    pub connector: LinkConnector,
    /// Stato iniziale del link. I worker vengono creati anche per i link
    /// validati ma momentaneamente esclusi, così lo switch della UI può
    /// abilitarli durante un download già in corso senza riavviarlo.
    pub enabled: bool,
    /// Connessioni concorrenti da aprire su questo link (§4.4: "start: 4
    /// connessioni per link"). Ogni worker apre una connessione nuova per
    /// segmento (nessun pooling, §4.2), quindi N worker con lo stesso
    /// `link_key` **sono** le N connessioni — non serve un meccanismo
    /// separato. Il controllo AIMD (`concurrency.rs`) che fa salire/scendere
    /// questo numero in corsa non è ancora cablato qui: il valore passato
    /// resta fisso per tutta la durata del download.
    pub connections: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiLinkOutcome {
    pub download_id: i64,
    pub bytes_written: u64,
    pub sha256_hex: String,
}

/// Stato osservabile dall'esterno di un download in corso (§6.2 della UI:
/// barra di progresso, stato testuale). Aggiornato dall'orchestrator,
/// letto dal loop di monitor per popolare `StatsSnapshot.download`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadRunState {
    Running,
    Verifying,
    Done,
    Error(String),
    Cancelled,
}

/// Presa esterna su un download in corso: permette a chi l'ha avviato
/// (comando Tauri `start_download`) di leggerne il progresso e di
/// pausare/annullare senza dover attendere il `Future` fino alla fine.
/// `scheduler` è lo **stesso** `Arc` usato dai worker: leggerlo da fuori
/// non serve un canale dedicato, i byte scritti sono già lì.
pub struct DownloadHandle {
    pub download_id: i64,
    pub name: String,
    pub total: Option<u64>,
    pub scheduler: Arc<Mutex<Scheduler>>,
    pub paused: Arc<std::sync::atomic::AtomicBool>,
    pub cancelled: Arc<std::sync::atomic::AtomicBool>,
    /// Controlli live condivisi con tutti i worker. `set_link_enabled`
    /// aggiorna questi flag oltre al registro persistente.
    pub link_enabled: Arc<HashMap<String, Arc<AtomicBool>>>,
    pub state: Arc<Mutex<DownloadRunState>>,
}

pub async fn multi_link_download(
    db: Arc<Mutex<StateDb>>,
    url: &str,
    dest: &Path,
    expected_sha256: Option<&str>,
    links: Vec<LinkSpec>,
) -> Result<MultiLinkOutcome, DualBondError> {
    multi_link_download_with_handle(db, url, dest, expected_sha256, links, None).await
}

/// Come `multi_link_download`, ma se `handle_tx` è dato, manda un
/// `DownloadHandle` al chiamante non appena scheduler/worker sono pronti
/// (prima di attendere il completamento) — è così che `start_download`
/// (comando Tauri) può ritornare subito e lasciare il download andare in
/// background, con pausa/annulla/progresso disponibili da quel momento.
pub async fn multi_link_download_with_handle(
    db: Arc<Mutex<StateDb>>,
    url: &str,
    dest: &Path,
    expected_sha256: Option<&str>,
    links: Vec<LinkSpec>,
    handle_tx: Option<tokio::sync::oneshot::Sender<DownloadHandle>>,
) -> Result<MultiLinkOutcome, DualBondError> {
    if links.is_empty() {
        return Err(DualBondError::Http(
            "nessun link disponibile per il download multi-link".into(),
        ));
    }

    let pr = probe(url).await?;
    let total = pr.total_size.ok_or_else(|| {
        DualBondError::Http("dimensione sconosciuta: multi-link richiede Content-Length".into())
    })?;
    if !pr.supports_ranges {
        return Err(DualBondError::Http(
            "il server non supporta Range: usare il downloader a link singolo (resumable_download)"
                .into(),
        ));
    }
    if total == 0 {
        return Err(DualBondError::Http(
            "file vuoto: nulla da schedulare".into(),
        ));
    }

    let uri: Uri = pr
        .final_url
        .parse()
        .map_err(|e| DualBondError::Http(format!("URL finale non valido: {e}")))?;
    let dest_str = dest.to_string_lossy().to_string();

    let initial_segment_size =
        segment_size_bytes(BOOTSTRAP_EWMA_BPS, Duration::from_millis(BOOTSTRAP_RTT_MS)).min(total);

    let scheduler = Scheduler::new(total, initial_segment_size);
    let segment_ranges: Vec<(u64, u64)> = scheduler
        .segments()
        .iter()
        .map(|s| (s.start, s.end))
        .collect();

    let download_id = {
        let db_guard = db.lock().unwrap();
        db_guard.create_download_with_segments(
            url,
            &pr.final_url,
            &dest_str,
            total,
            pr.etag.as_deref(),
            pr.last_modified.as_deref(),
            expected_sha256,
            &segment_ranges,
        )?
    };

    // §4.9: impedisce la sospensione del sistema per tutta la durata del
    // download (rilasciata al Drop, qualunque sia il percorso d'uscita —
    // completato, fallito, annullato).
    let _stay_awake = crate::core::power::StayAwakeGuard::acquire();

    let writer = Arc::new(FileWriter::create(dest, total)?);
    let scheduler = Arc::new(Mutex::new(scheduler));
    let paused = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let link_enabled = Arc::new(
        links
            .iter()
            .map(|link| (link.key.clone(), Arc::new(AtomicBool::new(link.enabled))))
            .collect::<HashMap<_, _>>(),
    );
    let state = Arc::new(Mutex::new(DownloadRunState::Running));
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| url.to_string());

    crate::core::events::push("info", format!("Download avviato: {name}"));

    if let Some(tx) = handle_tx {
        let _ = tx.send(DownloadHandle {
            download_id,
            name,
            total: Some(total),
            scheduler: scheduler.clone(),
            paused: paused.clone(),
            cancelled: cancelled.clone(),
            link_enabled: link_enabled.clone(),
            state: state.clone(),
        });
    }

    let mut handles = Vec::new();
    let (tx, mut rx) =
        tokio::sync::mpsc::channel::<Result<(), DualBondError>>(links.len().max(1) * 4);
    for link in links {
        let connections = link.connections.max(1);
        let enabled = link_enabled
            .get(&link.key)
            .expect("ogni LinkSpec deve avere un controllo live")
            .clone();
        // Un solo circuit breaker per link, condiviso tra le sue N
        // connessioni (§4.4): un fallimento su una conta per tutto il link.
        let circuit_breaker = Arc::new(Mutex::new(CircuitBreaker::new()));
        for _ in 0..connections {
            let ctx = WorkerContext {
                download_id,
                link_key: link.key.clone(),
                connector: link.connector,
                uri: uri.clone(),
                etag: pr.etag.clone(),
                scheduler: scheduler.clone(),
                db: db.clone(),
                writer: writer.clone(),
                segment_size_for_stealing: initial_segment_size,
                circuit_breaker: circuit_breaker.clone(),
                paused: paused.clone(),
                cancelled: cancelled.clone(),
                enabled: enabled.clone(),
            };
            let tx = tx.clone();
            handles.push(tokio::spawn(async move {
                let _ = tx.send(run_worker(ctx).await).await;
            }));
        }
    }
    drop(tx); // il canale si chiude quando l'ultimo worker ha finito

    // Primo errore fatale da un worker qualunque -> abortisce subito tutti
    // gli altri, invece di lasciarli scrivere sul file in background mentre
    // l'orchestrator è già andato avanti a calcolare l'hash (corruzione
    // silenziosa altrimenti possibile).
    let mut first_error = None;
    let mut remaining = handles.len();
    while remaining > 0 {
        match rx.recv().await {
            Some(Ok(())) => remaining -= 1,
            Some(Err(e)) => {
                first_error = Some(e);
                break;
            }
            None => break,
        }
    }
    if let Some(e) = first_error {
        for h in &handles {
            h.abort();
        }
        for h in handles {
            let _ = h.await;
        }
        db.lock()
            .unwrap()
            .mark_download_status(download_id, "failed")
            .ok();
        *state.lock().unwrap() = DownloadRunState::Error(e.to_string());
        crate::core::events::push("error", format!("Download fallito: {e}"));
        return Err(e);
    }

    if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        // I worker si sono fermati puliti (nessun errore), ma su richiesta
        // esplicita dell'utente: il file resta a metà, di proposito non
        // verificato/marcato "done".
        db.lock()
            .unwrap()
            .mark_download_status(download_id, "paused")
            .ok();
        *state.lock().unwrap() = DownloadRunState::Cancelled;
        return Err(DualBondError::Cancelled);
    }

    drop(writer);

    *state.lock().unwrap() = DownloadRunState::Verifying;
    {
        let db_guard = db.lock().unwrap();
        db_guard.mark_download_status(download_id, "verifying")?;
    }
    let sha256_hex = sha256_of_file(dest)?;
    let db_guard = db.lock().unwrap();
    if let Some(expected) = expected_sha256 {
        if !expected.eq_ignore_ascii_case(&sha256_hex) {
            db_guard.mark_download_status(download_id, "failed")?;
            *state.lock().unwrap() = DownloadRunState::Error(format!(
                "SHA-256 non corrisponde: atteso {expected}, ottenuto {sha256_hex}"
            ));
            return Err(DualBondError::Http(format!(
                "SHA-256 non corrisponde: atteso {expected}, ottenuto {sha256_hex}"
            )));
        }
    }
    db_guard.set_actual_sha256(download_id, &sha256_hex)?;
    db_guard.mark_download_status(download_id, "done")?;
    *state.lock().unwrap() = DownloadRunState::Done;
    crate::core::events::push("info", format!("Download completato: {}", dest.display()));

    Ok(MultiLinkOutcome {
        download_id,
        bytes_written: total,
        sha256_hex,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::test_server::{self, Scenario};
    use sha2::{Digest, Sha256};

    fn sha256_hex(data: &[u8]) -> String {
        Sha256::digest(data)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "dualbond-orchestrator-{}-{}",
            std::process::id(),
            name
        ));
        p
    }

    /// Due "link" (in realtà entrambi via `LinkConnector::Plain` verso lo
    /// stesso server locale — il binding reale a interfacce diverse
    /// richiede hardware multi-NIC, non disponibile in questo ambiente di
    /// sviluppo, vedi `network::connector`) che scaricano concorrentemente
    /// lo stesso file. Segmento piccolo apposta per forzare più lease reali
    /// e contendersi la coda, non un solo segmento preso dal primo worker.
    #[tokio::test]
    async fn two_links_download_the_same_file_concurrently_and_produce_correct_hash() {
        // > 10 MB (dimensione bootstrap del segmento, vedi BOOTSTRAP_*):
        // forza almeno 2 segmenti reali, cosi' il test esercita davvero la
        // contesa multi-worker sulla coda, non un solo segmento preso dal
        // primo worker che arriva.
        let body: Vec<u8> = (0..15_000_000u32).map(|i| ((i * 13) % 256) as u8).collect();
        let expected_hash = sha256_hex(&body);
        let server = test_server::spawn(Scenario::RangeSupported(body.clone())).await;
        let url = format!("http://127.0.0.1:{}/multi.bin", server.port);
        let dest = temp_path("multi.bin");
        let db = Arc::new(Mutex::new(StateDb::open_in_memory().unwrap()));

        let links = vec![
            LinkSpec {
                key: "link-a".into(),
                connector: LinkConnector::Plain,
                enabled: true,
                connections: 2,
            },
            LinkSpec {
                key: "link-b".into(),
                connector: LinkConnector::Plain,
                enabled: true,
                connections: 2,
            },
        ];

        let outcome = multi_link_download(db.clone(), &url, &dest, Some(&expected_hash), links)
            .await
            .unwrap();

        assert_eq!(outcome.sha256_hex, expected_hash);
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert_eq!(
            db.lock()
                .unwrap()
                .get_download(outcome.download_id)
                .unwrap()
                .unwrap()
                .status,
            "done"
        );

        std::fs::remove_file(&dest).ok();
        server.stop();
    }

    #[tokio::test]
    async fn rejects_server_without_range_support() {
        let server = test_server::spawn(Scenario::PlainOk(vec![1u8; 100])).await;
        let url = format!("http://127.0.0.1:{}/plain.bin", server.port);
        let dest = temp_path("norange.bin");
        let db = Arc::new(Mutex::new(StateDb::open_in_memory().unwrap()));

        let err = multi_link_download(
            db,
            &url,
            &dest,
            None,
            vec![LinkSpec {
                key: "only".into(),
                connector: LinkConnector::Plain,
                enabled: true,
                connections: 1,
            }],
        )
        .await
        .unwrap_err();

        assert!(matches!(err, DualBondError::Http(_)));
        server.stop();
    }

    #[tokio::test]
    async fn an_initially_disabled_link_does_not_block_download_completion() {
        let body: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let server = test_server::spawn(Scenario::RangeSupported(body.clone())).await;
        let url = format!("http://127.0.0.1:{}/one-active.bin", server.port);
        let dest = temp_path("one-active.bin");
        let db = Arc::new(Mutex::new(StateDb::open_in_memory().unwrap()));

        let result = tokio::time::timeout(
            Duration::from_secs(3),
            multi_link_download(
                db,
                &url,
                &dest,
                None,
                vec![
                    LinkSpec {
                        key: "active".into(),
                        connector: LinkConnector::Plain,
                        enabled: true,
                        connections: 1,
                    },
                    LinkSpec {
                        key: "disabled".into(),
                        connector: LinkConnector::Plain,
                        enabled: false,
                        connections: 1,
                    },
                ],
            ),
        )
        .await
        .expect("il worker disabilitato non deve tenere aperto il download")
        .unwrap();

        assert_eq!(result.bytes_written, body.len() as u64);
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        std::fs::remove_file(&dest).ok();
        server.stop();
    }

    /// Verifica manuale (richiede hardware reale, if_index/local_ip da
    /// `dualbond doctor`): riproduce esattamente il path reale di
    /// `download-multi` per isolare se il bug WSAEFAULT osservato in CLI è
    /// nell'orchestrator/worker o solo nel setup del runtime della CLI.
    #[tokio::test]
    #[ignore]
    async fn debug_real_multi_link_download_against_hetzner() {
        let db = Arc::new(Mutex::new(StateDb::open_in_memory().unwrap()));
        let dest = temp_path("debug-real.bin");
        let links = vec![LinkSpec {
            key: "wifi".into(),
            connector: LinkConnector::Bound {
                if_index: 10,
                local_ip: "192.168.1.234".parse().unwrap(),
            },
            enabled: true,
            connections: 4,
        }];

        let result = multi_link_download(
            db,
            "https://ash-speed.hetzner.com/100MB.bin",
            &dest,
            None,
            links,
        )
        .await;
        eprintln!("RISULTATO: {result:?}");
        std::fs::remove_file(&dest).ok();
        result.unwrap();
    }

    #[tokio::test]
    async fn rejects_empty_link_list() {
        let server = test_server::spawn(Scenario::RangeSupported(vec![1u8; 100])).await;
        let url = format!("http://127.0.0.1:{}/f.bin", server.port);
        let dest = temp_path("nolinks.bin");
        let db = Arc::new(Mutex::new(StateDb::open_in_memory().unwrap()));

        let err = multi_link_download(db, &url, &dest, None, vec![])
            .await
            .unwrap_err();
        assert!(matches!(err, DualBondError::Http(_)));
        server.stop();
    }
}
