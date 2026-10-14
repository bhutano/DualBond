use std::net::{IpAddr, SocketAddr};

use socket2::{Domain, Socket, Type};
use windows_sys::Win32::Networking::WinSock::{
    setsockopt, WSAGetLastError, IPPROTO_IP, IPPROTO_IPV6, IP_UNICAST_IF, IPV6_UNICAST_IF,
};

use crate::core::errors::{win32_err, DualBondError};

/// Calcola `(level, optname, valore)` per `IP_UNICAST_IF`/`IPV6_UNICAST_IF`
/// (§5 delle note tecniche). **Punto critico**: IPv4 vuole `ifindex` in
/// network byte order (big-endian), IPv6 in host byte order — la spec lo
/// segnala come l'errore più comune in assoluto su questa API.
fn unicast_if_sockopt(is_ipv6: bool, if_index: u32) -> (i32, i32, [u8; 4]) {
    if is_ipv6 {
        (IPPROTO_IPV6, IPV6_UNICAST_IF, if_index.to_ne_bytes())
    } else {
        (IPPROTO_IP, IP_UNICAST_IF, if_index.to_be_bytes())
    }
}

/// Crea un socket UDP bindato all'interfaccia di rete `if_index` tramite
/// `IP_UNICAST_IF`/`IPV6_UNICAST_IF` e bindato localmente su `local_ip`
/// (decisione 9.1 delle note tecniche: nessun host esterno, il probe di
/// verifica userà questo socket per inviare verso il gateway del link).
pub fn bind_udp_socket_to_interface(
    if_index: u32,
    local_ip: IpAddr,
) -> Result<Socket, DualBondError> {
    bind_socket_to_interface(if_index, local_ip, Type::DGRAM)
}

/// Come `bind_udp_socket_to_interface`, ma TCP: usato dal connector per-link
/// (§4.2, Milestone 4) per aprire le connessioni di download bindate a un
/// link specifico, bypassando la route lookup di default.
pub fn bind_tcp_socket_to_interface(
    if_index: u32,
    local_ip: IpAddr,
) -> Result<Socket, DualBondError> {
    bind_socket_to_interface(if_index, local_ip, Type::STREAM)
}

fn bind_socket_to_interface(
    if_index: u32,
    local_ip: IpAddr,
    sock_type: Type,
) -> Result<Socket, DualBondError> {
    let domain = match local_ip {
        IpAddr::V4(_) => Domain::IPV4,
        IpAddr::V6(_) => Domain::IPV6,
    };
    let socket = Socket::new(domain, sock_type, None)?;

    let (level, optname, optval) = unicast_if_sockopt(local_ip.is_ipv6(), if_index);
    let raw = socket_raw_handle(&socket);
    let ret = unsafe {
        setsockopt(raw, level, optname, optval.as_ptr(), optval.len() as i32)
    };
    if ret != 0 {
        let code = unsafe { WSAGetLastError() };
        return Err(win32_err("setsockopt(IP_UNICAST_IF)", code as u32));
    }

    socket.bind(&SocketAddr::new(local_ip, 0).into())?;
    Ok(socket)
}

#[cfg(windows)]
fn socket_raw_handle(socket: &Socket) -> usize {
    use std::os::windows::io::AsRawSocket;
    socket.as_raw_socket() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_uses_ipproto_ip_and_network_byte_order() {
        let (level, optname, val) = unicast_if_sockopt(false, 5);
        assert_eq!(level, IPPROTO_IP);
        assert_eq!(optname, IP_UNICAST_IF);
        assert_eq!(val, 5u32.to_be_bytes());
    }

    #[test]
    fn ipv6_uses_ipproto_ipv6_and_host_byte_order() {
        let (level, optname, val) = unicast_if_sockopt(true, 5);
        assert_eq!(level, IPPROTO_IPV6);
        assert_eq!(optname, IPV6_UNICAST_IF);
        assert_eq!(val, 5u32.to_ne_bytes());
    }

    #[test]
    fn ipv4_and_ipv6_byte_order_differ_for_non_symmetric_index() {
        // ifindex scelto apposta non palindromo in byte, cosi' i due layout
        // (BE vs NE) producono davvero byte diversi e il test non e' vacuo.
        let (_, _, v4) = unicast_if_sockopt(false, 0x0000_0102);
        let (_, _, v6) = unicast_if_sockopt(true, 0x0000_0102);
        assert_ne!(v4, v6);
    }
}
