use crate::core::errors::DualBondError;
use crate::storage::FileWriter;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::Response;

/// Scrive il body di `resp` su `writer` partendo da `base_offset`,
/// chiamando `on_chunk` per ogni pezzo ricevuto (usato per l'hashing
/// incrementale in Milestone 2, ignorato dal downloader risumibile di
/// Milestone 3 che ricalcola l'hash a fine trasferimento — §10: "verifica
/// finale opzionale, in streaming"). Ritorna i byte effettivamente scritti.
pub async fn stream_body_to_file(
    resp: Response<Incoming>,
    writer: &FileWriter,
    base_offset: u64,
    mut on_chunk: impl FnMut(&[u8]),
) -> Result<u64, DualBondError> {
    let mut body = resp.into_body();
    let mut offset = base_offset;
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|e| DualBondError::Http(format!("errore di streaming: {e}")))?;
        if let Some(data) = frame.data_ref() {
            writer.write_at(offset, data)?;
            on_chunk(data);
            offset += data.len() as u64;
        }
    }
    Ok(offset - base_offset)
}
