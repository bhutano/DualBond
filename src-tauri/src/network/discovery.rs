use std::net::{IpAddr, Ipv4Addr};

use crate::network::connectivity::InternetAccess;
use crate::network::identity::{mac_key, parse_usb_pnp_device_id, usb_key};
use crate::network::iphlpapi::{
    is_excluded_if_type, AdapterInfo, IF_TYPE_ETHERNET_CSMACD, IF_TYPE_IEEE80211,
};
use crate::network::wlan::WifiInfo;

/// Nome e indirizzo dell'adattatore virtuale (Wintun) creato dalla modalità
/// sistema. Non è un'uscita verso Internet ma il tunnel stesso: non deve mai
/// comparire tra i link (altrimenti il registro lo memorizza come connessione,
/// una volta per installazione).
pub const TUNNEL_ADAPTER_NAME: &str = "DualBond";
pub const TUNNEL_IPV4: Ipv4Addr = Ipv4Addr::new(10, 77, 0, 1);

fn is_own_tunnel(adapter: &AdapterInfo) -> bool {
    adapter.friendly_name == TUNNEL_ADAPTER_NAME
        || adapter.unicast_ips.contains(&IpAddr::V4(TUNNEL_IPV4))
}

#[derive(Debug, Clone, PartialEq)]
pub enum LinkKind {
    UsbTether,
    Ethernet,
    Wifi {
        ssid: Option<String>,
        signal_percent: Option<u32>,
    },
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinkCandidate {
    pub if_index: u32,
    pub name: String,
    pub kind: LinkKind,
    pub stable_key: String,
    pub ips: Vec<IpAddr>,
    pub internet_access: InternetAccess,
}

/// Combina un `AdapterInfo` (IpHelper) con il `PNPDeviceID` WMI (per
/// distinguere tethering USB Android da Ethernet reale, entrambi
/// `IF_TYPE_ETHERNET_CSMACD` — §2 delle note tecniche) e, per il Wi-Fi, con
/// l'esito di `WlanQueryInterface` per SSID/segnale.
///
/// Ritorna `None` per gli `IfType` sempre esclusi (loopback, tunnel — §2).
pub fn build_link_candidate(
    adapter: &AdapterInfo,
    pnp_device_id: Option<&str>,
    wifi: Option<&WifiInfo>,
) -> Option<LinkCandidate> {
    // GetAdaptersAddresses conserva indirizzi e route anche per interfacce
    // disconnesse. Senza questo controllo un Wi-Fi spento resta schedulabile
    // con l'ultimo IPv4 DHCP e ogni flusso fallisce con WSAEHOSTUNREACH.
    if !adapter.oper_status_up || is_excluded_if_type(adapter.if_type) || is_own_tunnel(adapter) {
        return None;
    }

    let usb_ids = pnp_device_id.and_then(parse_usb_pnp_device_id);

    let kind = if adapter.if_type == IF_TYPE_IEEE80211 {
        LinkKind::Wifi {
            ssid: wifi.map(|w| w.ssid.clone()),
            signal_percent: wifi.map(|w| w.signal_percent),
        }
    } else if adapter.if_type == IF_TYPE_ETHERNET_CSMACD && usb_ids.is_some() {
        LinkKind::UsbTether
    } else if adapter.if_type == IF_TYPE_ETHERNET_CSMACD {
        LinkKind::Ethernet
    } else {
        LinkKind::Other
    };

    let stable_key = if let Some((vid, pid, serial)) = &usb_ids {
        usb_key(vid, pid, serial)
    } else if let Some(mac) = adapter.physical_address {
        mac_key(mac)
    } else {
        format!("guid:{}", adapter.adapter_name)
    };

    Some(LinkCandidate {
        if_index: adapter.if_index,
        name: adapter.friendly_name.clone(),
        kind,
        stable_key,
        ips: adapter.unicast_ips.clone(),
        internet_access: InternetAccess::Unknown,
    })
}

/// Sceglie l'IP da usare come `local_ip` per il binding di un link
/// (§4.2 — `IP_UNICAST_IF` + bind locale). **Mai** il primo IP qualunque:
/// Windows elenca tipicamente l'IPv6 link-local (`fe80::/10`, assegnato
/// automaticamente a ogni interfaccia) prima dell'IPv4 vero, ma un
/// indirizzo link-local non è instradabile su internet — bindarci sopra
/// per una connessione verso un host pubblico fallisce (o peggio, sembra
/// funzionare a intermittenza a seconda di quale famiglia il DNS
/// restituisce quel momento, con errori bassi livello poco leggibili come
/// WSAEFAULT invece di un fallimento pulito — bug reale trovato con
/// hardware vero il 2026-08-21, non un problema ambientale come sembrava
/// la prima volta). Preferenza: IPv4 globale, poi IPv6 globale, mai
/// link-local.
pub fn preferred_local_ip(ips: &[IpAddr]) -> Option<IpAddr> {
    ips.iter()
        .find(|ip| matches!(ip, IpAddr::V4(v4) if !v4.is_link_local()))
        .or_else(|| ips.iter().find(|ip| matches!(ip, IpAddr::V6(v6) if !is_link_local_v6(v6))))
        .copied()
}

fn is_link_local_v6(v6: &std::net::Ipv6Addr) -> bool {
    (v6.segments()[0] & 0xffc0) == 0xfe80
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_adapter(if_type: u32) -> AdapterInfo {
        AdapterInfo {
            if_index: 7,
            adapter_name: "{4D36E972-E325-11CE-BFC1-08002BE10318}".to_string(),
            friendly_name: "Ethernet 3".to_string(),
            if_type,
            oper_status_up: true,
            physical_address: Some([0x00, 0x11, 0x22, 0x33, 0x44, 0x55]),
            unicast_ips: vec!["192.168.1.10".parse().unwrap()],
        }
    }

    #[test]
    fn excludes_dualbond_own_tunnel_adapter() {
        // Bug reale: l'adattatore Wintun "DualBond" (IfType virtuale, quindi non
        // escluso come tunnel) compariva tra le connessioni nelle Impostazioni.
        let mut by_name = base_adapter(53);
        by_name.friendly_name = TUNNEL_ADAPTER_NAME.to_string();
        assert_eq!(build_link_candidate(&by_name, None, None), None);

        let mut by_address = base_adapter(53);
        by_address.friendly_name = "DualBond 2".to_string();
        by_address.unicast_ips = vec![IpAddr::V4(TUNNEL_IPV4)];
        assert_eq!(build_link_candidate(&by_address, None, None), None);

        // un adattatore qualunque resta un candidato
        assert!(build_link_candidate(&base_adapter(IF_TYPE_ETHERNET_CSMACD), None, None).is_some());
    }

    #[test]
    fn excludes_loopback() {
        let a = base_adapter(24);
        assert_eq!(build_link_candidate(&a, None, None), None);
    }

    #[test]
    fn excludes_tunnel() {
        let a = base_adapter(131);
        assert_eq!(build_link_candidate(&a, None, None), None);
    }

    #[test]
    fn excludes_disconnected_adapter_even_with_a_stale_ip() {
        let mut a = base_adapter(IF_TYPE_IEEE80211);
        a.oper_status_up = false;
        assert_eq!(build_link_candidate(&a, None, None), None);
    }

    #[test]
    fn ethernet_csmacd_with_usb_pnp_id_is_usb_tether() {
        let a = base_adapter(IF_TYPE_ETHERNET_CSMACD);
        let c = build_link_candidate(&a, Some(r"USB\VID_18D1&PID_4EE7\0123456789ABCDEF"), None)
            .unwrap();
        assert_eq!(c.kind, LinkKind::UsbTether);
        assert_eq!(c.stable_key, "usb:18D1:4EE7:0123456789ABCDEF");
        assert_eq!(c.if_index, 7);
    }

    #[test]
    fn ethernet_csmacd_with_pci_pnp_id_is_real_ethernet() {
        let a = base_adapter(IF_TYPE_ETHERNET_CSMACD);
        let c = build_link_candidate(&a, Some(r"PCI\VEN_8086&DEV_15BE"), None).unwrap();
        assert_eq!(c.kind, LinkKind::Ethernet);
        assert_eq!(c.stable_key, mac_key([0x00, 0x11, 0x22, 0x33, 0x44, 0x55]));
    }

    #[test]
    fn ethernet_csmacd_without_pnp_id_is_real_ethernet() {
        let a = base_adapter(IF_TYPE_ETHERNET_CSMACD);
        let c = build_link_candidate(&a, None, None).unwrap();
        assert_eq!(c.kind, LinkKind::Ethernet);
    }

    #[test]
    fn wifi_with_active_connection_carries_ssid_and_signal() {
        let a = base_adapter(IF_TYPE_IEEE80211);
        let wifi = WifiInfo {
            ssid: "CasaMia5G".to_string(),
            signal_percent: 82,
            bssid: [1, 2, 3, 4, 5, 6],
        };
        let c = build_link_candidate(&a, None, Some(&wifi)).unwrap();
        assert_eq!(
            c.kind,
            LinkKind::Wifi {
                ssid: Some("CasaMia5G".to_string()),
                signal_percent: Some(82)
            }
        );
        assert_eq!(c.stable_key, mac_key([0x00, 0x11, 0x22, 0x33, 0x44, 0x55]));
    }

    #[test]
    fn wifi_without_active_connection_has_no_ssid() {
        let a = base_adapter(IF_TYPE_IEEE80211);
        let c = build_link_candidate(&a, None, None).unwrap();
        assert_eq!(
            c.kind,
            LinkKind::Wifi {
                ssid: None,
                signal_percent: None
            }
        );
    }

    #[test]
    fn falls_back_to_adapter_name_guid_when_no_mac_available() {
        let mut a = base_adapter(IF_TYPE_ETHERNET_CSMACD);
        a.physical_address = None;
        let c = build_link_candidate(&a, None, None).unwrap();
        assert_eq!(
            c.stable_key,
            "guid:{4D36E972-E325-11CE-BFC1-08002BE10318}"
        );
    }

    #[test]
    fn preferred_local_ip_skips_ipv6_link_local_listed_first() {
        // Bug reale con hardware vero (2026-08-21): Windows elenca
        // l'IPv6 link-local prima dell'IPv4 vero (esattamente l'ordine
        // osservato per il link Wi-Fi reale che ha causato il bug).
        let ips = vec![
            "fe80::448e:41d2:df0:630f".parse().unwrap(),
            "192.168.1.234".parse().unwrap(),
        ];
        assert_eq!(preferred_local_ip(&ips), Some("192.168.1.234".parse().unwrap()));
    }

    #[test]
    fn preferred_local_ip_uses_ipv4_even_if_listed_after_ipv6() {
        let ips = vec!["2001:db8::1".parse().unwrap(), "10.0.0.5".parse().unwrap()];
        assert_eq!(preferred_local_ip(&ips), Some("10.0.0.5".parse().unwrap()));
    }

    #[test]
    fn preferred_local_ip_falls_back_to_global_ipv6_when_no_ipv4() {
        let ips = vec!["fe80::1".parse().unwrap(), "2001:db8::1".parse().unwrap()];
        assert_eq!(preferred_local_ip(&ips), Some("2001:db8::1".parse().unwrap()));
    }

    #[test]
    fn preferred_local_ip_returns_none_when_only_link_local_addresses() {
        let ips = vec!["fe80::1".parse().unwrap(), "169.254.1.1".parse().unwrap()];
        assert_eq!(preferred_local_ip(&ips), None);
    }

    #[test]
    fn preferred_local_ip_returns_none_for_empty_list() {
        assert_eq!(preferred_local_ip(&[]), None);
    }
}
