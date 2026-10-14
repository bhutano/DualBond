use crate::core::errors::DualBondError;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

/// Hash SHA-256 in streaming: i byte vengono passati man mano che arrivano
/// dalla rete, senza dover tenere l'intero file in memoria (§10 della spec:
/// verifica finale opzionale, in streaming e interrompibile).
#[derive(Default)]
pub struct StreamingSha256 {
    hasher: Sha256,
}

impl StreamingSha256 {
    pub fn new() -> Self {
        Self { hasher: Sha256::new() }
    }

    pub fn update(&mut self, chunk: &[u8]) {
        self.hasher.update(chunk);
    }

    pub fn finalize_hex(self) -> String {
        let digest = self.hasher.finalize();
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Ricalcola l'hash leggendo il file da disco in streaming (buffer fisso,
/// nessun caricamento in memoria dell'intero file). Usata a fine download
/// risumibile, dove l'hash non può essere accumulato in memoria durante il
/// trasferimento perché un crash a metà perderebbe lo stato dell'hasher
/// (§10 della spec: verifica finale opzionale, in streaming).
pub fn sha256_of_file(path: &Path) -> Result<String, DualBondError> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = StreamingSha256::new();
    let mut buf = [0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize_hex())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_matches_known_sha256() {
        let h = StreamingSha256::new();
        assert_eq!(
            h.finalize_hex(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn chunked_updates_match_single_update() {
        let mut chunked = StreamingSha256::new();
        chunked.update(b"abc");
        chunked.update(b"def");

        let mut single = StreamingSha256::new();
        single.update(b"abcdef");

        assert_eq!(chunked.finalize_hex(), single.finalize_hex());
    }

    #[test]
    fn sha256_of_file_matches_streaming_hash() {
        let mut path = std::env::temp_dir();
        path.push(format!("dualbond-checksum-test-{}.bin", std::process::id()));
        let data: Vec<u8> = (0..5_000_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &data).unwrap();

        let mut expected = StreamingSha256::new();
        expected.update(&data);

        assert_eq!(sha256_of_file(&path).unwrap(), expected.finalize_hex());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn known_vector_abc() {
        let mut h = StreamingSha256::new();
        h.update(b"abc");
        assert_eq!(
            h.finalize_hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
