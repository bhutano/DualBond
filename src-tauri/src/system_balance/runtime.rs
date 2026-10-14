use super::model::{unix_ms, ServiceConfig, SystemBalanceStatus, SystemLinkStatus};
use super::route::{RouteGuard, WindowsRouteExecutor};
use super::scheduler::{preference, FlowScheduler, LinkSelection};
use super::socket::{connect_tcp, connect_udp};
use super::store::{save_config, save_status, ServicePaths};
use crate::doctor::discover_candidates;
use crate::network::connectivity::InternetAccess;
use ipstack::{IpStack, IpStackConfig, IpStackStream};
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::watch;
use tun::AbstractDevice;

pub struct ServiceRuntime {
    pub paths: ServicePaths,
    pub config: RwLock<ServiceConfig>,
    pub scheduler: Arc<Mutex<FlowScheduler>>,
    pub status: RwLock<SystemBalanceStatus>,
    desired_tx: watch::Sender<bool>,
    links_tx: watch::Sender<u64>,
}

impl ServiceRuntime {
    pub fn new(paths: ServicePaths, config: ServiceConfig) -> Arc<Self> {
        let (desired_tx, _) = watch::channel(config.enabled);
        let (links_tx, _) = watch::channel(0);
        Arc::new(Self {
            paths,
            config: RwLock::new(config),
            scheduler: Arc::new(Mutex::new(FlowScheduler::default())),
            status: RwLock::new(SystemBalanceStatus {
                service_available: true,
                enabled: false,
                state: "idle".into(),
                adapter_name: "DualBond".into(),
                ipv4_only: true,
                total_mbps: 0.0,
                total_avg_mbps: 0.0,
                tcp_flows: 0,
                udp_flows: 0,
                links: Vec::new(),
                last_error: None,
                updated_at: unix_ms(),
            }),
            desired_tx,
            links_tx,
        })
    }

    pub fn desired(&self) -> watch::Receiver<bool> {
        self.desired_tx.subscribe()
    }

    fn link_changes(&self) -> watch::Receiver<u64> {
        self.links_tx.subscribe()
    }

    pub fn snapshot(&self) -> SystemBalanceStatus {
        self.status.read().unwrap().clone()
    }

    pub fn set_enabled(&self, enabled: bool) -> io::Result<SystemBalanceStatus> {
        {
            let mut config = self.config.write().unwrap();
            config.enabled = enabled;
            save_config(&self.paths, &config)?;
        }
        self.desired_tx.send_replace(enabled);
        if !enabled {
            self.scheduler.lock().unwrap().deactivate();
        }
        let mut status = self.status.write().unwrap();
        status.enabled = enabled;
        status.state = if enabled { "starting" } else { "idle" }.into();
        status.last_error = None;
        if !enabled {
            zero_live_status(&mut status);
        }
        status.updated_at = unix_ms();
        save_status(&self.paths, &status)?;
        Ok(status.clone())
    }

    pub fn set_link_enabled(&self, key: &str, enabled: bool) -> io::Result<SystemBalanceStatus> {
        let config = {
            let mut config = self.config.write().unwrap();
            preference(&mut config, key).enabled = enabled;
            save_config(&self.paths, &config)?;
            config.clone()
        };
        self.refresh_links_with(&config);
        self.links_tx.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
        if !enabled {
            let mut status = self.status.write().unwrap();
            if let Some(link) = status.links.iter_mut().find(|link| link.key == key) {
                link.enabled = false;
                link.state = "disabled".into();
                link.mbps = 0.0;
                link.avg_mbps = 0.0;
                link.tcp_flows = 0;
                link.udp_flows = 0;
            }
            status.total_mbps = status.links.iter().map(|link| link.mbps).sum();
            status.total_avg_mbps = status.links.iter().map(|link| link.avg_mbps).sum();
            status.tcp_flows = status.links.iter().map(|link| link.tcp_flows).sum();
            status.udp_flows = status.links.iter().map(|link| link.udp_flows).sum();
            status.updated_at = unix_ms();
            let _ = save_status(&self.paths, &status);
        }
        Ok(self.snapshot())
    }

    pub fn set_link_metered(&self, key: &str, metered: bool) -> io::Result<SystemBalanceStatus> {
        let config = {
            let mut config = self.config.write().unwrap();
            preference(&mut config, key).metered = metered;
            save_config(&self.paths, &config)?;
            config.clone()
        };
        self.refresh_links_with(&config);
        Ok(self.snapshot())
    }

    pub fn refresh_links(&self) -> SystemBalanceStatus {
        let config = self.config.read().unwrap().clone();
        self.refresh_links_with(&config);
        self.snapshot()
    }

    /// Esiste almeno un link da cui un flusso potrebbe uscire? Senza, il
    /// tunnel non deve catturare il traffico (fail-open): Windows continua a
    /// navigare normalmente sul percorso di sistema.
    fn has_usable_link(&self) -> bool {
        let config = self.config.read().unwrap().clone();
        self.scheduler.lock().unwrap().has_eligible_link(&config)
    }

    fn refresh_links_with(&self, config: &ServiceConfig) {
        self.scheduler
            .lock()
            .unwrap()
            .refresh(discover_candidates(), config);
    }

    fn set_state(&self, state_name: &str, enabled: bool, error: Option<String>) {
        let mut status = self.status.write().unwrap();
        status.service_available = true;
        status.enabled = enabled;
        status.state = state_name.into();
        status.last_error = error;
        status.updated_at = unix_ms();
        let _ = save_status(&self.paths, &status);
    }

    fn publish_status_tick(
        &self,
        previous_bytes: &mut HashMap<String, u64>,
        current_rates: &mut HashMap<String, f64>,
        long_rates: &mut HashMap<String, f64>,
    ) {
        let system_enabled = self.status.read().unwrap().enabled;
        if !system_enabled {
            current_rates.clear();
            long_rates.clear();
        }
        let scheduler = self.scheduler.lock().unwrap();
        let mut links: Vec<SystemLinkStatus> = scheduler
            .links()
            .map(|link| {
                let total_bytes = link.bytes_in.saturating_add(link.bytes_out);
                let previous = previous_bytes
                    .insert(link.selection.key.clone(), total_bytes)
                    .unwrap_or(total_bytes);
                let delta = total_bytes.saturating_sub(previous);
                let visible = system_enabled
                    && link.enabled
                    && link.online
                    && link.internet_access == InternetAccess::Internet
                    && !link.is_penalized();
                let raw_mbps = if visible {
                    delta as f64 * 8.0 / 1_000_000.0
                } else {
                    0.0
                };
                // Due EWMA: quella principale reagisce abbastanza in fretta
                // ma elimina i burst di un singolo tick; la seconda è una
                // tendenza più stabile per il valore “Media”.
                let current = current_rates
                    .entry(link.selection.key.clone())
                    .or_insert(0.0);
                *current = if visible {
                    0.28 * raw_mbps + 0.72 * *current
                } else {
                    0.0
                };
                let long = long_rates.entry(link.selection.key.clone()).or_insert(0.0);
                *long = if visible {
                    0.10 * raw_mbps + 0.90 * *long
                } else {
                    0.0
                };
                SystemLinkStatus {
                    key: link.selection.key.clone(),
                    name: link.selection.name.clone(),
                    kind: link.selection.kind.clone(),
                    if_index: link.selection.if_index,
                    local_ip: link.selection.local_ip,
                    enabled: link.enabled,
                    metered: link.selection.metered,
                    state: if !link.online {
                        "offline"
                    } else if !link.enabled {
                        "disabled"
                    } else if link.internet_access == InternetAccess::LocalOnly {
                        "local_only"
                    } else if link.internet_access == InternetAccess::Unknown {
                        "probing"
                    } else if link.is_penalized() {
                        "degraded"
                    } else {
                        "online"
                    }
                    .into(),
                    mbps: *current,
                    avg_mbps: *long,
                    bytes_in: link.bytes_in,
                    bytes_out: link.bytes_out,
                    tcp_flows: if visible { link.active_tcp } else { 0 },
                    udp_flows: if visible { link.active_udp } else { 0 },
                    errors: link.errors,
                }
            })
            .collect();
        links.sort_by(|left, right| left.name.cmp(&right.name));
        drop(scheduler);

        let total_mbps = links.iter().map(|link| link.mbps).sum();
        let total_avg_mbps = links.iter().map(|link| link.avg_mbps).sum();
        let tcp_flows = links.iter().map(|link| link.tcp_flows).sum();
        let udp_flows = links.iter().map(|link| link.udp_flows).sum();
        let mut status = self.status.write().unwrap();
        status.links = links;
        status.total_mbps = total_mbps;
        status.total_avg_mbps = total_avg_mps(total_avg_mbps);
        status.tcp_flows = tcp_flows;
        status.udp_flows = udp_flows;
        status.updated_at = unix_ms();
        let _ = save_status(&self.paths, &status);
    }
}

fn zero_live_status(status: &mut SystemBalanceStatus) {
    status.total_mbps = 0.0;
    status.total_avg_mbps = 0.0;
    status.tcp_flows = 0;
    status.udp_flows = 0;
    for link in &mut status.links {
        link.mbps = 0.0;
        link.avg_mbps = 0.0;
        link.tcp_flows = 0;
        link.udp_flows = 0;
    }
}

fn total_avg_mps(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

pub async fn run(
    runtime: Arc<ServiceRuntime>,
    mut service_stop: watch::Receiver<bool>,
) -> io::Result<()> {
    runtime.refresh_links();

    let discovery_runtime = runtime.clone();
    let mut discovery_stop = service_stop.clone();
    let discovery = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(2)) => {
                    discovery_runtime.refresh_links();
                }
                changed = discovery_stop.changed() => {
                    if changed.is_err() || *discovery_stop.borrow() { break; }
                }
            }
        }
    });

    let metrics_runtime = runtime.clone();
    let mut metrics_stop = service_stop.clone();
    let metrics = tokio::spawn(async move {
        let mut previous_bytes = HashMap::new();
        let mut current_rates = HashMap::new();
        let mut long_rates = HashMap::new();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(1)) => {
                    metrics_runtime.publish_status_tick(
                        &mut previous_bytes,
                        &mut current_rates,
                        &mut long_rates,
                    );
                }
                changed = metrics_stop.changed() => {
                    if changed.is_err() || *metrics_stop.borrow() { break; }
                }
            }
        }
    });

    let mut desired = runtime.desired();
    loop {
        if *service_stop.borrow() {
            break;
        }
        if !*desired.borrow() {
            runtime.set_state("idle", false, None);
            tokio::select! {
                changed = desired.changed() => { if changed.is_err() { break; } }
                changed = service_stop.changed() => { if changed.is_err() || *service_stop.borrow() { break; } }
            }
            continue;
        }

        if !runtime.has_usable_link() {
            // Nessuna uscita possibile: niente route di cattura, il PC resta
            // online sul percorso normale di Windows. Si riprova appena un
            // link torna utilizzabile (discovery ogni 2 s).
            runtime.set_state("waiting", true, None);
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                changed = desired.changed() => { if changed.is_err() { break; } }
                changed = service_stop.changed() => { if changed.is_err() || *service_stop.borrow() { break; } }
            }
            continue;
        }

        runtime.set_state("starting", true, None);
        match run_tunnel(runtime.clone(), desired.clone(), service_stop.clone()).await {
            Ok(()) => {
                if !*desired.borrow() {
                    runtime.set_state("idle", false, None);
                }
            }
            Err(error) => {
                runtime.set_state("error", true, Some(redact_error(&error)));
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                    changed = desired.changed() => { if changed.is_err() { break; } }
                    changed = service_stop.changed() => { if changed.is_err() || *service_stop.borrow() { break; } }
                }
            }
        }
    }

    discovery.abort();
    metrics.abort();
    runtime.set_state("stopped", false, None);
    Ok(())
}

async fn run_tunnel(
    runtime: Arc<ServiceRuntime>,
    mut desired: watch::Receiver<bool>,
    mut service_stop: watch::Receiver<bool>,
) -> io::Result<()> {
    let wintun_path = wintun_path()?;
    let mut configuration = tun::Configuration::default();
    configuration
        .tun_name(crate::network::discovery::TUNNEL_ADAPTER_NAME)
        .address(crate::network::discovery::TUNNEL_IPV4)
        .netmask((255, 255, 255, 252))
        .mtu(1500)
        .up()
        .platform_config(|platform| {
            platform.wintun_file(wintun_path);
            platform.wait_for_interfaces(true, false, Duration::from_secs(10));
        });

    let device = tun::create_as_async(&configuration)
        .map_err(|error| io::Error::other(format!("Wintun: {error}")))?;
    let if_index = device
        .tun_index()
        .map_err(|error| io::Error::other(format!("indice adattatore: {error}")))?
        as u32;
    let _routes = RouteGuard::install(WindowsRouteExecutor, if_index)?;

    let mut stack_config = IpStackConfig::default();
    stack_config
        .mtu(1500)
        .map_err(|error| io::Error::other(error.to_string()))?;
    stack_config.udp_timeout(Duration::from_secs(60));
    let mut stack = IpStack::new(stack_config, device);
    let mut flows = tokio::task::JoinSet::new();
    let mut health = tokio::time::interval(Duration::from_secs(2));
    runtime.set_state("active", true, None);

    loop {
        tokio::select! {
            accepted = stack.accept() => {
                match accepted {
                    Ok(IpStackStream::Tcp(stream)) => {
                        let runtime = runtime.clone();
                        flows.spawn(async move { handle_tcp(runtime, stream).await; });
                    }
                    Ok(IpStackStream::Udp(stream)) => {
                        let runtime = runtime.clone();
                        flows.spawn(async move { handle_udp(runtime, stream).await; });
                    }
                    Ok(_) => {}
                    Err(error) => return Err(io::Error::other(format!("stack IP: {error}"))),
                }
            }
            changed = desired.changed() => {
                if changed.is_err() || !*desired.borrow() { break; }
            }
            changed = service_stop.changed() => {
                if changed.is_err() || *service_stop.borrow() { break; }
            }
            Some(_) = flows.join_next(), if !flows.is_empty() => {}
            _ = health.tick() => {
                if !runtime.has_usable_link() {
                    // Tutti i link sono spariti o disattivati: si chiude il
                    // tunnel (le route /1 vengono rimosse da RouteGuard) invece
                    // di lasciare Windows senza alcuna uscita.
                    tracing::warn!("nessun link utilizzabile: tunnel sospeso fino al ritorno di un link");
                    break;
                }
            }
        }
    }
    flows.abort_all();
    while flows.join_next().await.is_some() {}
    runtime.scheduler.lock().unwrap().deactivate();
    Ok(())
}

async fn handle_tcp(runtime: Arc<ServiceRuntime>, mut inbound: ipstack::IpStackTcpStream) {
    let remote = inbound.peer_addr();
    if !supports_remote(remote) {
        tracing::debug!(%remote, "flusso TCP ignorato in modalita IPv4-only");
        return;
    }
    let config = runtime.config.read().unwrap().clone();
    let Some(link) = runtime.scheduler.lock().unwrap().reserve(&config, false) else {
        tracing::warn!(%remote, "flusso TCP rifiutato: nessun link disponibile");
        return;
    };
    tracing::info!(
        %remote,
        link = %link.key,
        interface = %link.name,
        if_index = link.if_index,
        local_ip = %link.local_ip,
        "flusso TCP assegnato"
    );
    let mut outbound = match connect_tcp(link.if_index, link.local_ip, remote).await {
        Ok(outbound) => {
            tracing::info!(%remote, link = %link.key, "connessione TCP stabilita");
            outbound
        }
        Err(error) => {
            finish_flow(&runtime, &link, false);
            runtime.scheduler.lock().unwrap().record_failure(&link.key);
            tracing::warn!(%remote, link = %link.key, %error, "connessione TCP fallita");
            return;
        }
    };
    let relay = async {
        let (mut from_pc, mut to_pc) = tokio::io::split(&mut inbound);
        let (mut from_network, mut to_network) = tokio::io::split(&mut outbound);
        let outgoing = copy_and_count(
            &mut from_pc,
            &mut to_network,
            runtime.scheduler.clone(),
            link.key.clone(),
            false,
        );
        let incoming = copy_and_count(
            &mut from_network,
            &mut to_pc,
            runtime.scheduler.clone(),
            link.key.clone(),
            true,
        );
        tokio::try_join!(outgoing, incoming)?;
        Ok::<(), io::Error>(())
    };
    let result = tokio::select! {
        result = relay => result,
        _ = wait_until_link_disabled(runtime.clone(), link.key.clone()) => Ok(()),
    };
    if let Err(error) = result {
        tracing::warn!(%remote, link = %link.key, %error, "relay TCP terminato con errore");
    }
    finish_flow(&runtime, &link, false);
}

async fn handle_udp(runtime: Arc<ServiceRuntime>, mut inbound: ipstack::IpStackUdpStream) {
    let remote = inbound.peer_addr();
    if !supports_remote(remote) {
        tracing::debug!(%remote, "flusso UDP ignorato in modalita IPv4-only");
        return;
    }
    let config = runtime.config.read().unwrap().clone();
    let Some(link) = runtime.scheduler.lock().unwrap().reserve(&config, true) else {
        tracing::warn!(%remote, "flusso UDP rifiutato: nessun link disponibile");
        return;
    };
    let socket = match connect_udp(link.if_index, link.local_ip, remote) {
        Ok(socket) => socket,
        Err(error) => {
            finish_flow(&runtime, &link, true);
            runtime.scheduler.lock().unwrap().record_failure(&link.key);
            tracing::warn!(%remote, link = %link.key, %error, "socket UDP fallito");
            return;
        }
    };
    let result = tokio::select! {
        result = relay_udp(&mut inbound, &link, socket, runtime.scheduler.clone()) => result,
        _ = wait_until_link_disabled(runtime.clone(), link.key.clone()) => Ok(()),
    };
    if let Err(error) = result {
        tracing::warn!(%remote, link = %link.key, %error, "relay UDP terminato con errore");
    }
    finish_flow(&runtime, &link, true);
}

async fn wait_until_link_disabled(runtime: Arc<ServiceRuntime>, key: String) {
    let mut changes = runtime.link_changes();
    loop {
        let enabled = runtime
            .config
            .read()
            .unwrap()
            .links
            .get(&key)
            .map(|preference| preference.enabled)
            .unwrap_or(true);
        if !enabled {
            return;
        }
        if changes.changed().await.is_err() {
            return;
        }
    }
}

async fn relay_udp(
    inbound: &mut ipstack::IpStackUdpStream,
    link: &LinkSelection,
    socket: tokio::net::UdpSocket,
    scheduler: Arc<Mutex<FlowScheduler>>,
) -> io::Result<()> {
    let mut from_tun = vec![0u8; 65_535];
    let mut from_network = vec![0u8; 65_535];
    loop {
        tokio::select! {
            read = inbound.read(&mut from_tun) => {
                let count = read?;
                if count == 0 { break; }
                socket.send(&from_tun[..count]).await?;
                scheduler.lock().unwrap().record_bytes(&link.key, count as u64, false);
            }
            read = socket.recv(&mut from_network) => {
                let count = read?;
                inbound.write_all(&from_network[..count]).await?;
                scheduler.lock().unwrap().record_bytes(&link.key, count as u64, true);
            }
        }
    }
    Ok(())
}

fn finish_flow(runtime: &ServiceRuntime, link: &LinkSelection, udp: bool) {
    let mut scheduler = runtime.scheduler.lock().unwrap();
    scheduler.flow_finished(&link.key, udp);
}

async fn copy_and_count<R, W>(
    reader: &mut R,
    writer: &mut W,
    scheduler: Arc<Mutex<FlowScheduler>>,
    key: String,
    inbound: bool,
) -> io::Result<u64>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut buffer = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            writer.shutdown().await?;
            return Ok(total);
        }
        writer.write_all(&buffer[..count]).await?;
        scheduler
            .lock()
            .unwrap()
            .record_bytes(&key, count as u64, inbound);
        total = total.saturating_add(count as u64);
    }
}

fn wintun_path() -> io::Result<PathBuf> {
    let executable = std::env::current_exe()?;
    let path = executable
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("wintun.dll");
    if path.is_file() {
        Ok(path)
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("wintun.dll non trovato accanto a {}", executable.display()),
        ))
    }
}

fn redact_error(error: &io::Error) -> String {
    let text = error.to_string();
    if text.len() > 300 {
        format!("{}…", &text[..300])
    } else {
        text
    }
}

fn supports_remote(remote: std::net::SocketAddr) -> bool {
    remote.is_ipv4()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_average_is_never_exposed() {
        assert_eq!(total_avg_mps(f64::NAN), 0.0);
        assert_eq!(total_avg_mps(-5.0), 0.0);
        assert_eq!(total_avg_mps(3.5), 3.5);
    }

    #[test]
    fn error_redaction_bounds_state_file_payload() {
        let error = io::Error::other("x".repeat(1_000));
        assert!(redact_error(&error).len() <= 303);
    }

    #[test]
    fn disabled_status_is_zeroed_immediately() {
        let mut status = SystemBalanceStatus::unavailable("test");
        status.total_mbps = 12.0;
        status.total_avg_mbps = 8.0;
        status.tcp_flows = 4;
        status.udp_flows = 2;
        zero_live_status(&mut status);
        assert_eq!(status.total_mbps, 0.0);
        assert_eq!(status.total_avg_mbps, 0.0);
        assert_eq!(status.tcp_flows, 0);
        assert_eq!(status.udp_flows, 0);
    }

    #[test]
    fn ipv4_only_runtime_rejects_ipv6_before_scheduling_a_link() {
        assert!(supports_remote("192.0.2.1:443".parse().unwrap()));
        assert!(!supports_remote("[2001:db8::1]:443".parse().unwrap()));
        assert!(!supports_remote("[ff02::fb]:5353".parse().unwrap()));
    }
}
