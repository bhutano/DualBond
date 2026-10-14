use std::fs::OpenOptions;
use std::io;
use std::path::Path;

pub fn init() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

/// In un servizio Windows stdout/stderr non sono normalmente osservabili.
/// Manteniamo quindi una traccia persistente dei flussi e degli errori di
/// trasporto nel percorso di stato del servizio.
pub fn init_file(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    let writer = move || {
        file.try_clone()
            .expect("impossibile duplicare il file di log DualBond")
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(filter)
        .with_writer(writer)
        .try_init()
        .map_err(io::Error::other)
}
