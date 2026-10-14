use super::model::{ServiceConfig, SystemBalanceStatus};
use rand::RngExt;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ServicePaths {
    pub root: PathBuf,
    pub config: PathBuf,
    pub state: PathBuf,
    pub token: PathBuf,
    pub log: PathBuf,
}

impl ServicePaths {
    pub fn system() -> Self {
        let base = std::env::var_os("PROGRAMDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
        Self::under(base.join("DualBond"))
    }

    pub fn under(root: PathBuf) -> Self {
        Self {
            config: root.join("service-config.json"),
            state: root.join("service-state.json"),
            token: root.join("control.token"),
            log: root.join("service.log"),
            root,
        }
    }

    pub fn ensure(&self) -> io::Result<()> {
        fs::create_dir_all(&self.root)
    }
}

pub fn load_config(paths: &ServicePaths) -> io::Result<ServiceConfig> {
    load_or_default(&paths.config)
}

pub fn save_config(paths: &ServicePaths, config: &ServiceConfig) -> io::Result<()> {
    write_json_atomic(&paths.config, config)
}

pub fn save_status(paths: &ServicePaths, status: &SystemBalanceStatus) -> io::Result<()> {
    write_json_atomic(&paths.state, status)
}

pub fn load_or_create_token(paths: &ServicePaths) -> io::Result<String> {
    paths.ensure()?;
    if let Ok(existing) = fs::read_to_string(&paths.token) {
        let token = existing.trim();
        if token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Ok(token.to_owned());
        }
    }

    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    let token = hex(&bytes);
    write_text_atomic(&paths.token, &token)?;
    Ok(token)
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

fn load_or_default<T>(path: &Path) -> io::Result<T>
where
    T: DeserializeOwned + Default,
{
    match read_json(path) {
        Ok(value) => Ok(value),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) if error.kind() == io::ErrorKind::InvalidData => Ok(T::default()),
        Err(error) => Err(error),
    }
}

fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<T> {
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let data = serde_json::to_vec_pretty(value)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    write_atomic(path, &data)
}

fn write_text_atomic(path: &Path, value: &str) -> io::Result<()> {
    write_atomic(path, value.as_bytes())
}

fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, data)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths(name: &str) -> ServicePaths {
        let unique = format!("dualbond-{name}-{}", std::process::id());
        ServicePaths::under(std::env::temp_dir().join(unique))
    }

    #[test]
    fn config_round_trip_and_replacement_work() {
        let paths = temp_paths("config");
        let _ = fs::remove_dir_all(&paths.root);
        let mut config = ServiceConfig {
            enabled: true,
            ..Default::default()
        };
        save_config(&paths, &config).unwrap();
        assert!(load_config(&paths).unwrap().enabled);
        config.enabled = false;
        save_config(&paths, &config).unwrap();
        assert!(!load_config(&paths).unwrap().enabled);
        fs::remove_dir_all(paths.root).unwrap();
    }

    #[test]
    fn corrupt_config_falls_back_to_safe_default() {
        let paths = temp_paths("corrupt");
        let _ = fs::remove_dir_all(&paths.root);
        paths.ensure().unwrap();
        fs::write(&paths.config, b"not-json").unwrap();
        assert!(!load_config(&paths).unwrap().enabled);
        fs::remove_dir_all(paths.root).unwrap();
    }

    #[test]
    fn token_is_strong_well_formed_and_persistent() {
        let paths = temp_paths("token");
        let _ = fs::remove_dir_all(&paths.root);
        let first = load_or_create_token(&paths).unwrap();
        let second = load_or_create_token(&paths).unwrap();
        assert_eq!(first.len(), 64);
        assert_eq!(first, second);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        fs::remove_dir_all(paths.root).unwrap();
    }
}
