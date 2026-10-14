/// Chiave stabile per un adattatore USB, derivata da `PNPDeviceID`
/// (spec §3.2): `"usb:{VID}:{PID}:{serial}"`.
pub fn usb_key(vid: &str, pid: &str, serial: &str) -> String {
    format!("usb:{vid}:{pid}:{serial}")
}

/// Chiave stabile per un adattatore con MAC fisico stabile (Ethernet reale,
/// Wi-Fi): `"mac:{XX:XX:XX:XX:XX:XX}"` (esadecimale minuscolo).
pub fn mac_key(mac: [u8; 6]) -> String {
    format!(
        "mac:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}

/// Estrae `(vid, pid, serial)` da un `PNPDeviceID` Windows nella forma
/// `"USB\VID_xxxx&PID_xxxx\<serial>"`. Ritorna `None` se non è un device
/// USB (prefisso diverso da `"USB\"`) o se il formato non combacia.
pub fn parse_usb_pnp_device_id(pnp: &str) -> Option<(String, String, String)> {
    let rest = pnp.strip_prefix(r"USB\")?;
    let (ids, serial) = rest.split_once('\\')?;
    let (vid_part, pid_part) = ids.split_once('&')?;
    let vid = vid_part.strip_prefix("VID_")?;
    let pid = pid_part.strip_prefix("PID_")?;
    if vid.is_empty() || pid.is_empty() || serial.is_empty() {
        return None;
    }
    Some((vid.to_string(), pid.to_string(), serial.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_key_formats_vid_pid_serial() {
        assert_eq!(
            usb_key("18D1", "4EE7", "0123456789ABCDEF"),
            "usb:18D1:4EE7:0123456789ABCDEF"
        );
    }

    #[test]
    fn mac_key_formats_lowercase_colon_separated() {
        assert_eq!(
            mac_key([0xAA, 0xBB, 0x0C, 0xDD, 0xEE, 0xFF]),
            "mac:aa:bb:0c:dd:ee:ff"
        );
    }

    #[test]
    fn parse_usb_pnp_device_id_extracts_vid_pid_serial() {
        let parsed = parse_usb_pnp_device_id(r"USB\VID_18D1&PID_4EE7\0123456789ABCDEF");
        assert_eq!(
            parsed,
            Some((
                "18D1".to_string(),
                "4EE7".to_string(),
                "0123456789ABCDEF".to_string()
            ))
        );
    }

    #[test]
    fn parse_usb_pnp_device_id_rejects_non_usb_prefix() {
        let parsed = parse_usb_pnp_device_id(r"PCI\VEN_8086&DEV_15BE\3&11583659&0&C8");
        assert_eq!(parsed, None);
    }

    #[test]
    fn parse_usb_pnp_device_id_rejects_malformed_usb_id() {
        let parsed = parse_usb_pnp_device_id(r"USB\garbage");
        assert_eq!(parsed, None);
    }
}
