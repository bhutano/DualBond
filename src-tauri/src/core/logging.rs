use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Dimensione oltre la quale `service.log` viene ruotato in `service.log.1`.
/// Con un solo file di backup il log del servizio occupa al massimo ~2× questo
/// valore (prima di questo limite era cresciuto senza fine, oltre 80 MB).
pub const SERVICE_LOG_MAX_BYTES: u64 = 10 * 1024 * 1024;

pub fn init() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

/// In un servizio Windows stdout/stderr non sono normalmente osservabili.
/// Manteniamo quindi una traccia persistente dei flussi e degli errori di
/// trasporto nel percorso di stato del servizio, con rotazione per dimensione.
pub fn init_file(path: &Path) -> io::Result<()> {
    let file = RotatingFile::open(path.to_path_buf(), SERVICE_LOG_MAX_BYTES)?;
    let shared = Arc::new(Mutex::new(file));
    let writer = move || RotatingWriter(shared.clone());
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(filter)
        .with_writer(writer)
        .try_init()
        .map_err(io::Error::other)
}

/// Scrive ogni panic nel log prima che il processo prosegua (o venga
/// abortito, ad esempio per un panic dentro una callback `extern "system"`).
/// Senza questo hook un crash del servizio non lascia alcuna traccia: il log
/// si interrompe e Windows registra solo l'evento 7034.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!(
            thread = thread.name().unwrap_or("<senza nome>"),
            "panic: {info}\n{backtrace}"
        );
        previous(info);
    }));
}

pub struct RotatingFile {
    path: PathBuf,
    max_bytes: u64,
    file: File,
    written: u64,
}

impl RotatingFile {
    pub fn open(path: PathBuf, max_bytes: u64) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let written = file.metadata()?.len();
        let mut rotating = Self {
            path,
            max_bytes,
            file,
            written,
        };
        if rotating.written >= rotating.max_bytes {
            rotating.rotate()?;
        }
        Ok(rotating)
    }

    fn backup_path(&self) -> PathBuf {
        let mut name = self.path.as_os_str().to_owned();
        name.push(".1");
        PathBuf::from(name)
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.flush()?;
        let backup = self.backup_path();
        let _ = std::fs::remove_file(&backup);
        std::fs::rename(&self.path, &backup)?;
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        self.written = 0;
        Ok(())
    }
}

impl Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.written > 0 && self.written + buf.len() as u64 > self.max_bytes {
            // Se la rotazione fallisce (file bloccato da un altro processo)
            // continuiamo a scrivere sul file corrente: meglio un log lungo
            // che un log perso.
            let _ = self.rotate();
        }
        let n = self.file.write(buf)?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

struct RotatingWriter(Arc<Mutex<RotatingFile>>);

impl Write for RotatingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Un panic mentre il lock è preso non deve azzittire il log per sempre.
        let mut file = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        file.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_log(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("dualbond-log-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("service.log")
    }

    #[test]
    fn rotates_when_next_write_exceeds_limit() {
        let path = temp_log("rotate");
        let mut log = RotatingFile::open(path.clone(), 10).unwrap();
        log.write_all(b"12345678").unwrap();
        log.write_all(b"abcd").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"abcd");
        assert_eq!(std::fs::read(log.backup_path()).unwrap(), b"12345678");
    }

    #[test]
    fn keeps_a_single_backup() {
        let path = temp_log("single-backup");
        let mut log = RotatingFile::open(path.clone(), 4).unwrap();
        log.write_all(b"aaaa").unwrap();
        log.write_all(b"bbbb").unwrap();
        log.write_all(b"cccc").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"cccc");
        assert_eq!(std::fs::read(log.backup_path()).unwrap(), b"bbbb");
    }

    #[test]
    fn rotates_oversized_existing_file_on_open() {
        let path = temp_log("oversized");
        std::fs::write(&path, b"vecchio log molto lungo").unwrap();
        let log = RotatingFile::open(path.clone(), 8).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"");
        assert_eq!(
            std::fs::read(log.backup_path()).unwrap(),
            b"vecchio log molto lungo"
        );
    }

    #[test]
    fn appends_to_existing_small_file() {
        let path = temp_log("append");
        std::fs::write(&path, b"ab").unwrap();
        let mut log = RotatingFile::open(path.clone(), 100).unwrap();
        log.write_all(b"cd").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"abcd");
        assert!(!log.backup_path().exists());
    }
}
