use crate::core::errors::DualBondError;
use crate::download::body_stream::stream_body_to_file;
use crate::download::http_client::{empty_body, empty_request, send_once, send_once_with_retry};
use crate::download::probe::{probe, ProbeResult};
use crate::storage::{sha256_of_file, DownloadRow, FileWriter, StateDb};
use hyper::header::{ETAG, IF_RANGE, RANGE};
use hyper::{Method, StatusCode, Uri};
use std::path::Path;

/// Stesso dimensionamento fisso di `download::single` (Milestone 2): il
/// sizing adattivo con termine BDP arriva con lo scheduler multi-link
/// (Milestone 4).
const CHUNK_SIZE: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumableOutcome {
    pub download_id: i64,
    pub bytes_written: u64,
    pub sha256_hex: String,
    pub resumed_from: u64,
}

/// Downloader a link singolo con persistenza SQLite, resume byte-preciso
/// e barriera di durabilità (Milestone 3, §4.5 della spec). Interrompere
/// il processo in qualunque momento e richiamare questa funzione con gli
/// stessi `url`/`dest` riprende da `chunk.written`, mai da zero, e non
/// scarica mai un file "completo ma corrotto": o bit-identico, o
/// dichiarato incompleto (criterio di accettazione, Parte 10).
pub async fn resumable_download(
    db: &StateDb,
    url: &str,
    dest: &Path,
    expected_sha256: Option<&str>,
) -> Result<ResumableOutcome, DualBondError> {
    let pr = probe(url).await?;
    let total = pr
        .total_size
        .ok_or_else(|| DualBondError::Http("dimensione sconosciuta: resume non supportato".into()))?;
    let dest_str = dest.to_string_lossy().to_string();

    let existing = db.find_resumable(url, &dest_str)?;
    let can_resume = |row: &DownloadRow| -> bool {
        pr.supports_ranges
            && row.total_size == Some(total)
            && row.etag.is_some()
            && row.etag == pr.etag
            && row.final_url.as_deref() == Some(pr.final_url.as_str())
    };

    let (download_id, resume_offset, writer) = match existing {
        Some(row) if can_resume(&row) => {
            let chunk = db
                .get_chunk(row.id, 0)?
                .ok_or_else(|| DualBondError::Http("stato download corrotto: chunk mancante".into()))?;
            let writer = FileWriter::open_existing(dest)?;
            (row.id, chunk.written, writer)
        }
        Some(row) => {
            // Metadati non rivalidati (§4.5 punto 2): mai fondere byte
            // vecchi e nuovi, si ricomincia da zero.
            db.discard_download(row.id)?;
            let id = create_row(db, url, &pr, total, &dest_str, expected_sha256)?;
            let writer = FileWriter::create(dest, total)?;
            (id, 0, writer)
        }
        None => {
            let id = create_row(db, url, &pr, total, &dest_str, expected_sha256)?;
            let writer = FileWriter::create(dest, total)?;
            (id, 0, writer)
        }
    };
    let resumed_from = resume_offset;

    if pr.supports_ranges {
        download_ranged_resumable(&pr, &writer, db, download_id, total, resume_offset).await?;
    } else {
        download_plain_from_scratch(&pr, &writer).await?;
        writer.sync()?;
        db.update_chunk_written(download_id, 0, total)?;
    }

    db.mark_chunk_done(download_id, 0)?;
    db.mark_download_status(download_id, "verifying")?;
    drop(writer);

    let sha256_hex = sha256_of_file(dest)?;
    if let Some(expected) = expected_sha256 {
        if !expected.eq_ignore_ascii_case(&sha256_hex) {
            db.mark_download_status(download_id, "failed")?;
            return Err(DualBondError::Http(format!(
                "SHA-256 non corrisponde: atteso {expected}, ottenuto {sha256_hex}"
            )));
        }
    }
    db.set_actual_sha256(download_id, &sha256_hex)?;
    db.mark_download_status(download_id, "done")?;

    Ok(ResumableOutcome {
        download_id,
        bytes_written: total,
        sha256_hex,
        resumed_from,
    })
}

fn create_row(
    db: &StateDb,
    url: &str,
    pr: &ProbeResult,
    total: u64,
    dest_str: &str,
    expected_sha256: Option<&str>,
) -> Result<i64, DualBondError> {
    db.create_download(
        url,
        &pr.final_url,
        dest_str,
        total,
        pr.etag.as_deref(),
        pr.last_modified.as_deref(),
        expected_sha256,
    )
}

async fn download_ranged_resumable(
    pr: &ProbeResult,
    writer: &FileWriter,
    db: &StateDb,
    download_id: i64,
    total: u64,
    start_from: u64,
) -> Result<(), DualBondError> {
    let uri: Uri = pr
        .final_url
        .parse()
        .map_err(|e| DualBondError::Http(format!("URL finale non valido: {e}")))?;

    let mut offset = start_from;
    while offset < total {
        let end = (offset + CHUNK_SIZE - 1).min(total - 1);
        let build_req = || {
            let mut builder =
                empty_request(Method::GET, &uri)?.header(RANGE, format!("bytes={offset}-{end}"));
            if let Some(etag) = &pr.etag {
                // If-Range: se la risorsa è cambiata, il server risponde 200
                // (intero file) invece di 206 — intercettato sotto come errore,
                // mai scritto (§4.5 punto 3, §0.3-C).
                builder = builder.header(IF_RANGE, etag.as_str());
            }
            builder.body(empty_body()).map_err(|e| DualBondError::Http(e.to_string()))
        };

        // Ritenta fino a 3 volte, solo su fallimenti di trasporto transitori
        // (connect/handshake/send falliti) — mai su status inatteso o ETag
        // divergente, decisioni di protocollo permanenti (§4.7 della spec).
        let resp = send_once_with_retry(&uri, build_req).await?;
        if resp.status() != StatusCode::PARTIAL_CONTENT {
            return Err(DualBondError::Http(format!(
                "atteso 206 per il segmento {offset}-{end} durante il resume, ricevuto {}: risorsa \
                 probabilmente cambiata",
                resp.status()
            )));
        }
        if let (Some(expected), Some(got)) = (&pr.etag, header_str(resp.headers(), &ETAG)) {
            if expected != &got {
                return Err(DualBondError::Http(format!(
                    "ETag divergente durante il resume: atteso {expected}, ricevuto {got}"
                )));
            }
        }

        let written = stream_body_to_file(resp, writer, offset, |_| {}).await?;
        let expected_len = end - offset + 1;
        if written != expected_len {
            return Err(DualBondError::Http(format!(
                "segmento {offset}-{end} troncato: attesi {expected_len} byte, ricevuti {written}"
            )));
        }

        // Barriera di durabilità (§4.5): fsync PRIMA di dichiarare i byte
        // scritti nel DB. `written` è sempre ≤ ai byte realmente durevoli:
        // un crash qui può solo far ri-scaricare byte già presenti, mai
        // dichiarare completi byte assenti.
        writer.sync()?;
        let new_watermark = end + 1;
        db.update_chunk_written(download_id, 0, new_watermark)?;

        offset = new_watermark;
    }
    Ok(())
}

async fn download_plain_from_scratch(pr: &ProbeResult, writer: &FileWriter) -> Result<(), DualBondError> {
    let uri: Uri = pr
        .final_url
        .parse()
        .map_err(|e| DualBondError::Http(format!("URL finale non valido: {e}")))?;
    let req = empty_request(Method::GET, &uri)?
        .body(empty_body())
        .map_err(|e| DualBondError::Http(e.to_string()))?;
    let resp = send_once(&uri, req).await?;
    if resp.status() != StatusCode::OK {
        return Err(DualBondError::Http(format!("atteso 200, ricevuto {}", resp.status())));
    }
    stream_body_to_file(resp, writer, 0, |_| {}).await?;
    Ok(())
}

fn header_str(headers: &hyper::HeaderMap, name: &hyper::header::HeaderName) -> Option<String> {
    headers.get(name)?.to_str().ok().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::test_server::{self, Scenario};
    use sha2::{Digest, Sha256};

    fn sha256_hex(data: &[u8]) -> String {
        Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("dualbond-resumable-{}-{}", std::process::id(), name));
        p
    }

    #[tokio::test]
    async fn fresh_download_completes_and_matches_hash() {
        let body: Vec<u8> = (0..3_000_000u32).map(|i| (i % 256) as u8).collect();
        let expected = sha256_hex(&body);
        let server = test_server::spawn(Scenario::RangeSupported(body.clone())).await;
        let url = format!("http://127.0.0.1:{}/f.bin", server.port);
        let dest = temp_path("fresh.bin");
        let db = StateDb::open_in_memory().unwrap();

        let outcome = resumable_download(&db, &url, &dest, Some(&expected)).await.unwrap();

        assert_eq!(outcome.resumed_from, 0);
        assert_eq!(outcome.sha256_hex, expected);
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert_eq!(db.get_download(outcome.download_id).unwrap().unwrap().status, "done");

        std::fs::remove_file(&dest).ok();
        server.stop();
    }

    /// Simula un crash interrompendo il task di download a metà
    /// trasferimento (abort del future, come un kill di processo a livello
    /// dei byte già scritti) e verifica che una seconda chiamata riprenda
    /// esattamente da dove il DB dichiara e produca un file finale
    /// bit-identico. Ripetuto con punti di interruzione diversi per
    /// coprire più barriere di durabilità (criterio di uscita Milestone 3:
    /// 100 crash injection senza corruzione).
    #[tokio::test]
    async fn crash_injection_100_times_never_corrupts_final_file() {
        let body: Vec<u8> = (0..25_000_000u32).map(|i| ((i * 7) % 256) as u8).collect();
        let expected_hash = sha256_hex(&body);

        // Un solo server riusato per tutte le iterazioni: aprire 100
        // listener TCP concorrenti (in aggiunta a tutti gli altri test in
        // esecuzione parallela) esaurirebbe le porte effimere e produrrebbe
        // timeout di connessione spuri, non rappresentativi di un vero bug.
        let server = test_server::spawn(Scenario::RangeSupported(body.clone())).await;
        let url = format!("http://127.0.0.1:{}/crash.bin", server.port);

        for i in 0..100u64 {
            let dest = temp_path(&format!("crash-{i}.bin"));
            let db = StateDb::open_in_memory().unwrap();

            // Punto di interruzione variabile: presto/a metà/tardi nel
            // trasferimento, in millisecondi dall'avvio. `timeout` fa
            // cadere (drop) il future di download a metà poll esattamente
            // come un crash di processo interromperebbe la scrittura.
            let abort_after_ms = 1 + (i % 15);
            let _ = tokio::time::timeout(
                std::time::Duration::from_millis(abort_after_ms),
                resumable_download(&db, &url, &dest, None),
            )
            .await;

            // Invariante centrale: se il DB dichiara `written` byte durevoli
            // per il chunk, quei byte devono già combaciare col sorgente
            // (mai completi-ma-corrotti, mai dichiarati oltre il reale).
            if let Some(row) = db.find_resumable(&url, &dest.to_string_lossy()).unwrap() {
                if let Some(chunk) = db.get_chunk(row.id, 0).unwrap() {
                    if chunk.written > 0 {
                        let on_disk = std::fs::read(&dest).unwrap();
                        assert_eq!(
                            &on_disk[..chunk.written as usize],
                            &body[..chunk.written as usize],
                            "iterazione {i}: byte dichiarati durevoli non corrispondono al sorgente"
                        );
                    }
                }
            }

            // Completa il download (riprendendo se necessario) e verifica
            // che il file finale sia bit-identico, mai "completo ma
            // corrotto" (criterio di accettazione non negoziabile, Parte 10).
            let outcome = resumable_download(&db, &url, &dest, Some(&expected_hash))
                .await
                .unwrap_or_else(|e| panic!("iterazione {i}: resume finale fallito: {e}"));
            assert_eq!(outcome.sha256_hex, expected_hash);
            assert_eq!(std::fs::read(&dest).unwrap(), body);

            std::fs::remove_file(&dest).ok();
        }
        server.stop();
    }

    /// Pre-semina DB e file con uno stato "a metà" (come lascerebbe un
    /// crash già passato per la barriera di durabilità) e verifica che il
    /// resume riprenda esattamente da quel watermark, senza ri-scaricare
    /// l'inizio, producendo comunque un file finale bit-identico.
    /// Simula 2 connessioni consecutive che cadono a livello di trasporto
    /// (reset immediato, nessuna risposta) prima che la terza vada a buon
    /// fine: verifica che il retry loop su `DualBondError::Transport` (punto
    /// di ripresa post-pausa) assorba il fallimento transitorio invece di
    /// abortire il download.
    #[tokio::test]
    async fn download_ranged_resumable_retries_transient_transport_failures() {
        let body: Vec<u8> = (0..500_000u32).map(|i| (i % 256) as u8).collect();
        let server = test_server::spawn(Scenario::FlakyThenRangeSupported {
            fail_first: 2,
            body: body.clone(),
        })
        .await;
        let url = format!("http://127.0.0.1:{}/flaky.bin", server.port);
        let total = body.len() as u64;

        let pr = ProbeResult {
            final_url: url.clone(),
            total_size: Some(total),
            supports_ranges: true,
            etag: Some(test_server::TEST_ETAG.to_string()),
            last_modified: None,
            content_type: None,
        };

        let db = StateDb::open_in_memory().unwrap();
        let dest = temp_path("flaky.bin");
        let download_id = db
            .create_download(&url, &pr.final_url, &dest.to_string_lossy(), total, pr.etag.as_deref(), None, None)
            .unwrap();
        let writer = FileWriter::create(&dest, total).unwrap();

        download_ranged_resumable(&pr, &writer, &db, download_id, total, 0)
            .await
            .unwrap_or_else(|e| panic!("il retry doveva assorbire il fallimento transitorio: {e}"));

        drop(writer);
        assert_eq!(std::fs::read(&dest).unwrap(), body);

        std::fs::remove_file(&dest).ok();
        server.stop();
    }

    #[tokio::test]
    async fn resume_continues_from_existing_watermark_without_redownloading_start() {
        let body: Vec<u8> = (0..2_000_000u32).map(|i| (i % 200) as u8).collect();
        let server = test_server::spawn(Scenario::RangeSupported(body.clone())).await;
        let url = format!("http://127.0.0.1:{}/half.bin", server.port);
        let dest = temp_path("half.bin");
        let db = StateDb::open_in_memory().unwrap();

        let pr = probe(&url).await.unwrap();
        let half = body.len() as u64 / 2;

        let writer = FileWriter::create(&dest, body.len() as u64).unwrap();
        writer.write_at(0, &body[..half as usize]).unwrap();
        writer.sync().unwrap();
        drop(writer);

        let download_id = db
            .create_download(
                &url,
                &pr.final_url,
                &dest.to_string_lossy(),
                body.len() as u64,
                pr.etag.as_deref(),
                pr.last_modified.as_deref(),
                None,
            )
            .unwrap();
        db.update_chunk_written(download_id, 0, half).unwrap();

        let outcome = resumable_download(&db, &url, &dest, None).await.unwrap();

        assert_eq!(outcome.resumed_from, half, "doveva riprendere dal watermark preesistente");
        assert_eq!(std::fs::read(&dest).unwrap(), body);

        std::fs::remove_file(&dest).ok();
        server.stop();
    }
}
