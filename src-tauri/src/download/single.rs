use crate::core::errors::DualBondError;
use crate::download::body_stream::stream_body_to_file;
use crate::download::http_client::{empty_body, empty_request, send_once};
use crate::download::probe::{probe, ProbeResult};
use crate::storage::{FileWriter, StreamingSha256};
use hyper::header::{ETAG, RANGE};
use hyper::{Method, StatusCode, Uri};
use std::path::Path;

/// Dimensione dei segmenti per il downloader a link singolo (Milestone 2).
/// Il dimensionamento adattivo con termine BDP (§4.4) arriva con lo
/// scheduler multi-link in Milestone 4: qui basta un valore fisso
/// ragionevole per esercitare `FileWriter` su offset multipli e file grandi
/// senza tenere l'intero corpo in memoria.
const CHUNK_SIZE: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadOutcome {
    pub bytes_written: u64,
    pub sha256_hex: String,
}

/// Scarica `url` in `dest` con probe + FileWriter sparse + verifica
/// SHA-256 opzionale. Criterio di uscita Milestone 2 (Parte 11 della
/// spec): file da 1 GB scaricato, hash corretto, nessuno stallo da
/// zero-fill su HDD (garantito da `FileWriter::create`, §0.3-D).
pub async fn download_to_file(
    url: &str,
    dest: &Path,
    expected_sha256: Option<&str>,
) -> Result<DownloadOutcome, DualBondError> {
    let pr = probe(url).await?;
    let total = pr.total_size.ok_or_else(|| {
        DualBondError::Http(
            "dimensione sconosciuta (Content-Length assente o chunked): modalità \
             single-connection senza dimensione nota non ancora gestita"
                .into(),
        )
    })?;

    let writer = FileWriter::create(dest, total)?;
    let mut hasher = StreamingSha256::new();

    let written = if pr.supports_ranges {
        download_ranged(&pr, &writer, &mut hasher, total).await?
    } else {
        download_plain(&pr, &writer, &mut hasher).await?
    };

    if written != total {
        return Err(DualBondError::Http(format!(
            "scaricati {written} byte, attesi {total}: risposta troncata"
        )));
    }
    writer.sync()?;

    let sha256_hex = hasher.finalize_hex();
    if let Some(expected) = expected_sha256 {
        if !expected.eq_ignore_ascii_case(&sha256_hex) {
            return Err(DualBondError::Http(format!(
                "SHA-256 non corrisponde: atteso {expected}, ottenuto {sha256_hex}"
            )));
        }
    }

    Ok(DownloadOutcome {
        bytes_written: written,
        sha256_hex,
    })
}

async fn download_ranged(
    pr: &ProbeResult,
    writer: &FileWriter,
    hasher: &mut StreamingSha256,
    total: u64,
) -> Result<u64, DualBondError> {
    let uri: Uri = pr
        .final_url
        .parse()
        .map_err(|e| DualBondError::Http(format!("URL finale non valido: {e}")))?;

    let mut offset = 0u64;
    while offset < total {
        let end = (offset + CHUNK_SIZE - 1).min(total - 1);
        let req = empty_request(Method::GET, &uri)?
            .header(RANGE, format!("bytes={offset}-{end}"))
            .body(empty_body())
            .map_err(|e| DualBondError::Http(e.to_string()))?;

        let resp = send_once(&uri, req).await?;
        if resp.status() != StatusCode::PARTIAL_CONTENT {
            // §0.3-C: un 200 in risposta a una richiesta Range significa che
            // il server la sta ignorando. Abortire senza scrivere un byte.
            return Err(DualBondError::Http(format!(
                "atteso 206 per il segmento {offset}-{end}, ricevuto {}: il server ignora Range",
                resp.status()
            )));
        }
        if let (Some(expected), Some(got)) = (&pr.etag, header_str(resp.headers(), &ETAG)) {
            if expected != &got {
                return Err(DualBondError::Http(format!(
                    "ETag divergente durante il download: atteso {expected}, ricevuto {got}"
                )));
            }
        }

        let written = stream_body_to_file(resp, writer, offset, |data| hasher.update(data)).await?;
        let expected_len = end - offset + 1;
        if written != expected_len {
            return Err(DualBondError::Http(format!(
                "segmento {offset}-{end} troncato: attesi {expected_len} byte, ricevuti {written}"
            )));
        }
        offset = end + 1;
    }
    Ok(offset)
}

async fn download_plain(
    pr: &ProbeResult,
    writer: &FileWriter,
    hasher: &mut StreamingSha256,
) -> Result<u64, DualBondError> {
    let uri: Uri = pr
        .final_url
        .parse()
        .map_err(|e| DualBondError::Http(format!("URL finale non valido: {e}")))?;
    let req = empty_request(Method::GET, &uri)?
        .body(empty_body())
        .map_err(|e| DualBondError::Http(e.to_string()))?;
    let resp = send_once(&uri, req).await?;
    if resp.status() != StatusCode::OK {
        return Err(DualBondError::Http(format!(
            "atteso 200, ricevuto {}",
            resp.status()
        )));
    }
    stream_body_to_file(resp, writer, 0, |data| hasher.update(data)).await
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
        let digest = Sha256::digest(data);
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("dualbond-single-{}-{}", std::process::id(), name));
        p
    }

    #[tokio::test]
    async fn downloads_multi_segment_ranged_file_byte_for_byte() {
        // Corpo più grande di CHUNK_SIZE così il downloader deve fare
        // più richieste Range consecutive e scrivere a offset diversi.
        let body: Vec<u8> = (0..20 * 1024 * 1024u32).map(|i| (i % 256) as u8).collect();
        let expected_hash = sha256_hex(&body);
        let server = test_server::spawn(Scenario::RangeSupported(body.clone())).await;
        let url = format!("http://127.0.0.1:{}/big.bin", server.port);
        let dest = temp_path("ranged.bin");

        let outcome = download_to_file(&url, &dest, Some(&expected_hash)).await.unwrap();

        assert_eq!(outcome.bytes_written, body.len() as u64);
        assert_eq!(outcome.sha256_hex, expected_hash);
        assert_eq!(std::fs::read(&dest).unwrap(), body);

        std::fs::remove_file(&dest).ok();
        server.stop();
    }

    #[tokio::test]
    async fn falls_back_to_plain_download_when_range_is_ignored() {
        let body = vec![42u8; 3000];
        let server = test_server::spawn(Scenario::RangeDeclaredButIgnored(body.clone())).await;
        let url = format!("http://127.0.0.1:{}/whole.bin", server.port);
        let dest = temp_path("plain.bin");

        let outcome = download_to_file(&url, &dest, None).await.unwrap();

        assert_eq!(outcome.bytes_written, body.len() as u64);
        assert_eq!(std::fs::read(&dest).unwrap(), body);

        std::fs::remove_file(&dest).ok();
        server.stop();
    }

    #[tokio::test]
    async fn rejects_wrong_expected_sha256() {
        let body = vec![1u8; 100];
        let server = test_server::spawn(Scenario::PlainOk(body)).await;
        let url = format!("http://127.0.0.1:{}/mismatch.bin", server.port);
        let dest = temp_path("mismatch.bin");

        let err = download_to_file(&url, &dest, Some("0000")).await.unwrap_err();

        assert!(err.to_string().contains("SHA-256 non corrisponde"));
        std::fs::remove_file(&dest).ok();
        server.stop();
    }
}
