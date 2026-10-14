//! Loop del monitor (§6.1: "vivo anche senza download attivi"): rilegge
//! le interfacce reali e i loro contatori ogni 250ms, aggiorna il
//! registro (§3.2/§7) e emette `StatsSnapshot` via evento Tauri `stats`.
//!
//! **Scelta deliberata**: usa `list_adapters()` + `get_if_counters()`
//! (letture locali, economiche) a ogni giro, **non** la validazione
//! empirica del binding (`verify_bind::run_probe`, ~5s per link) — quella
//! resta un'azione a parte (`doctor`/avvio di un download), altrimenti il
//! monitor bloccherebbe la UI per secondi 4 volte al secondo. Di
//! conseguenza lo stato `probing`/`not_validated` di §6.4 non è ancora
//! popolato da questo loop: ogni link presente e abilitato è mostrato
//! `online` finché non sparisce da `list_adapters()`. La validazione vera
//! resta quella di M1/M4 (`doctor`, `download-multi`).

use crate::download::orchestrator::{DownloadHandle, DownloadRunState};
use crate::download::scheduler::SegmentState;
use crate::metrics::sampler::{LinkSampler, ScaleMaxTracker};
use crate::metrics::snapshot::{DownloadStats, LinkStats, StatsSnapshot};
use crate::network::discovery::{
    build_link_candidate, preferred_local_ip, LinkCandidate, LinkKind,
};
use crate::network::iphlpapi::{get_default_routes, get_if_counters, list_adapters, IF_TYPE_IEEE80211};
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};
use crate::network::registry::LinkRegistry;
use crate::network::wlan::{parse_guid_braced, WlanHandle};
use crate::network::wmi::query_network_adapters;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TICK: Duration = Duration::from_millis(250);

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn kind_str(kind: &LinkKind) -> &'static str {
    match kind {
        LinkKind::UsbTether => "usb",
        LinkKind::Wifi { .. } => "wifi",
        LinkKind::Ethernet => "ethernet",
        LinkKind::Other => "other",
    }
}

fn icon_str(kind: &LinkKind) -> &'static str {
    match kind {
        LinkKind::UsbTether => "phone",
        LinkKind::Wifi { .. } => "wifi",
        LinkKind::Ethernet => "ethernet",
        LinkKind::Other => "ethernet",
    }
}

/// Enumerazione leggera dei candidati reali (senza validazione, senza
/// interrogare WMI/WLAN per ogni link a ogni giro — WMI in particolare è
/// costosetto, va fatto una tantum e riusato). Ritorna anche una mappa
/// verso i `WifiInfo` già risolti, per il segnale Wi-Fi.
fn discover_light(wlan: Option<&WlanHandle>) -> Vec<LinkCandidate> {
    let adapters = list_adapters().unwrap_or_default();
    let wmi_by_index: HashMap<u32, String> = query_network_adapters()
        .map(|list| {
            list.into_iter()
                .filter_map(|a| Some((a.interface_index?, a.pnp_device_id?)))
                .collect()
        })
        .unwrap_or_default();

    // Solo le schede con una route di default sono un'uscita verso Internet.
    // Senza questo filtro gli switch virtuali di Hyper-V/WSL ("vEthernet")
    // comparivano come connessioni vive. Se la tabella delle route non è
    // leggibile non si filtra (meglio mostrare troppo che nascondere tutto).
    let gateway_indexes: Option<std::collections::HashSet<u32>> = {
        let mut set = std::collections::HashSet::new();
        let mut any_ok = false;
        for family in [AF_INET, AF_INET6] {
            if let Ok(routes) = get_default_routes(family) {
                any_ok = true;
                set.extend(routes.into_iter().map(|route| route.if_index));
            }
        }
        any_ok.then_some(set)
    };
    adapters
        .iter()
        // La dashboard principale descrive percorsi vivi, non tutte le
        // schede installate: Windows deve segnalarli Up e deve esserci un
        // indirizzo instradabile utilizzabile dal binding.
        .filter(|adapter| {
            adapter.oper_status_up && preferred_local_ip(&adapter.unicast_ips).is_some()
        })
        .filter(|adapter| {
            gateway_indexes
                .as_ref()
                .map_or(true, |indexes| indexes.contains(&adapter.if_index))
        })
        .filter_map(|adapter| {
            let pnp = wmi_by_index.get(&adapter.if_index).map(String::as_str);
            let wifi = if adapter.if_type == IF_TYPE_IEEE80211 {
                wlan.and_then(|h| {
                    let guid = parse_guid_braced(&adapter.adapter_name)?;
                    h.query_current_connection(&guid).ok().flatten()
                })
            } else {
                None
            };
            build_link_candidate(adapter, pnp, wifi.as_ref())
        })
        .collect()
}

/// Avvia il loop di monitor su un task in background. Gira finché il
/// processo vive — non c'è un modo pulito di fermarlo (il monitor deve
/// restare vivo quanto l'app, §6.1), quindi niente handle di stop.
pub fn spawn(
    app: tauri::AppHandle,
    registry: Arc<Mutex<LinkRegistry>>,
    active_download: Arc<Mutex<Option<DownloadHandle>>>,
) {
    tauri::async_runtime::spawn(async move {
        use tauri::Emitter;

        if let Ok(removed) = registry.lock().unwrap().forget_own_tunnel_entries() {
            if removed > 0 {
                tracing::info!(removed, "rimosse dal registro le voci dell'adattatore del tunnel");
            }
        }
        let wlan = WlanHandle::open().ok();
        let mut samplers: HashMap<u32, LinkSampler> = HashMap::new();
        let mut live_samplers: HashMap<u32, LinkSampler> = HashMap::new();
        let mut session_baseline: HashMap<u32, u64> = HashMap::new();
        let mut last_total_bytes: HashMap<u32, u64> = HashMap::new();
        let mut scale_tracker = ScaleMaxTracker::new();
        let mut last_tick = Instant::now();
        let mut previously_online: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut previous_download_state: Option<String> = None;

        loop {
            tokio::time::sleep(TICK).await;
            let now = Instant::now();
            let elapsed = now.duration_since(last_tick);
            last_tick = now;

            // Le velocità mostrate descrivono il traffico di DualBond, non
            // quello generico dell'interfaccia da browser/Windows. Il flag
            // `cancelled` cambia già nel comando IPC, prima che i worker
            // abbiano terminato, così Annulla azzera il display al tick
            // successivo senza trascinarsi dietro l'EWMA.
            let download_is_transferring = active_download
                .lock()
                .unwrap()
                .as_ref()
                .map(|handle| {
                    matches!(*handle.state.lock().unwrap(), DownloadRunState::Running)
                        && !handle.paused.load(std::sync::atomic::Ordering::Acquire)
                        && !handle.cancelled.load(std::sync::atomic::Ordering::Acquire)
                })
                .unwrap_or(false);

            let candidates = discover_light(wlan.as_ref());
            let mut link_stats = Vec::with_capacity(candidates.len());
            let mut total_mbps = 0.0f64;
            let mut total_avg_mbps = 0.0f64;
            let mut max_link_mbps = 0.0f64;

            for c in &candidates {
                let identity = {
                    let reg = registry.lock().unwrap();
                    let metered_hint = false; // NetworkCostType/WinRT non ancora integrato (§3.3)
                    reg.upsert_seen(
                        &c.stable_key,
                        kind_str(&c.kind),
                        Some(&c.name),
                        icon_str(&c.kind),
                        metered_hint,
                        now_unix_ms() / 1000,
                    )
                    .ok()
                };
                let Some(identity) = identity else { continue };

                let counters = get_if_counters(c.if_index).ok();
                let in_octets = counters.as_ref().map(|c| c.in_octets).unwrap_or(0);

                let sampler = samplers.entry(c.if_index).or_default();
                let sample = if download_is_transferring && identity.enabled {
                    sampler.sample(in_octets, elapsed)
                } else {
                    sampler.reset_at(in_octets);
                    Default::default()
                };
                // misura sempre attiva del traffico reale della scheda, per la
                // schermata principale: indipendente da download e interruttori
                let out_octets = counters.as_ref().map(|c| c.out_octets).unwrap_or(0);
                let live = live_samplers
                    .entry(c.if_index)
                    .or_default()
                    .sample(in_octets.saturating_add(out_octets), elapsed);
                total_mbps += sample.mbps;
                total_avg_mbps += sample.avg_mbps;
                max_link_mbps = max_link_mbps.max(sample.mbps);

                let baseline = *session_baseline.entry(c.if_index).or_insert(in_octets);
                let session_bytes = in_octets.saturating_sub(baseline);

                let prev_total = *last_total_bytes.entry(c.if_index).or_insert(in_octets);
                let delta_since_last_tick = in_octets.saturating_sub(prev_total);
                last_total_bytes.insert(c.if_index, in_octets);
                if delta_since_last_tick > 0 {
                    registry
                        .lock()
                        .unwrap()
                        .add_bytes(&c.stable_key, delta_since_last_tick)
                        .ok();
                }

                let signal_percent = match &c.kind {
                    LinkKind::Wifi { signal_percent, .. } => *signal_percent,
                    _ => None,
                };

                let state = if !identity.enabled {
                    "disabled"
                } else {
                    "online"
                };

                link_stats.push(LinkStats {
                    key: identity.key.clone(),
                    name: identity.display_name.clone(),
                    kind: identity.kind.clone(),
                    color: identity.color.clone(),
                    state: state.to_string(),
                    enabled: identity.enabled,
                    metered: identity.metered,
                    mbps: sample.mbps,
                    avg_mbps: sample.avg_mbps,
                    peak_mbps: sample.peak_mbps,
                    live_mbps: live.mbps,
                    live_avg_mbps: live.avg_mbps,
                    bytes: identity.total_bytes + session_bytes,
                    session_bytes,
                    rtt_ms: None, // non ancora misurato in questo loop (vedi note in testa al file)
                    connections: 0,
                    errors: 0,
                    signal_percent,
                });
            }

            // Link noti (registro) ma non più rilevati adesso: offline.
            let seen_keys: std::collections::HashSet<String> =
                link_stats.iter().map(|l| l.key.clone()).collect();
            if let Ok(known) = registry.lock().unwrap().list_all() {
                for id in known {
                    if seen_keys.contains(&id.key) {
                        continue;
                    }
                    link_stats.push(LinkStats {
                        key: id.key,
                        name: id.display_name,
                        kind: id.kind,
                        color: id.color,
                        // Offline descrive la presenza fisica; `enabled`
                        // è una preferenza separata e non deve far sembrare
                        // collegata una scheda memorizzata ma assente.
                        state: "offline".into(),
                        enabled: id.enabled,
                        metered: id.metered,
                        mbps: 0.0,
                        avg_mbps: 0.0,
                        peak_mbps: 0.0,
                        live_mbps: 0.0,
                        live_avg_mbps: 0.0,
                        bytes: id.total_bytes,
                        session_bytes: 0,
                        rtt_ms: None,
                        connections: 0,
                        errors: 0,
                        signal_percent: None,
                    });
                }
            }

            // Transizioni online<->offline (§6.7 "Link scomparso"): un
            // evento solo al cambio di stato, non a ogni giro.
            let currently_online: std::collections::HashSet<String> = link_stats
                .iter()
                .filter(|l| l.state == "online")
                .map(|l| l.key.clone())
                .collect();
            for l in &link_stats {
                let was_online = previously_online.contains(&l.key);
                let is_online = currently_online.contains(&l.key);
                if was_online && !is_online {
                    crate::core::events::push("warn", format!("{} non è più collegato", l.name));
                } else if !was_online && is_online && !previously_online.is_empty() {
                    // `previously_online.is_empty()` esclude il primissimo
                    // giro dopo l'avvio dell'app: altrimenti ogni link già
                    // collegato genererebbe un falso "ricollegato".
                    crate::core::events::push("info", format!("{} collegato", l.name));
                }
            }
            previously_online = currently_online;

            let scale_max = scale_tracker.observe(total_mbps, elapsed);
            let gain_factor = if max_link_mbps > 0.01 {
                total_mbps / max_link_mbps
            } else {
                1.0
            };

            let download = active_download.lock().unwrap().as_ref().map(download_stats);

            // Notifica di sistema a download completato (§6.8) — un solo
            // colpo alla transizione di stato, non a ogni giro finché
            // resta "done".
            if let Some(d) = &download {
                if d.state == "done" && previous_download_state.as_deref() != Some("done") {
                    use tauri_plugin_notification::NotificationExt;
                    let _ = app
                        .notification()
                        .builder()
                        .title("Download completato")
                        .body(&d.name)
                        .show();
                }
                previous_download_state = Some(d.state.clone());
            } else {
                previous_download_state = None;
            }

            // Tray: velocità corrente nel tooltip (§6.8).
            if let Some(tray) = app.tray_by_id("main-tray") {
                let tooltip = if total_mbps > 0.1 {
                    format!("DualBond — {total_mbps:.1} Mbps · media {total_avg_mbps:.1}")
                } else {
                    "DualBond".to_string()
                };
                let _ = tray.set_tooltip(Some(&tooltip));
            }

            let recent_events = crate::core::events::global()
                .recent(50)
                .into_iter()
                .map(|e| crate::metrics::snapshot::RecentEvent {
                    ts: e.ts,
                    level: e.level,
                    text: e.text,
                })
                .collect();

            let snapshot = StatsSnapshot {
                ts: now_unix_ms(),
                links: link_stats,
                total_mbps,
                total_avg_mbps,
                gain_factor,
                scale_max,
                download,
                recent_events,
            };

            let _ = app.emit("stats", snapshot);
        }
    });
}

fn download_stats(handle: &DownloadHandle) -> DownloadStats {
    let sched = handle.scheduler.lock().unwrap();
    let downloaded: u64 = sched.segments().iter().map(|s| s.written).sum();
    let segments_total = sched.segments().len() as u32;
    let segments_done = sched
        .segments()
        .iter()
        .filter(|s| s.state == SegmentState::Done)
        .count() as u32;
    let retries: u32 = sched.segments().iter().map(|s| s.attempts).sum();
    drop(sched);

    let total = handle.total;
    let percent = total
        .filter(|&t| t > 0)
        .map(|t| (downloaded as f64 / t as f64) * 100.0)
        .unwrap_or(0.0);

    let state_str = match &*handle.state.lock().unwrap() {
        DownloadRunState::Running => {
            if handle.paused.load(std::sync::atomic::Ordering::Relaxed) {
                "paused"
            } else {
                "running"
            }
        }
        DownloadRunState::Verifying => "verifying",
        DownloadRunState::Done => "done",
        DownloadRunState::Error(_) => "error",
        DownloadRunState::Cancelled => "cancelled",
    };

    DownloadStats {
        name: handle.name.clone(),
        downloaded,
        total,
        percent,
        eta_sec: None, // richiede una stima di velocità aggregata: TODO M7
        segments_done,
        segments_total,
        retries,
        state: state_str.to_string(),
    }
}
