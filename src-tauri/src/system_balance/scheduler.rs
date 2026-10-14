use super::model::{LinkPreference, ServiceConfig};
use crate::network::connectivity::InternetAccess;
use crate::network::discovery::{LinkCandidate, LinkKind};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::time::{Duration, Instant};

const REINTEGRATION_START: f64 = 0.25;
const FAILURE_PENALTY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSelection {
    pub key: String,
    pub name: String,
    pub kind: String,
    pub if_index: u32,
    pub local_ip: IpAddr,
    pub metered: bool,
}

#[derive(Debug, Clone)]
pub struct ScheduledLink {
    pub selection: LinkSelection,
    pub online: bool,
    pub internet_access: InternetAccess,
    pub enabled: bool,
    pub active_tcp: u32,
    pub active_udp: u32,
    pub errors: u32,
    pub bytes_in: u64,
    pub bytes_out: u64,
    capacity_ewma: f64,
    reintegration: f64,
    penalized_until: Option<Instant>,
}

impl ScheduledLink {
    fn effective_weight(&self, now: Instant) -> f64 {
        let health = if self.penalized_until.is_some_and(|until| until > now) {
            0.15
        } else {
            1.0
        };
        self.capacity_ewma.clamp(0.25, 4.0) * self.reintegration.clamp(0.25, 1.0) * health
    }

    pub fn is_penalized(&self) -> bool {
        self.penalized_until
            .is_some_and(|until| until > Instant::now())
    }
}

#[derive(Default)]
pub struct FlowScheduler {
    links: HashMap<String, ScheduledLink>,
    cursor: u64,
}

impl FlowScheduler {
    pub fn refresh(&mut self, candidates: Vec<LinkCandidate>, config: &ServiceConfig) {
        let mut seen = HashSet::new();
        for candidate in candidates {
            let Some(local_ip) = crate::network::discovery::preferred_local_ip(&candidate.ips)
            else {
                continue;
            };
            let key = candidate.stable_key.clone();
            seen.insert(key.clone());
            let preference = config.links.get(&key).cloned().unwrap_or_default();
            let kind = kind_name(&candidate.kind).to_owned();
            self.links
                .entry(key.clone())
                .and_modify(|link| {
                    link.selection.name = candidate.name.clone();
                    link.selection.kind = kind.clone();
                    link.selection.if_index = candidate.if_index;
                    link.selection.local_ip = local_ip;
                    link.selection.metered = preference.metered;
                    link.enabled = preference.enabled;
                    link.internet_access = candidate.internet_access;
                    if !link.online {
                        link.reintegration = REINTEGRATION_START;
                    }
                    link.online = true;
                })
                .or_insert(ScheduledLink {
                    selection: LinkSelection {
                        key,
                        name: candidate.name,
                        kind,
                        if_index: candidate.if_index,
                        local_ip,
                        metered: preference.metered,
                    },
                    online: true,
                    internet_access: candidate.internet_access,
                    enabled: preference.enabled,
                    active_tcp: 0,
                    active_udp: 0,
                    errors: 0,
                    bytes_in: 0,
                    bytes_out: 0,
                    capacity_ewma: 1.0,
                    reintegration: REINTEGRATION_START,
                    penalized_until: None,
                });
        }
        for (key, link) in &mut self.links {
            if !seen.contains(key) {
                link.online = false;
            }
        }
    }

    /// Sceglie e prenota un link in un'unica operazione. Il contatore viene
    /// incrementato prima di rilasciare il lock esterno dello scheduler, così
    /// una raffica di task non può prendere decisioni sugli stessi valori.
    pub fn reserve(&mut self, config: &ServiceConfig, udp: bool) -> Option<LinkSelection> {
        let now = Instant::now();
        let cursor = self.cursor;
        self.cursor = self.cursor.wrapping_add(1);

        let eligible: Vec<&ScheduledLink> = self.eligible_links(config).collect();
        // Un link che ha appena fallito non deve ricevere altri flussi finche'
        // esiste un'alternativa sana. Il peso ridotto da solo non basta: con
        // diverse connessioni attive sul link buono, quello guasto tornava
        // comunque a essere selezionato.
        let have_healthy = eligible.iter().any(|link| !link.is_penalized());
        let mut scored: Vec<(&ScheduledLink, f64)> = eligible
            .into_iter()
            .filter(|link| !have_healthy || !link.is_penalized())
            .map(|link| {
                let active = if udp {
                    link.active_udp
                } else {
                    link.active_tcp
                };
                (link, (active as f64 + 1.0) / link.effective_weight(now))
            })
            .collect();
        scored.sort_by(|(left, left_score), (right, right_score)| {
            left_score
                .total_cmp(right_score)
                .then_with(|| left.selection.key.cmp(&right.selection.key))
        });
        let best_score = scored.first()?.1;
        let mut tied: Vec<&ScheduledLink> = scored
            .into_iter()
            .take_while(|(_, score)| (*score - best_score).abs() < 1e-9)
            .map(|(link, _)| link)
            .collect();
        tied.sort_by(|left, right| left.selection.key.cmp(&right.selection.key));
        let key = tied[(cursor as usize) % tied.len()].selection.key.clone();
        let chosen = self.links.get_mut(&key)?;
        if udp {
            chosen.active_udp = chosen.active_udp.saturating_add(1);
        } else {
            chosen.active_tcp = chosen.active_tcp.saturating_add(1);
        }
        Some(chosen.selection.clone())
    }

    /// Link a cui `reserve` può davvero assegnare un flusso: online,
    /// abilitati, con accesso a Internet non escluso; quelli a consumo solo
    /// se non esistono alternative (o se il fallback è consentito).
    fn eligible_links<'a>(&'a self, config: &'a ServiceConfig) -> impl Iterator<Item = &'a ScheduledLink> + 'a {
        let usable = |link: &ScheduledLink| {
            link.online && link.enabled && link.internet_access != InternetAccess::LocalOnly
        };
        let have_unmetered = self
            .links
            .values()
            .any(|link| usable(link) && !link.selection.metered);
        self.links.values().filter(move |link| {
            usable(link)
                && (!link.selection.metered || !have_unmetered || config.allow_metered_fallback)
        })
    }

    /// `true` se almeno un flusso potrebbe uscire da qualche link. Il servizio
    /// lo usa per non catturare il traffico di Windows quando non c'è alcuna
    /// uscita possibile (altrimenti il PC resterebbe senza Internet).
    pub fn has_eligible_link(&self, config: &ServiceConfig) -> bool {
        self.eligible_links(config).next().is_some()
    }

    pub fn flow_finished(&mut self, key: &str, udp: bool) {
        if let Some(link) = self.links.get_mut(key) {
            if udp {
                link.active_udp = link.active_udp.saturating_sub(1);
            } else {
                link.active_tcp = link.active_tcp.saturating_sub(1);
            }
        }
    }

    pub fn record_bytes(&mut self, key: &str, bytes: u64, inbound: bool) {
        if let Some(link) = self.links.get_mut(key) {
            if inbound {
                link.bytes_in = link.bytes_in.saturating_add(bytes);
            } else {
                link.bytes_out = link.bytes_out.saturating_add(bytes);
            }
            let sample = ((bytes as f64) / 65_536.0).clamp(0.25, 4.0);
            link.capacity_ewma = 0.15 * sample + 0.85 * link.capacity_ewma;
            link.reintegration = (0.2 + 0.8 * link.reintegration).min(1.0);
            link.internet_access = InternetAccess::Internet;
            // Un trasferimento riuscito dimostra che il percorso è vivo:
            // non lasciare la UI in "degradato" per un precedente endpoint
            // remoto irraggiungibile.
            link.penalized_until = None;
        }
    }

    pub fn record_failure(&mut self, key: &str) {
        if let Some(link) = self.links.get_mut(key) {
            link.errors = link.errors.saturating_add(1);
            link.penalized_until = Some(Instant::now() + FAILURE_PENALTY);
        }
    }

    pub fn deactivate(&mut self) {
        for link in self.links.values_mut() {
            link.active_tcp = 0;
            link.active_udp = 0;
        }
    }

    pub fn links(&self) -> impl Iterator<Item = &ScheduledLink> {
        self.links.values()
    }
}

fn kind_name(kind: &LinkKind) -> &'static str {
    match kind {
        LinkKind::UsbTether => "usb",
        LinkKind::Ethernet => "ethernet",
        LinkKind::Wifi { .. } => "wifi",
        LinkKind::Other => "other",
    }
}

pub fn preference<'a>(config: &'a mut ServiceConfig, key: &str) -> &'a mut LinkPreference {
    config.links.entry(key.to_owned()).or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(key: &str, index: u32) -> LinkCandidate {
        LinkCandidate {
            if_index: index,
            name: key.into(),
            kind: LinkKind::Ethernet,
            stable_key: key.into(),
            ips: vec![format!("192.168.{index}.2").parse().unwrap()],
            internet_access: InternetAccess::Internet,
        }
    }

    #[test]
    fn has_eligible_link_matches_what_reserve_can_actually_use() {
        // Bug reale: l'unico link (Wi-Fi) disattivato dall'utente, modalità
        // sistema attiva → il tunnel catturava tutto il traffico senza alcuna
        // uscita e il PC restava senza Internet. Il servizio deve poter sapere
        // in anticipo se esiste almeno un link utilizzabile.
        let mut config = ServiceConfig::default();
        preference(&mut config, "wifi").enabled = false;
        let mut scheduler = FlowScheduler::default();
        assert!(!scheduler.has_eligible_link(&config), "nessun link: niente tunnel");

        scheduler.refresh(vec![candidate("wifi", 1)], &config);
        assert!(!scheduler.has_eligible_link(&config), "unico link disattivato");
        assert!(scheduler.reserve(&config, false).is_none());

        let mut local = candidate("lan", 2);
        local.internet_access = InternetAccess::LocalOnly;
        scheduler.refresh(vec![candidate("wifi", 1), local], &config);
        assert!(!scheduler.has_eligible_link(&config), "solo rete locale senza Internet");

        preference(&mut config, "wifi").metered = true;
        preference(&mut config, "wifi").enabled = true;
        scheduler.refresh(vec![candidate("wifi", 1)], &config);
        assert!(scheduler.has_eligible_link(&config), "un link a consumo da solo resta utilizzabile");
        assert!(scheduler.reserve(&config, false).is_some());
    }

    #[test]
    fn disabled_and_offline_links_are_never_selected() {
        let mut config = ServiceConfig::default();
        preference(&mut config, "b").enabled = false;
        let mut scheduler = FlowScheduler::default();
        scheduler.refresh(vec![candidate("a", 1), candidate("b", 2)], &config);
        for _ in 0..10 {
            let chosen = scheduler.reserve(&config, false).unwrap();
            assert_eq!(chosen.key, "a");
            scheduler.flow_finished(&chosen.key, false);
        }
        scheduler.refresh(vec![candidate("b", 2)], &config);
        assert!(scheduler.reserve(&config, false).is_none());
    }

    #[test]
    fn metered_link_is_fallback_when_no_unmetered_link_is_online() {
        let mut config = ServiceConfig::default();
        preference(&mut config, "metered").metered = true;
        let mut scheduler = FlowScheduler::default();
        scheduler.refresh(vec![candidate("free", 1), candidate("metered", 2)], &config);
        for _ in 0..8 {
            let chosen = scheduler.reserve(&config, false).unwrap();
            assert_eq!(chosen.key, "free");
            scheduler.flow_finished(&chosen.key, false);
        }
        scheduler.refresh(vec![candidate("metered", 2)], &config);
        assert_eq!(scheduler.reserve(&config, false).unwrap().key, "metered");
    }

    #[test]
    fn least_connections_spreads_parallel_flows() {
        let config = ServiceConfig::default();
        let mut scheduler = FlowScheduler::default();
        scheduler.refresh(vec![candidate("a", 1), candidate("b", 2)], &config);
        let first = scheduler.reserve(&config, false).unwrap();
        let second = scheduler.reserve(&config, false).unwrap();
        assert_ne!(first.key, second.key);
    }

    #[test]
    fn equal_idle_links_rotate_between_sequential_flows() {
        let config = ServiceConfig::default();
        let mut scheduler = FlowScheduler::default();
        scheduler.refresh(vec![candidate("a", 1), candidate("b", 2)], &config);
        let choices: Vec<String> = (0..4)
            .map(|_| {
                let chosen = scheduler.reserve(&config, false).unwrap();
                scheduler.flow_finished(&chosen.key, false);
                chosen.key
            })
            .collect();
        assert_eq!(choices, vec!["a", "b", "a", "b"]);
    }

    #[test]
    fn counters_saturate_and_failure_is_recorded() {
        let config = ServiceConfig::default();
        let mut scheduler = FlowScheduler::default();
        scheduler.refresh(vec![candidate("a", 1)], &config);
        scheduler.flow_finished("a", false);
        scheduler.record_failure("a");
        let link = scheduler.links().next().unwrap();
        assert_eq!(link.active_tcp, 0);
        assert_eq!(link.errors, 1);
    }

    #[test]
    fn successful_traffic_clears_a_previous_link_penalty() {
        let config = ServiceConfig::default();
        let mut scheduler = FlowScheduler::default();
        scheduler.refresh(vec![candidate("a", 1)], &config);
        scheduler.record_failure("a");
        assert!(scheduler.links().next().unwrap().is_penalized());
        scheduler.record_bytes("a", 1024, true);
        assert!(!scheduler.links().next().unwrap().is_penalized());
    }

    #[test]
    fn penalized_link_is_not_reused_while_a_healthy_link_exists() {
        let config = ServiceConfig::default();
        let mut scheduler = FlowScheduler::default();
        scheduler.refresh(vec![candidate("bad", 1), candidate("good", 2)], &config);
        scheduler.record_failure("bad");
        for _ in 0..20 {
            let chosen = scheduler.reserve(&config, false).unwrap();
            assert_eq!(chosen.key, "good");
            scheduler.flow_finished(&chosen.key, false);
        }
    }

    #[test]
    fn deactivation_clears_all_live_flow_counters() {
        let config = ServiceConfig::default();
        let mut scheduler = FlowScheduler::default();
        scheduler.refresh(vec![candidate("a", 1), candidate("b", 2)], &config);
        let first = scheduler.reserve(&config, false).unwrap();
        let second = scheduler.reserve(&config, true).unwrap();
        assert_ne!(first.key, second.key);
        scheduler.deactivate();
        assert!(scheduler
            .links()
            .all(|link| link.active_tcp == 0 && link.active_udp == 0));
    }

    #[test]
    fn local_only_link_is_not_scheduled_or_counted_as_usable() {
        let config = ServiceConfig::default();
        let mut local = candidate("local", 1);
        local.internet_access = InternetAccess::LocalOnly;
        let mut scheduler = FlowScheduler::default();
        scheduler.refresh(vec![local, candidate("internet", 2)], &config);

        for _ in 0..8 {
            let chosen = scheduler.reserve(&config, false).unwrap();
            assert_eq!(chosen.key, "internet");
            scheduler.flow_finished(&chosen.key, false);
        }
    }

    #[test]
    fn reservation_updates_load_before_the_next_choice() {
        let config = ServiceConfig::default();
        let mut scheduler = FlowScheduler::default();
        scheduler.refresh(vec![candidate("a", 1), candidate("b", 2)], &config);

        let first = scheduler.reserve(&config, false).unwrap();
        let second = scheduler.reserve(&config, false).unwrap();

        assert_ne!(first.key, second.key);
        assert_eq!(
            scheduler.links().map(|link| link.active_tcp).sum::<u32>(),
            2
        );
    }

    #[test]
    fn concurrent_burst_of_flows_stays_balanced_across_equivalent_links() {
        use std::sync::{Arc, Mutex};
        use std::thread;

        let config = ServiceConfig::default();
        let mut scheduler = FlowScheduler::default();
        scheduler.refresh(vec![candidate("a", 1), candidate("b", 2)], &config);
        let scheduler = Arc::new(Mutex::new(scheduler));

        const THREADS: usize = 8;
        const PER_THREAD: usize = 50;
        let mut handles = Vec::with_capacity(THREADS);
        for _ in 0..THREADS {
            let scheduler = Arc::clone(&scheduler);
            let config = config.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..PER_THREAD {
                    // `reserve` sceglie e incrementa il contatore sotto lo
                    // stesso lock: nessuna finestra in cui due thread possono
                    // decidere sugli stessi contatori stantii.
                    scheduler.lock().unwrap().reserve(&config, false).unwrap();
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }

        let scheduler = scheduler.lock().unwrap();
        let counts: Vec<u32> = scheduler.links().map(|link| link.active_tcp).collect();
        let total: u32 = counts.iter().sum();
        assert_eq!(total, (THREADS * PER_THREAD) as u32);
        let (min, max) = (
            *counts.iter().min().unwrap(),
            *counts.iter().max().unwrap(),
        );
        assert!(
            max - min <= 1,
            "distribuzione sbilanciata dopo la raffica concorrente: {counts:?}"
        );
    }
}
