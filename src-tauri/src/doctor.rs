use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

use serde::Serialize;
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};

use crate::network::discovery::{build_link_candidate, LinkCandidate};
use crate::network::connectivity::{
    internet_access_by_adapter, normalize_adapter_guid, InternetAccess,
};
use crate::network::iphlpapi::{get_default_routes, list_adapters, IF_TYPE_IEEE80211};
use crate::network::verify_bind::run_probe;
use crate::network::wlan::{parse_guid_braced, WlanHandle};
use crate::network::wmi::query_network_adapters;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub family: String,
    pub outcome: String,
    pub bound_correctly: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkReport {
    pub if_index: u32,
    pub name: String,
    pub kind: String,
    pub stable_key: String,
    pub ips: Vec<String>,
    pub internet_access: String,
    pub probes: Vec<ProbeResult>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DoctorReport {
    pub links: Vec<LinkReport>,
    pub warnings: Vec<String>,
}

/// Raccoglie discovery + validazione binding (§10 punto 6 delle note
/// tecniche, criterio di uscita Milestone 1). Condivisa tra il comando
/// `dualbond doctor` da terminale (`cli.rs`) e il comando Tauri `doctor`
/// invocato dalla UI.
pub fn collect_report() -> DoctorReport {
    let mut warnings = Vec::new();

    let adapters = match list_adapters() {
        Ok(a) => a,
        Err(e) => {
            warnings.push(format!("Errore GetAdaptersAddresses: {e}"));
            return DoctorReport {
                links: Vec::new(),
                warnings,
            };
        }
    };

    let wmi_by_index: HashMap<u32, String> = match query_network_adapters() {
        Ok(list) => list
            .into_iter()
            .filter_map(|a| Some((a.interface_index?, a.pnp_device_id?)))
            .collect(),
        Err(e) => {
            warnings.push(format!(
                "Query WMI fallita ({e}), identità USB non disponibile"
            ));
            HashMap::new()
        }
    };

    let wlan_handle = WlanHandle::open().ok();
    let internet_by_adapter = match internet_access_by_adapter() {
        Ok(statuses) => Some(statuses),
        Err(error) => {
            warnings.push(format!(
                "Stato Internet NCSI non disponibile ({error}); uso il rilevamento di compatibilità"
            ));
            None
        }
    };

    let mut candidates: Vec<LinkCandidate> = Vec::new();
    for adapter in &adapters {
        let pnp = wmi_by_index.get(&adapter.if_index).map(String::as_str);
        let wifi = if adapter.if_type == IF_TYPE_IEEE80211 {
            wlan_handle.as_ref().and_then(|h| {
                let guid = parse_guid_braced(&adapter.adapter_name)?;
                match h.query_current_connection(&guid) {
                    Ok(w) => w,
                    Err(e) => {
                        warnings.push(format!(
                            "Query SSID/segnale fallita per '{}': {e}",
                            adapter.friendly_name
                        ));
                        None
                    }
                }
            })
        } else {
            None
        };
        if let Some(mut c) = build_link_candidate(adapter, pnp, wifi.as_ref()) {
            c.internet_access = candidate_internet_access(
                &adapter.adapter_name,
                internet_by_adapter.as_ref(),
            );
            candidates.push(c);
        }
    }

    if candidates.is_empty() {
        return DoctorReport {
            links: Vec::new(),
            warnings,
        };
    }

    let mut gateways_by_index: HashMap<u32, Vec<IpAddr>> = HashMap::new();
    for family in [AF_INET, AF_INET6] {
        match get_default_routes(family) {
            Ok(routes) => {
                for r in routes {
                    gateways_by_index
                        .entry(r.if_index)
                        .or_default()
                        .push(r.next_hop);
                }
            }
            Err(e) => warnings.push(format!("GetIpForwardTable2 (family={family}) fallita: {e}")),
        }
    }

    // Come `discover_candidates`: la diagnostica mostra solo i percorsi verso
    // Internet. Switch virtuali Hyper-V/WSL, Npcap loopback e simili non hanno
    // una route predefinita e comparirebbero solo come falsi problemi.
    candidates.retain(|c| gateways_by_index.contains_key(&c.if_index));
    if candidates.is_empty() {
        warnings.push("Nessuna scheda di rete ha una route verso Internet.".into());
        return DoctorReport {
            links: Vec::new(),
            warnings,
        };
    }

    let all_indexes: Vec<u32> = candidates.iter().map(|c| c.if_index).collect();

    let links = candidates
        .into_iter()
        .map(|c| {
            let probes = match gateways_by_index.get(&c.if_index) {
                None => vec![ProbeResult {
                    family: "-".into(),
                    outcome: "nessun gateway di default".into(),
                    bound_correctly: false,
                }],
                Some(gateways) => gateways
                    .iter()
                    .map(|&gateway| probe_one(&c, gateway, &all_indexes))
                    .collect(),
            };
            (
                c.clone(),
                LinkReport {
                    if_index: c.if_index,
                    name: c.name,
                    kind: format!("{:?}", c.kind),
                    stable_key: c.stable_key,
                    ips: c.ips.iter().map(IpAddr::to_string).collect(),
                    internet_access: format!("{:?}", c.internet_access),
                    probes,
                },
            )
        })
        .collect::<Vec<_>>();

    DoctorReport {
        links: links.into_iter().map(|(_, report)| report).collect(),
        warnings,
    }
}

/// Come `collect_report`, ma ritorna anche i `LinkCandidate` con gli
/// `IpAddr` reali (persi nella serializzazione JSON di `DoctorReport`,
/// pensata per `LinkReport.ips: Vec<String>`) per i soli link il cui
/// binding è stato validato empiricamente (§4.3: solo questi vanno usati
/// per un download multi-link reale). Usata dal comando CLI
/// `download-multi` per costruire `LinkConnector::Bound`.
pub fn collect_validated_candidates() -> (DoctorReport, Vec<LinkCandidate>) {
    let report = collect_report();
    // Ricostruzione minima invece di una seconda enumerazione completa:
    // `collect_report` già sa quali `if_index` sono validati (probe con
    // `bound_correctly`), qui basta rifiltrare gli stessi adapter reali.
    let validated_indexes: std::collections::HashSet<u32> = report
        .links
        .iter()
        .filter(|l| l.probes.iter().any(|p| p.bound_correctly))
        .map(|l| l.if_index)
        .collect();
    if validated_indexes.is_empty() {
        return (report, Vec::new());
    }
    let candidates = discover_candidates();
    let validated = candidates
        .into_iter()
        .filter(|c| validated_indexes.contains(&c.if_index))
        .collect();
    (report, validated)
}

/// Enumerazione dei `LinkCandidate` reali (stesso percorso usato da
/// `collect_report` internamente, estratto per essere richiamabile senza
/// rifare anche la parte di validazione).
pub fn discover_candidates() -> Vec<LinkCandidate> {
    let adapters = match list_adapters() {
        Ok(a) => a,
        Err(_) => return Vec::new(),
    };
    // Il bilanciatore deve vedere soltanto percorsi che possono davvero
    // raggiungere Internet. Gli switch virtuali Hyper-V/WSL espongono un
    // IPv4 e sembrano Ethernet normali, ma non hanno una route predefinita:
    // includerli farebbe finire parte dei flussi in un vicolo cieco.
    let routable_indexes: std::collections::HashSet<u32> = get_default_routes(AF_INET)
        .unwrap_or_default()
        .into_iter()
        .map(|route| route.if_index)
        .collect();
    if routable_indexes.is_empty() {
        return Vec::new();
    }
    let wmi_by_index: HashMap<u32, String> = query_network_adapters()
        .map(|list| {
            list.into_iter()
                .filter_map(|a| Some((a.interface_index?, a.pnp_device_id?)))
                .collect()
        })
        .unwrap_or_default();
    let wlan_handle = WlanHandle::open().ok();
    let internet_by_adapter = internet_access_by_adapter().ok();

    adapters
        .iter()
        .filter(|adapter| routable_indexes.contains(&adapter.if_index))
        .filter_map(|adapter| {
            let pnp = wmi_by_index.get(&adapter.if_index).map(String::as_str);
            let wifi = if adapter.if_type == IF_TYPE_IEEE80211 {
                wlan_handle.as_ref().and_then(|h| {
                    let guid = parse_guid_braced(&adapter.adapter_name)?;
                    h.query_current_connection(&guid).ok().flatten()
                })
            } else {
                None
            };
            let mut candidate = build_link_candidate(adapter, pnp, wifi.as_ref())?;
            candidate.internet_access = candidate_internet_access(
                &adapter.adapter_name,
                internet_by_adapter.as_ref(),
            );
            Some(candidate)
        })
        .collect()
}

fn candidate_internet_access(
    adapter_name: &str,
    statuses: Option<&HashMap<String, InternetAccess>>,
) -> InternetAccess {
    let Some(statuses) = statuses else {
        return InternetAccess::Unknown;
    };
    normalize_adapter_guid(adapter_name)
        .and_then(|key| statuses.get(&key).copied())
        .unwrap_or(InternetAccess::LocalOnly)
}

fn probe_one(c: &LinkCandidate, gateway: IpAddr, all_indexes: &[u32]) -> ProbeResult {
    let family = if gateway.is_ipv6() { "IPv6" } else { "IPv4" };
    let Some(local_ip) = c
        .ips
        .iter()
        .find(|ip| ip.is_ipv6() == gateway.is_ipv6())
        .copied()
    else {
        return ProbeResult {
            family: family.into(),
            outcome: "nessun IP locale della stessa famiglia".into(),
            bound_correctly: false,
        };
    };

    match run_probe(
        c.if_index,
        local_ip,
        gateway,
        all_indexes,
        Duration::from_secs(5),
    ) {
        Ok(outcome) if outcome.bound_correctly => ProbeResult {
            family: family.into(),
            outcome: "OK (binding corretto)".into(),
            bound_correctly: true,
        },
        Ok(outcome) => ProbeResult {
            family: family.into(),
            outcome: format!(
                "FALLITO — {} byte osservati su if {} per {} byte di probe (maggior traffico su if {:?})",
                outcome.expected_delta,
                c.if_index,
                outcome.probe_payload_bytes,
                outcome.winner_if_index,
            ),
            bound_correctly: false,
        },
        Err(e) => ProbeResult {
            family: family.into(),
            outcome: format!("errore probe: {e}"),
            bound_correctly: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_uses_the_camel_case_names_the_ui_reads() {
        // `src/lib/ipc.ts` legge `boundCorrectly`, `stableKey`, `ifIndex`,
        // `internetAccess`: con i nomi snake_case un esito OK appariva rosso.
        let report = DoctorReport {
            links: vec![LinkReport {
                if_index: 7,
                name: "Wi-Fi".into(),
                kind: "Wifi".into(),
                stable_key: "mac:00:11:22:33:44:55".into(),
                ips: vec!["192.0.2.10".into()],
                internet_access: "Internet".into(),
                probes: vec![ProbeResult {
                    family: "IPv4".into(),
                    outcome: "OK (binding corretto)".into(),
                    bound_correctly: true,
                }],
            }],
            warnings: Vec::new(),
        };
        let json = serde_json::to_value(&report).unwrap();
        let link = &json["links"][0];
        assert_eq!(link["ifIndex"], 7);
        assert_eq!(link["stableKey"], "mac:00:11:22:33:44:55");
        assert_eq!(link["internetAccess"], "Internet");
        assert_eq!(link["probes"][0]["boundCorrectly"], true);
    }
}
