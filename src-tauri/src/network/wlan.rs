use std::ffi::c_void;

use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_INVALID_STATE, ERROR_NOT_FOUND, ERROR_SUCCESS, HANDLE,
};
use windows_sys::Win32::NetworkManagement::WiFi::{
    wlan_intf_opcode_current_connection, WlanCloseHandle, WlanEnumInterfaces, WlanFreeMemory,
    WlanOpenHandle, WlanQueryInterface, DOT11_SSID, WLAN_CONNECTION_ATTRIBUTES,
    WLAN_INTERFACE_INFO_LIST, WLAN_OPCODE_VALUE_TYPE,
};

use crate::core::errors::{win32_err, DualBondError};

/// Converte `DOT11_SSID` (§6 delle note tecniche) in `String`. Il campo
/// `ucSSID` non è garantito UTF-8 dallo standard 802.11, ma nella pratica
/// lo è quasi sempre: si usa `from_utf8_lossy` per non fallire mai.
pub fn ssid_to_string(ssid: &DOT11_SSID) -> String {
    let len = (ssid.uSSIDLength as usize).min(ssid.ucSSID.len());
    String::from_utf8_lossy(&ssid.ucSSID[..len]).into_owned()
}

/// Confronta due `GUID` campo per campo (il tipo non deriva `PartialEq`
/// in windows-sys).
pub fn guid_eq(a: &GUID, b: &GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}

/// Fa il parsing di una stringa GUID Windows nella forma
/// `"{AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE}"` (con o senza graffe), come
/// restituita in `AdapterName`/`WLAN_INTERFACE_INFO.InterfaceGuid` per il
/// match tra adapter IpHelper e interfaccia WLAN (§6 delle note tecniche).
pub fn parse_guid_braced(s: &str) -> Option<GUID> {
    let inner = s.strip_prefix('{').and_then(|r| r.strip_suffix('}')).unwrap_or(s);
    let hex: String = inner.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let value = u128::from_str_radix(&hex, 16).ok()?;
    Some(GUID::from_u128(value))
}

/// Codici di `WlanQueryInterface` che significano solo "qui non c'è una
/// connessione Wi-Fi da leggere": `ERROR_INVALID_STATE` (scheda non
/// associata) e `ERROR_NOT_FOUND` (adattatore 802.11 che il servizio WLAN
/// non gestisce, come il "Microsoft Wi-Fi Direct Virtual Adapter").
fn means_no_connection(ret: u32) -> bool {
    ret == ERROR_INVALID_STATE || ret == ERROR_NOT_FOUND
}

#[derive(Debug, Clone)]
pub struct WifiInfo {
    pub ssid: String,
    pub signal_percent: u32,
    pub bssid: [u8; 6],
}

fn u16_array_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// Handle verso `wlanapi.dll` (§6 delle note tecniche). Chiude sempre con
/// `WlanCloseHandle` via `Drop`.
pub struct WlanHandle(HANDLE);

// Sicuro: un HANDLE Win32 è un valore opaco, spostabile liberamente tra
// thread (la API non lo lega al thread che l'ha aperto). Non è `Sync`
// implicitamente derivato: usarlo da più thread in contemporanea non è
// coperto, ma il monitor (`metrics::monitor`) ne possiede uno solo,
// spostato dentro un unico task di background, mai condiviso.
unsafe impl Send for WlanHandle {}

impl WlanHandle {
    pub fn open() -> Result<Self, DualBondError> {
        let mut negotiated = 0u32;
        let mut handle: HANDLE = std::ptr::null_mut();
        let ret =
            unsafe { WlanOpenHandle(2, std::ptr::null(), &mut negotiated, &mut handle) };
        if ret != ERROR_SUCCESS {
            return Err(win32_err("WlanOpenHandle", ret));
        }
        Ok(WlanHandle(handle))
    }

    /// Elenca le interfacce WLAN come `(GUID, descrizione)`. Il `GUID` va
    /// confrontato con quello parsato da `AdapterName` per collegare
    /// l'interfaccia WLAN all'`IfIndex` di `GetAdaptersAddresses`.
    pub fn list_interfaces(&self) -> Result<Vec<(GUID, String)>, DualBondError> {
        let mut list_ptr: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        let ret = unsafe { WlanEnumInterfaces(self.0, std::ptr::null(), &mut list_ptr) };
        if ret != ERROR_SUCCESS {
            return Err(win32_err("WlanEnumInterfaces", ret));
        }
        let result = unsafe {
            let list = &*list_ptr;
            let items = std::slice::from_raw_parts(
                list.InterfaceInfo.as_ptr(),
                list.dwNumberOfItems as usize,
            );
            items
                .iter()
                .map(|i| (i.InterfaceGuid, u16_array_to_string(&i.strInterfaceDescription)))
                .collect::<Vec<_>>()
        };
        unsafe { WlanFreeMemory(list_ptr as *const c_void) };
        Ok(result)
    }

    /// Interroga SSID/segnale della connessione corrente per l'interfaccia
    /// `guid`. Le risposte "nessuna connessione da leggere" (vedi
    /// `means_no_connection`) non sono errori di sistema: si mappano a `Ok(None)`.
    pub fn query_current_connection(&self, guid: &GUID) -> Result<Option<WifiInfo>, DualBondError> {
        let mut data_size = 0u32;
        let mut data_ptr: *mut c_void = std::ptr::null_mut();
        let mut value_type: WLAN_OPCODE_VALUE_TYPE = 0;
        let ret = unsafe {
            WlanQueryInterface(
                self.0,
                guid,
                wlan_intf_opcode_current_connection,
                std::ptr::null(),
                &mut data_size,
                &mut data_ptr,
                &mut value_type,
            )
        };
        if means_no_connection(ret) {
            return Ok(None);
        }
        if ret == ERROR_ACCESS_DENIED {
            return Err(DualBondError::LocationServicesRequired);
        }
        if ret != ERROR_SUCCESS {
            return Err(win32_err("WlanQueryInterface", ret));
        }
        let result = unsafe {
            let attrs = &*(data_ptr as *const WLAN_CONNECTION_ATTRIBUTES);
            let assoc = &attrs.wlanAssociationAttributes;
            WifiInfo {
                ssid: ssid_to_string(&assoc.dot11Ssid),
                signal_percent: assoc.wlanSignalQuality,
                bssid: assoc.dot11Bssid,
            }
        };
        unsafe { WlanFreeMemory(data_ptr as *const c_void) };
        Ok(Some(result))
    }
}

impl Drop for WlanHandle {
    fn drop(&mut self) {
        unsafe { WlanCloseHandle(self.0, std::ptr::null()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_associated_and_not_a_wlan_interface_mean_no_connection() {
        // 5023: scheda Wi-Fi non associata; 1168: GUID che il servizio WLAN
        // non gestisce (es. "Microsoft Wi-Fi Direct Virtual Adapter")
        assert!(means_no_connection(ERROR_INVALID_STATE));
        assert!(means_no_connection(ERROR_NOT_FOUND));
        assert!(!means_no_connection(ERROR_ACCESS_DENIED));
        assert!(!means_no_connection(ERROR_SUCCESS));
    }

    #[test]
    fn ssid_to_string_reads_only_declared_length() {
        let mut buf = [0u8; 32];
        buf[..8].copy_from_slice(b"MyWifi5G");
        let ssid = DOT11_SSID {
            uSSIDLength: 8,
            ucSSID: buf,
        };
        assert_eq!(ssid_to_string(&ssid), "MyWifi5G");
    }

    #[test]
    fn ssid_to_string_ignores_trailing_garbage_past_length() {
        let mut buf = [0u8; 32];
        buf[..2].copy_from_slice(b"Hi");
        buf[2] = 0xFF; // byte non-UTF8 oltre uSSIDLength, non deve influire
        let ssid = DOT11_SSID {
            uSSIDLength: 2,
            ucSSID: buf,
        };
        assert_eq!(ssid_to_string(&ssid), "Hi");
    }

    #[test]
    fn guid_eq_true_for_identical_guids() {
        let g = GUID {
            data1: 1,
            data2: 2,
            data3: 3,
            data4: [4, 5, 6, 7, 8, 9, 10, 11],
        };
        assert!(guid_eq(&g, &g));
    }

    #[test]
    fn guid_eq_false_when_data4_differs() {
        let a = GUID {
            data1: 1,
            data2: 2,
            data3: 3,
            data4: [4, 5, 6, 7, 8, 9, 10, 11],
        };
        let mut b = a;
        b.data4[7] = 99;
        assert!(!guid_eq(&a, &b));
    }

    #[test]
    fn parse_guid_braced_parses_standard_form() {
        let g = parse_guid_braced("{4D36E972-E325-11CE-BFC1-08002BE10318}").unwrap();
        assert_eq!(g.data1, 0x4D36E972);
        assert_eq!(g.data2, 0xE325);
        assert_eq!(g.data3, 0x11CE);
        assert_eq!(g.data4, [0xBF, 0xC1, 0x08, 0x00, 0x2B, 0xE1, 0x03, 0x18]);
    }

    #[test]
    fn parse_guid_braced_accepts_without_braces() {
        let g = parse_guid_braced("4D36E972-E325-11CE-BFC1-08002BE10318").unwrap();
        assert_eq!(g.data1, 0x4D36E972);
    }

    #[test]
    fn parse_guid_braced_rejects_malformed_input() {
        assert!(parse_guid_braced("not-a-guid").is_none());
        assert!(parse_guid_braced("{4D36E972-E325-11CE-BFC1}").is_none());
    }
}
