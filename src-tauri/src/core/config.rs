//! `%APPDATA%\DualBond\config.toml` (§9 della spec): generato coi
//! default al primo avvio, tutti i campi opzionali — un file mancante o
//! parziale non è un errore, i default coprono ogni buco.

use serde::{Deserialize, Serialize};
use std::path::Path;

fn default_download_directory() -> String {
    String::new() // vuoto = usa la cartella Download di sistema (risolta a runtime, non qui)
}
fn default_max_connections() -> String {
    "auto".into()
}
fn default_segment_size() -> String {
    "auto".into()
}
fn default_checksum_algorithm() -> String {
    "sha256".into()
}
fn default_retry_count() -> u32 {
    3
}
fn default_retry_backoff_ms() -> u64 {
    500
}
fn default_scheduler_interval_ms() -> u64 {
    250
}
fn default_true() -> bool {
    true
}
fn default_theme() -> String {
    "system".into()
}
fn default_chart_window_sec() -> u32 {
    60
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    #[serde(default = "default_download_directory")]
    pub download_directory: String,
    #[serde(default = "default_max_connections")]
    pub max_connections: String,
    #[serde(default = "default_segment_size")]
    pub segment_size: String,
    #[serde(default = "default_checksum_algorithm")]
    pub checksum_algorithm: String,
    #[serde(default = "default_retry_count")]
    pub retry_count: u32,
    #[serde(default = "default_retry_backoff_ms")]
    pub retry_backoff_ms: u64,
    #[serde(default = "default_scheduler_interval_ms")]
    pub scheduler_interval_ms: u64,
    #[serde(default = "default_true")]
    pub prevent_sleep: bool,
    #[serde(default = "default_true")]
    pub auto_resume_on_start: bool,
    #[serde(default = "default_true")]
    pub ask_before_metered: bool,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_chart_window_sec")]
    pub chart_window_sec: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            download_directory: default_download_directory(),
            max_connections: default_max_connections(),
            segment_size: default_segment_size(),
            checksum_algorithm: default_checksum_algorithm(),
            retry_count: default_retry_count(),
            retry_backoff_ms: default_retry_backoff_ms(),
            scheduler_interval_ms: default_scheduler_interval_ms(),
            prevent_sleep: true,
            auto_resume_on_start: true,
            ask_before_metered: true,
            theme: default_theme(),
            chart_window_sec: default_chart_window_sec(),
        }
    }
}

impl Settings {
    /// Legge `path`; se manca, ritorna i default senza errore (§9: "tutti
    /// i campi opzionali", e implicitamente anche il file stesso al primo
    /// avvio). Un file presente ma con TOML invalido resta un errore vero
    /// — meglio far notare un file corrotto che ignorarlo in silenzio.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(contents) => toml::from_str(&contents).map_err(|e| e.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let contents = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, contents).map_err(|e| e.to_string())
    }

    /// `"auto"` -> 4 (bootstrap §4.4), un numero esplicito -> se stesso
    /// clampato a 2..32 (range dichiarato in §9).
    pub fn resolved_max_connections(&self) -> u32 {
        match self.max_connections.parse::<u32>() {
            Ok(n) => n.clamp(2, 32),
            Err(_) => 4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("dualbond-config-test-{}-{}", std::process::id(), name));
        p
    }

    #[test]
    fn load_missing_file_returns_defaults() {
        let path = temp_path("missing.toml");
        std::fs::remove_file(&path).ok();
        let settings = Settings::load(&path).unwrap();
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn save_then_load_roundtrips() {
        let path = temp_path("roundtrip.toml");
        let mut settings = Settings::default();
        settings.max_connections = "8".into();
        settings.theme = "dark".into();
        settings.save(&path).unwrap();

        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded, settings);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn partial_toml_fills_missing_fields_with_defaults() {
        let path = temp_path("partial.toml");
        std::fs::write(&path, "theme = \"light\"\n").unwrap();

        let settings = Settings::load(&path).unwrap();
        assert_eq!(settings.theme, "light");
        assert_eq!(settings.retry_count, default_retry_count());
        assert_eq!(settings.max_connections, "auto");

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn invalid_toml_is_a_real_error_not_silently_ignored() {
        let path = temp_path("invalid.toml");
        std::fs::write(&path, "this is not valid toml {{{").unwrap();
        assert!(Settings::load(&path).is_err());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn resolved_max_connections_defaults_to_4_when_auto() {
        let settings = Settings::default();
        assert_eq!(settings.resolved_max_connections(), 4);
    }

    #[test]
    fn resolved_max_connections_clamps_explicit_values() {
        let mut settings = Settings::default();
        settings.max_connections = "1".into();
        assert_eq!(settings.resolved_max_connections(), 2);
        settings.max_connections = "999".into();
        assert_eq!(settings.resolved_max_connections(), 32);
        settings.max_connections = "10".into();
        assert_eq!(settings.resolved_max_connections(), 10);
    }
}
