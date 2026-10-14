use serde::Deserialize;
use wmi::{COMLibrary, WMIConnection};

use crate::core::errors::DualBondError;

#[derive(Deserialize, Debug, Clone)]
#[serde(rename = "Win32_NetworkAdapter")]
#[serde(rename_all = "PascalCase")]
pub struct NetworkAdapterWmi {
    pub net_connection_id: Option<String>,
    pub pnp_device_id: Option<String>,
    pub interface_index: Option<u32>,
    pub guid: Option<String>,
}

/// Interroga WMI per tutti i `Win32_NetworkAdapter` (§7 delle note
/// tecniche), usata per collegare `PNPDeviceID` (identità USB stabile)
/// all'`IfIndex` di `GetAdaptersAddresses`.
///
/// Inizializza COM sul thread corrente: **va chiamata da un thread
/// dedicato** (es. `tokio::task::spawn_blocking`), mai dal thread UI dove
/// Tauri/WebView2 gestisce già COM.
pub fn query_network_adapters() -> Result<Vec<NetworkAdapterWmi>, DualBondError> {
    let com_con = COMLibrary::new().map_err(|e| DualBondError::Wmi(e.to_string()))?;
    let wmi_con = WMIConnection::new(com_con).map_err(|e| DualBondError::Wmi(e.to_string()))?;
    wmi_con
        .query()
        .map_err(|e| DualBondError::Wmi(e.to_string()))
}
