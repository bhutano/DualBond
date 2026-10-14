//! Apre connessioni TCP per un worker di download, bindate a un link
//! specifico (§4.2 della spec: `IP_UNICAST_IF` + bind locale, bypassando la
//! route lookup di default) oppure con la route di sistema di default.
//!
//! **Limite noto**: la risoluzione DNS qui usa il resolver di sistema
//! (`tokio::net::lookup_host`), condiviso tra tutti i link — la spec (§4.2)
//! chiede una cache DNS separata per link (`resolver.rs`, non ancora
//! implementato). Per ora tutti i link risolvono lo stesso host verso lo
//! stesso IP remoto, il che è corretto nella grande maggioranza dei casi
//! (un solo servidor dietro l'URL) ma non nei casi anycast/split-horizon.
//!
//! `LinkConnector::Bound` è validabile solo con hardware multi-interfaccia
//! reale (stesso vincolo di Milestone 1, verificato manualmente con
//! `cargo run -- doctor` — non c'è modo di provarlo in automatico in un
//! ambiente di sviluppo con una sola interfaccia utilizzabile).

use crate::core::errors::DualBondError;
use crate::network::socket_bind::bind_tcp_socket_to_interface;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use tokio::net::TcpStream;

#[derive(Debug, Clone, Copy)]
pub enum LinkConnector {
    /// Connessione bindata a un link specifico via `IP_UNICAST_IF`.
    Bound { if_index: u32, local_ip: IpAddr },
    /// Route di default del sistema. Usata nei test di orchestrazione dove
    /// un solo host locale impersona più "link" logici, e come fallback se
    /// un giorno servirà un modo esplicito di non fissare l'uscita.
    Plain,
}

impl LinkConnector {
    pub async fn connect(&self, host: &str, port: u16) -> Result<TcpStream, DualBondError> {
        match *self {
            LinkConnector::Plain => connect_plain(host, port).await,
            LinkConnector::Bound { if_index, local_ip } => {
                connect_bound(if_index, local_ip, host, port).await
            }
        }
    }
}

async fn connect_plain(host: &str, port: u16) -> Result<TcpStream, DualBondError> {
    tokio::time::timeout(Duration::from_secs(10), TcpStream::connect((host, port)))
        .await
        .map_err(|_| DualBondError::Transport(format!("timeout di connessione a {host}:{port}")))?
        .map_err(|e| DualBondError::Transport(format!("connessione fallita a {host}:{port}: {e}")))
}

/// Sceglie, tra gli indirizzi risolti dal DNS, il primo della stessa
/// famiglia (IPv4/IPv6) di `local_ip` — mai un indirizzo qualunque. Un
/// socket bindato a un `local_ip` IPv4 non può connettersi a un
/// `SocketAddr` IPv6 (o viceversa): il mismatch produce un errore basso
/// livello poco leggibile (WSAEFAULT su Windows) invece di fallire in
/// modo pulito o, peggio, di non fallire mai perché per caso il DNS
/// aveva restituito la famiglia giusta quella volta.
fn pick_matching_family(addrs: &[SocketAddr], local_ip: IpAddr) -> Option<SocketAddr> {
    addrs.iter().copied().find(|a| a.is_ipv6() == local_ip.is_ipv6())
}

async fn connect_bound(
    if_index: u32,
    local_ip: IpAddr,
    host: &str,
    port: u16,
) -> Result<TcpStream, DualBondError> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| DualBondError::Transport(format!("risoluzione DNS fallita per {host}: {e}")))?
        .collect();
    // Bug reale trovato con hardware vero (2026-08-21): host con record A
    // e AAAA entrambi (es. ash-speed.hetzner.com). Il socket qui sotto è
    // creato per la famiglia di `local_ip`; connettersi a un indirizzo di
    // famiglia diversa (il resolver di sistema può restituire l'IPv6 per
    // primo) produce un mismatch — osservato come WSAEFAULT su Windows
    // invece di un errore pulito di famiglia non supportata. Va scelto un
    // indirizzo della stessa famiglia del link, mai il primo qualunque.
    let family = if local_ip.is_ipv6() { "IPv6" } else { "IPv4" };
    let remote = pick_matching_family(&addrs, local_ip).ok_or_else(|| {
        DualBondError::Transport(format!("nessun indirizzo {family} per {host} (il link usa {family})"))
    })?;

    // `connect_timeout` di socket2 è bloccante: gira su un thread del pool
    // blocking di tokio, mai sul reattore async.
    let socket = tokio::task::spawn_blocking(move || {
        let socket = bind_tcp_socket_to_interface(if_index, local_ip)?;
        socket.connect_timeout(&remote.into(), Duration::from_secs(10)).map_err(|e| {
            DualBondError::Transport(format!(
                "connessione bindata al link fallita verso {remote}: {e}"
            ))
        })?;
        Ok::<_, DualBondError>(socket)
    })
    .await
    .map_err(|e| DualBondError::Transport(format!("task di connessione interrotto: {e}")))??;

    let std_stream: std::net::TcpStream = socket.into();
    std_stream.set_nonblocking(true).map_err(DualBondError::from)?;
    TcpStream::from_std(std_stream).map_err(DualBondError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn picks_ipv4_remote_when_local_ip_is_ipv4_even_if_ipv6_listed_first() {
        // Bug reale trovato con hardware vero (2026-08-21): host con
        // record A e AAAA entrambi (es. ash-speed.hetzner.com). Se il
        // resolver ritorna prima l'IPv6, connettersi con quello a un
        // socket creato per la famiglia di `local_ip` (IPv4) causa un
        // mismatch di famiglia — WSAEFAULT osservato su Windows invece di
        // un errore pulito di famiglia non supportata.
        let addrs = [
            "[2a01:4ff:ef::fa57:1]:443".parse().unwrap(),
            "5.161.7.195:443".parse().unwrap(),
        ];
        let local_ip: IpAddr = "192.168.1.234".parse().unwrap();

        let picked = pick_matching_family(&addrs, local_ip);

        assert_eq!(picked, Some(addrs[1]), "deve scartare l'IPv6 e prendere l'IPv4, anche se elencato per primo");
    }

    #[test]
    fn picks_ipv6_remote_when_local_ip_is_ipv6() {
        let addrs = [
            "5.161.7.195:443".parse().unwrap(),
            "[2a01:4ff:ef::fa57:1]:443".parse().unwrap(),
        ];
        let local_ip: IpAddr = "fe80::1".parse().unwrap();

        let picked = pick_matching_family(&addrs, local_ip);

        assert_eq!(picked, Some(addrs[1]));
    }

    #[test]
    fn returns_none_when_no_address_matches_the_local_family() {
        let addrs = ["[2a01:4ff:ef::fa57:1]:443".parse().unwrap()];
        let local_ip: IpAddr = "192.168.1.234".parse().unwrap();

        assert_eq!(pick_matching_family(&addrs, local_ip), None);
    }

    #[test]
    fn keeps_dns_order_among_addresses_of_the_matching_family() {
        let addrs = [
            "5.161.7.195:443".parse().unwrap(),
            "203.0.113.9:443".parse().unwrap(),
        ];
        let local_ip: IpAddr = "192.168.1.234".parse().unwrap();

        assert_eq!(pick_matching_family(&addrs, local_ip), Some(addrs[0]));
    }

    #[tokio::test]
    async fn plain_connector_reaches_a_local_listener() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let accept = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 5];
            stream.read_exact(&mut buf).await.unwrap();
            assert_eq!(&buf, b"hello");
        });

        let mut stream = LinkConnector::Plain.connect("127.0.0.1", port).await.unwrap();
        stream.write_all(b"hello").await.unwrap();
        accept.await.unwrap();
    }

    /// Verifica manuale (richiede hardware reale + `dualbond doctor` già
    /// eseguito per avere if_index/local_ip aggiornati): diagnostica diretta
    /// del path `Bound`, mai esercitato prima su hardware vero prima di
    /// stanotte. Aggiornare if_index/local_ip con l'output di `doctor`.
    #[tokio::test]
    #[ignore]
    async fn debug_bound_connector_against_real_link() {
        use crate::download::http_client::{empty_body, empty_request, send_via_connector_with_retry};
        use hyper::{Method, Uri};

        let connector = LinkConnector::Bound {
            if_index: 10,
            local_ip: "192.168.1.234".parse().unwrap(),
        };
        let uri: Uri = "https://ash-speed.hetzner.com/100MB.bin".parse().unwrap();
        let build_req = || {
            empty_request(Method::GET, &uri)?
                .header(hyper::header::RANGE, "bytes=0-999")
                .body(empty_body())
                .map_err(|e| DualBondError::Http(e.to_string()))
        };
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            send_via_connector_with_retry(&connector, &uri, build_req),
        )
        .await;
        eprintln!("RISULTATO: {result:?}");
        let resp = result.expect("timeout dopo 20s: hang confermato oltre il connect").unwrap();
        eprintln!("STATUS: {}", resp.status());
    }

    #[tokio::test]
    #[ignore]
    async fn debug_repeated_sequential_bound_connects() {
        for i in 0..20u32 {
            let connector = LinkConnector::Bound {
                if_index: 10,
                local_ip: "192.168.1.234".parse().unwrap(),
            };
            let r = connector.connect("ash-speed.hetzner.com", 443).await;
            eprintln!("tentativo {i}: {:?}", r.as_ref().map(|_| "OK").map_err(|e| e.to_string()));
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore]
    async fn debug_four_concurrent_bound_requests_same_link() {
        use crate::download::http_client::{empty_body, empty_request, send_via_connector_with_retry};
        use hyper::{Method, Uri};

        let mut tasks = Vec::new();
        for i in 0..4u32 {
            tasks.push(tokio::spawn(async move {
                let connector = LinkConnector::Bound {
                    if_index: 10,
                    local_ip: "192.168.1.234".parse().unwrap(),
                };
                let uri: Uri = "https://ash-speed.hetzner.com/100MB.bin".parse().unwrap();
                let build_req = || {
                    empty_request(Method::GET, &uri)?
                        .header(hyper::header::RANGE, format!("bytes={}-{}", i * 1000, i * 1000 + 999))
                        .body(empty_body())
                        .map_err(|e| DualBondError::Http(e.to_string()))
                };
                let r = tokio::time::timeout(
                    std::time::Duration::from_secs(15),
                    send_via_connector_with_retry(&connector, &uri, build_req),
                )
                .await;
                eprintln!("worker {i}: {r:?}");
                r
            }));
        }
        for (i, t) in tasks.into_iter().enumerate() {
            let outcome = t.await.unwrap();
            let _ = outcome.unwrap_or_else(|_| panic!("worker {i} in timeout: hang confermato solo in concorrenza"));
        }
    }

    #[tokio::test]
    async fn plain_connector_reports_transport_error_on_refused_connection() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let err = LinkConnector::Plain.connect("127.0.0.1", port).await.unwrap_err();
        assert!(matches!(err, DualBondError::Transport(_)));
    }
}
