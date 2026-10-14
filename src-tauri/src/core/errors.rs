use std::fmt;

/// Errore comune per il layer di rete/discovery. Distingue esplicitamente
/// i fallimenti Win32 (codice + nome funzione) dagli errori io/wmi generici
/// perché il messaggio deve restare diagnosticabile da `dualbond doctor`.
#[derive(Debug)]
pub enum DualBondError {
    Win32 { function: &'static str, code: u32 },
    Io(std::io::Error),
    Wmi(String),
    /// `WlanQueryInterface`/`netsh wlan` falliscono con `ERROR_ACCESS_DENIED`
    /// se i Servizi di localizzazione di Windows sono disattivati: da
    /// Windows 10 leggere SSID/BSSID richiede il permesso di posizione
    /// (SSID/BSSID sono usabili per geolocalizzazione). Confermato
    /// riproducendo lo stesso errore con `netsh wlan show interfaces`.
    LocationServicesRequired,
    /// Errori di protocollo/logica HTTP: URL non valido, status inatteso
    /// durante probe/download, troppi redirect, risposta incoerente con
    /// quanto atteso da §4.6/§4.7 della spec.
    Http(String),
    Sqlite(String),
    /// Fallimento di trasporto (connessione, handshake, invio/ricezione a
    /// livello TCP): a differenza di `Http`, che segnala una decisione di
    /// protocollo (status inatteso, ETag divergente — mai da riprovare
    /// ciecamente), questi sono transitori per natura e il chiamante può
    /// ritentare (§4.7 della spec: "Timeout / connection reset → segmento
    /// in coda").
    Transport(String),
    /// Download interrotto dall'utente (pulsante Annulla, §6.2 della UI):
    /// non è un fallimento, va mostrato/loggato diversamente da un errore
    /// vero.
    Cancelled,
}

impl fmt::Display for DualBondError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DualBondError::Win32 { function, code } => {
                write!(f, "{function} fallita (WIN32_ERROR {code})")
            }
            DualBondError::Io(e) => write!(f, "errore IO: {e}"),
            DualBondError::Wmi(msg) => write!(f, "errore WMI: {msg}"),
            DualBondError::LocationServicesRequired => write!(
                f,
                "SSID/segnale non leggibili: attiva Servizi di localizzazione in \
                 Impostazioni → Privacy e sicurezza → Posizione (richiesto da Windows \
                 per leggere lo stato Wi-Fi)"
            ),
            DualBondError::Http(msg) => write!(f, "{msg}"),
            DualBondError::Sqlite(msg) => write!(f, "errore database: {msg}"),
            DualBondError::Transport(msg) => write!(f, "errore di trasporto: {msg}"),
            DualBondError::Cancelled => write!(f, "annullato dall'utente"),
        }
    }
}

impl std::error::Error for DualBondError {}

impl From<std::io::Error> for DualBondError {
    fn from(e: std::io::Error) -> Self {
        DualBondError::Io(e)
    }
}

impl From<rusqlite::Error> for DualBondError {
    fn from(e: rusqlite::Error) -> Self {
        DualBondError::Sqlite(e.to_string())
    }
}

pub fn win32_err(function: &'static str, code: u32) -> DualBondError {
    DualBondError::Win32 { function, code }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win32_error_message_includes_function_name_and_code() {
        let e = win32_err("GetAdaptersAddresses", 111);
        assert_eq!(
            e.to_string(),
            "GetAdaptersAddresses fallita (WIN32_ERROR 111)"
        );
    }

    #[test]
    fn location_services_required_message_mentions_settings_path() {
        let e = DualBondError::LocationServicesRequired;
        assert!(e.to_string().contains("Servizi di localizzazione"));
        assert!(e.to_string().contains("Posizione"));
    }
}
