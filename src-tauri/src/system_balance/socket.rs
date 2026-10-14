use crate::network::socket_bind::{bind_tcp_socket_to_interface, bind_udp_socket_to_interface};
use std::io;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::{TcpStream, UdpSocket};

pub async fn connect_tcp(
    if_index: u32,
    local_ip: std::net::IpAddr,
    remote: SocketAddr,
) -> io::Result<TcpStream> {
    if local_ip.is_ipv6() != remote.is_ipv6() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "famiglia IP locale e remota non compatibile",
        ));
    }
    let socket = tokio::task::spawn_blocking(move || {
        let socket = bind_tcp_socket_to_interface(if_index, local_ip)
            .map_err(|error| io::Error::other(error.to_string()))?;
        socket.connect_timeout(&remote.into(), Duration::from_secs(10))?;
        Ok::<_, io::Error>(socket)
    })
    .await
    .map_err(|error| io::Error::other(format!("task connect interrotto: {error}")))??;
    let stream: std::net::TcpStream = socket.into();
    stream.set_nonblocking(true)?;
    TcpStream::from_std(stream)
}

pub fn connect_udp(
    if_index: u32,
    local_ip: std::net::IpAddr,
    remote: SocketAddr,
) -> io::Result<UdpSocket> {
    if local_ip.is_ipv6() != remote.is_ipv6() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "famiglia IP locale e remota non compatibile",
        ));
    }
    let socket = bind_udp_socket_to_interface(if_index, local_ip)
        .map_err(|error| io::Error::other(error.to_string()))?;
    socket.connect(&remote.into())?;
    socket.set_nonblocking(true)?;
    let socket: std::net::UdpSocket = socket.into();
    UdpSocket::from_std(socket)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn plain_loopback_tcp_shape_is_compatible_with_bidirectional_copy() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4];
            stream.read_exact(&mut request).await.unwrap();
            stream.write_all(&request).await.unwrap();
        });
        let mut client = TcpStream::connect(address).await.unwrap();
        client.write_all(b"ping").await.unwrap();
        let mut response = [0u8; 4];
        client.read_exact(&mut response).await.unwrap();
        assert_eq!(&response, b"ping");
        server.await.unwrap();
    }

    #[test]
    fn udp_rejects_mixed_address_families_before_touching_hardware() {
        let error =
            connect_udp(1, "127.0.0.1".parse().unwrap(), "[::1]:53".parse().unwrap()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
}
