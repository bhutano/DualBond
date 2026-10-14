use std::ffi::{c_void, CStr};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use windows_sys::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_SUCCESS};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetAdaptersAddresses, GetIfEntry2, GetIpForwardTable2, IP_ADAPTER_ADDRESSES_LH,
    MIB_IF_ROW2, MIB_IPFORWARD_TABLE2,
};
use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows_sys::Win32::Networking::WinSock::{
    ADDRESS_FAMILY, AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR, SOCKADDR_IN, SOCKADDR_IN6,
    SOCKADDR_INET,
};

use crate::core::errors::{win32_err, DualBondError};

/// IANA `ifType` valori stabili non esposti da windows-sys (§2 delle note
/// tecniche). Usati per classificare gli adapter da `GetAdaptersAddresses`.
pub const IF_TYPE_ETHERNET_CSMACD: u32 = 6;
pub const IF_TYPE_IEEE80211: u32 = 71;
pub const IF_TYPE_PPP: u32 = 23;
pub const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
pub const IF_TYPE_TUNNEL: u32 = 131;

/// Adapter da escludere sempre dalla discovery link fisici (loopback,
/// tunnel virtuali): non rappresentano un percorso di rete reale.
pub fn is_excluded_if_type(if_type: u32) -> bool {
    matches!(if_type, IF_TYPE_SOFTWARE_LOOPBACK | IF_TYPE_TUNNEL)
}

/// Legge un `SOCKADDR` (puntatore raw restituito dalle API IpHelper/WinSock)
/// e lo converte in `IpAddr`, leggendo sempre byte-a-byte per evitare
/// confusione di endianness (mai `S_addr: u32` grezzo — vedi §4 delle note).
///
/// # Safety
/// `p` deve puntare a un `SOCKADDR` valido (o essere null) per la durata
/// della chiamata, con `sa_family` coerente con la dimensione reale della
/// struct puntata (`SOCKADDR_IN` per AF_INET, `SOCKADDR_IN6` per AF_INET6).
pub unsafe fn ip_from_sockaddr(p: *const SOCKADDR) -> Option<IpAddr> {
    if p.is_null() {
        return None;
    }
    match (*p).sa_family {
        AF_INET => {
            let sin = &*(p as *const SOCKADDR_IN);
            let b = sin.sin_addr.S_un.S_un_b;
            Some(IpAddr::V4(Ipv4Addr::new(b.s_b1, b.s_b2, b.s_b3, b.s_b4)))
        }
        AF_INET6 => {
            let sin6 = &*(p as *const SOCKADDR_IN6);
            Some(IpAddr::V6(Ipv6Addr::from(sin6.sin6_addr.u.Byte)))
        }
        _ => None,
    }
}

/// Legge un `SOCKADDR_INET` (usato in `NextHop`/`DestinationPrefix.Prefix`
/// di `MIB_IPFORWARD_ROW2`). L'unione ha lo stesso layout byte-per-byte di
/// `SOCKADDR` (family nei primi due byte), quindi si riusa `ip_from_sockaddr`
/// sull'indirizzo del variant `Ipv4` senza bisogno di leggere `si_family`.
///
/// # Safety
/// `a` deve puntare a un `SOCKADDR_INET` inizializzato dall'API Windows.
pub unsafe fn ip_from_sockaddr_inet(a: &SOCKADDR_INET) -> Option<IpAddr> {
    ip_from_sockaddr(std::ptr::addr_of!(a.Ipv4) as *const SOCKADDR)
}

#[derive(Debug, Clone)]
pub struct AdapterInfo {
    pub if_index: u32,
    pub adapter_name: String,
    pub friendly_name: String,
    pub if_type: u32,
    pub oper_status_up: bool,
    pub physical_address: Option<[u8; 6]>,
    pub unicast_ips: Vec<IpAddr>,
}

unsafe fn pwstr_to_string(p: *mut u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while *p.add(len) != 0 {
        len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
}

/// Enumera tutti gli adapter di rete via `GetAdaptersAddresses`, con retry
/// su buffer via `ERROR_BUFFER_OVERFLOW` (§2 delle note tecniche).
pub fn list_adapters() -> Result<Vec<AdapterInfo>, DualBondError> {
    let mut size: u32 = 15_000;
    let mut buf: Vec<u8>;
    loop {
        buf = vec![0u8; size as usize];
        let ret = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC as u32,
                0,
                std::ptr::null(),
                buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH,
                &mut size,
            )
        };
        match ret {
            ERROR_SUCCESS => break,
            ERROR_BUFFER_OVERFLOW => continue,
            other => return Err(win32_err("GetAdaptersAddresses", other)),
        }
    }

    let mut result = Vec::new();
    let mut p = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
    unsafe {
        while !p.is_null() {
            let a = &*p;
            let adapter_name = if a.AdapterName.is_null() {
                String::new()
            } else {
                CStr::from_ptr(a.AdapterName as *const i8)
                    .to_string_lossy()
                    .into_owned()
            };
            let physical_address = if a.PhysicalAddressLength == 6 {
                let mut mac = [0u8; 6];
                mac.copy_from_slice(&a.PhysicalAddress[..6]);
                Some(mac)
            } else {
                None
            };
            let mut unicast_ips = Vec::new();
            let mut up = a.FirstUnicastAddress;
            while !up.is_null() {
                let ua = &*up;
                if let Some(ip) = ip_from_sockaddr(ua.Address.lpSockaddr) {
                    unicast_ips.push(ip);
                }
                up = ua.Next;
            }
            result.push(AdapterInfo {
                if_index: a.Anonymous1.Anonymous.IfIndex,
                adapter_name,
                friendly_name: pwstr_to_string(a.FriendlyName),
                if_type: a.IfType,
                oper_status_up: a.OperStatus == IfOperStatusUp,
                physical_address,
                unicast_ips,
            });
            p = a.Next;
        }
    }
    Ok(result)
}

#[derive(Debug, Clone, Copy)]
pub struct IfCounters {
    pub in_octets: u64,
    pub out_octets: u64,
}

/// Legge i contatori per-interfaccia via `GetIfEntry2` (§4.3, validazione
/// empirica del binding).
pub fn get_if_counters(if_index: u32) -> Result<IfCounters, DualBondError> {
    let mut row: MIB_IF_ROW2 = unsafe { std::mem::zeroed() };
    row.InterfaceIndex = if_index;
    let ret = unsafe { GetIfEntry2(&mut row) };
    if ret != ERROR_SUCCESS {
        return Err(win32_err("GetIfEntry2", ret));
    }
    Ok(IfCounters {
        in_octets: row.InOctets,
        out_octets: row.OutOctets,
    })
}

#[derive(Debug, Clone, Copy)]
pub struct DefaultRoute {
    pub if_index: u32,
    pub next_hop: IpAddr,
}

/// Trova tutte le route di default (`PrefixLength == 0`) via
/// `GetIpForwardTable2` (§3, route/gateway) — una per interfaccia
/// multi-homed, non solo quella con la metrica migliore. `family`
/// tipicamente `AF_INET` o `AF_INET6`.
pub fn get_default_routes(family: ADDRESS_FAMILY) -> Result<Vec<DefaultRoute>, DualBondError> {
    let mut table_ptr: *mut MIB_IPFORWARD_TABLE2 = std::ptr::null_mut();
    let ret = unsafe { GetIpForwardTable2(family, &mut table_ptr) };
    if ret != ERROR_SUCCESS {
        return Err(win32_err("GetIpForwardTable2", ret));
    }

    let result = unsafe {
        let table = &*table_ptr;
        let rows = std::slice::from_raw_parts(table.Table.as_ptr(), table.NumEntries as usize);
        rows.iter()
            .filter(|r| r.DestinationPrefix.PrefixLength == 0)
            .filter_map(|r| {
                ip_from_sockaddr_inet(&r.NextHop).map(|ip| DefaultRoute {
                    if_index: r.InterfaceIndex,
                    next_hop: ip,
                })
            })
            .collect()
    };
    unsafe { FreeMibTable(table_ptr as *const c_void) };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Networking::WinSock::{IN6_ADDR, IN6_ADDR_0, IN_ADDR, IN_ADDR_0};

    #[test]
    fn excludes_loopback_and_tunnel() {
        assert!(is_excluded_if_type(IF_TYPE_SOFTWARE_LOOPBACK));
        assert!(is_excluded_if_type(IF_TYPE_TUNNEL));
    }

    #[test]
    fn does_not_exclude_ethernet_wifi_ppp() {
        assert!(!is_excluded_if_type(IF_TYPE_ETHERNET_CSMACD));
        assert!(!is_excluded_if_type(IF_TYPE_IEEE80211));
        assert!(!is_excluded_if_type(IF_TYPE_PPP));
    }

    #[test]
    fn ip_from_sockaddr_null_returns_none() {
        let result = unsafe { ip_from_sockaddr(std::ptr::null()) };
        assert_eq!(result, None);
    }

    #[test]
    fn ip_from_sockaddr_parses_ipv4() {
        let sin = SOCKADDR_IN {
            sin_family: AF_INET,
            sin_port: 0,
            sin_addr: IN_ADDR {
                S_un: IN_ADDR_0 {
                    S_un_b: windows_sys::Win32::Networking::WinSock::IN_ADDR_0_0 {
                        s_b1: 192,
                        s_b2: 168,
                        s_b3: 1,
                        s_b4: 42,
                    },
                },
            },
            sin_zero: [0; 8],
        };
        let result = unsafe { ip_from_sockaddr(&sin as *const _ as *const SOCKADDR) };
        assert_eq!(result, Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 42))));
    }

    #[test]
    fn ip_from_sockaddr_parses_ipv6() {
        let bytes: [u8; 16] = [
            0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01,
        ];
        let sin6 = SOCKADDR_IN6 {
            sin6_family: AF_INET6,
            sin6_port: 0,
            sin6_flowinfo: 0,
            sin6_addr: IN6_ADDR {
                u: IN6_ADDR_0 { Byte: bytes },
            },
            Anonymous: windows_sys::Win32::Networking::WinSock::SOCKADDR_IN6_0 {
                sin6_scope_id: 0,
            },
        };
        let result = unsafe { ip_from_sockaddr(&sin6 as *const _ as *const SOCKADDR) };
        assert_eq!(
            result,
            Some(IpAddr::V6(Ipv6Addr::from(bytes)))
        );
    }
}
