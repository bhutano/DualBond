//! Stato Internet per adattatore ricavato da Windows NCSI/Network List
//! Manager. A differenza di `OperStatus`, distingue un link con solo gateway
//! locale da un percorso realmente utilizzabile verso Internet.

use std::collections::HashMap;

use windows::core::GUID;
use windows::Win32::Networking::NetworkListManager::{
    INetworkListManager, NetworkListManager, NLM_CONNECTIVITY_IPV4_INTERNET,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InternetAccess {
    Internet,
    LocalOnly,
    Unknown,
}

/// Restituisce lo stato NCSI indicizzato tramite il GUID dell'adattatore nella
/// stessa forma usata da `IP_ADAPTER_ADDRESSES.AdapterName`.
pub fn internet_access_by_adapter() -> windows::core::Result<HashMap<String, InternetAccess>> {
    let apartment = ComApartment::initialize()?;
    let manager: INetworkListManager = unsafe {
        CoCreateInstance(&NetworkListManager, None, CLSCTX_ALL)?
    };
    let connections = unsafe { manager.GetNetworkConnections()? };
    let mut result = HashMap::new();

    loop {
        let mut item = [None];
        let mut fetched = 0u32;
        unsafe { connections.Next(&mut item, Some(&mut fetched))? };
        if fetched == 0 {
            break;
        }
        let Some(connection) = item[0].take() else {
            continue;
        };
        let adapter_id = unsafe { connection.GetAdapterId()? };
        let connectivity = unsafe { connection.GetConnectivity()? };
        let access = if connectivity.0 & NLM_CONNECTIVITY_IPV4_INTERNET.0 != 0 {
            InternetAccess::Internet
        } else {
            InternetAccess::LocalOnly
        };
        result.insert(guid_key(adapter_id), access);
    }

    drop(manager);
    drop(apartment);
    Ok(result)
}

pub fn normalize_adapter_guid(value: &str) -> Option<String> {
    let inner = value
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
        .unwrap_or(value);
    let hex: String = inner
        .chars()
        .filter(|character| *character != '-')
        .map(|character| character.to_ascii_lowercase())
        .collect();
    (hex.len() == 32 && hex.chars().all(|character| character.is_ascii_hexdigit()))
        .then_some(hex)
}

fn guid_key(guid: GUID) -> String {
    format!(
        "{:08x}{:04x}{:04x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        guid.data1,
        guid.data2,
        guid.data3,
        guid.data4[0],
        guid.data4[1],
        guid.data4[2],
        guid.data4[3],
        guid.data4[4],
        guid.data4[5],
        guid.data4[6],
        guid.data4[7]
    )
}

struct ComApartment(bool);

impl ComApartment {
    fn initialize() -> windows::core::Result<Self> {
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        result.ok()?;
        Ok(Self(true))
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_adapter_guids_for_ncsi_lookup() {
        assert_eq!(
            normalize_adapter_guid("{4D36E972-E325-11CE-BFC1-08002BE10318}").as_deref(),
            Some("4d36e972e32511cebfc108002be10318")
        );
    }

    #[test]
    fn rejects_malformed_adapter_guids() {
        assert_eq!(normalize_adapter_guid("not-a-guid"), None);
    }

    #[test]
    fn guid_key_matches_ip_helper_text_normalization() {
        let guid = GUID::from_u128(0x4d36e972_e325_11ce_bfc1_08002be10318);
        assert_eq!(
            guid_key(guid),
            normalize_adapter_guid("{4D36E972-E325-11CE-BFC1-08002BE10318}").unwrap()
        );
    }
}
