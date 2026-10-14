use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use socket2::SockAddr;

use crate::core::errors::DualBondError;
use crate::network::iphlpapi::get_if_counters;
use crate::network::socket_bind::bind_udp_socket_to_interface;

#[derive(Debug, Clone, PartialEq)]
pub struct VerifyOutcome {
    pub expected_if_index: u32,
    /// Interfaccia con il maggior delta `OutOctets` durante il probe.
    /// `None` se nessuna interfaccia ha trasmesso o se il risultato è
    /// ambiguo (pareggio tra due interfacce).
    pub winner_if_index: Option<u32>,
    /// Byte realmente osservati sul link che il probe voleva validare.
    pub expected_delta: u64,
    /// Byte di payload accettati dal socket durante il probe.
    pub probe_payload_bytes: u64,
    pub bound_correctly: bool,
}

/// Decide quale interfaccia ha effettivamente trasmesso i pacchetti del
/// probe confrontando i delta di `OutOctets` prima/dopo (§4.3, decisione
/// 9.1 delle note tecniche): intercetta il caso di collisione di subnet
/// dove l'ARP risolve sull'interfaccia sbagliata nonostante il bind
/// esplicito via `IP_UNICAST_IF`.
pub fn evaluate_counters(
    expected_if_index: u32,
    probe_payload_bytes: u64,
    before: &HashMap<u32, u64>,
    after: &HashMap<u32, u64>,
) -> VerifyOutcome {
    let mut deltas: Vec<(u32, u64)> = after
        .iter()
        .filter_map(|(&if_index, &after_val)| {
            let before_val = *before.get(&if_index)?;
            Some((if_index, after_val.saturating_sub(before_val)))
        })
        .filter(|(_, delta)| *delta > 0)
        .collect();
    deltas.sort_by(|a, b| b.1.cmp(&a.1));

    let winner_if_index = match deltas.as_slice() {
        [] => None,
        [(only, _)] => Some(*only),
        [(first, first_delta), (_, second_delta), ..] if first_delta > second_delta => Some(*first),
        _ => None, // pareggio sul delta massimo: ambiguo
    };

    let expected_delta = after
        .get(&expected_if_index)
        .zip(before.get(&expected_if_index))
        .map(|(after_val, before_val)| after_val.saturating_sub(*before_val))
        .unwrap_or(0);

    // `OutOctets` include anche gli header, quindi quando il socket ha
    // davvero usato il link atteso il delta deve essere almeno pari al
    // payload consegnato allo stack. Il traffico sulle altre interfacce è
    // irrilevante: prima sceglievamo il delta massimo globale e un normale
    // aggiornamento/app in background sul Wi-Fi faceva bocciare un tether
    // USB perfettamente funzionante.
    let bound_correctly = probe_payload_bytes > 0 && expected_delta >= probe_payload_bytes;

    VerifyOutcome {
        expected_if_index,
        winner_if_index,
        expected_delta,
        probe_payload_bytes,
        bound_correctly,
    }
}

fn snapshot_counters(if_indexes: &[u32]) -> Result<HashMap<u32, u64>, DualBondError> {
    let mut map = HashMap::new();
    for &idx in if_indexes {
        map.insert(idx, get_if_counters(idx)?.out_octets);
    }
    Ok(map)
}

/// Esegue il probe di validazione empirica (§4.3, decisione 9.1 delle note
/// tecniche): invia datagrammi UDP verso il gateway del link per
/// `duration`, poi confronta i delta `OutOctets` di tutte le interfacce
/// candidate per determinare su quale ha effettivamente viaggiato il
/// traffico. Nessun host esterno oltre al gateway del link stesso.
pub fn run_probe(
    if_index: u32,
    local_ip: IpAddr,
    gateway_ip: IpAddr,
    candidate_if_indexes: &[u32],
    duration: Duration,
) -> Result<VerifyOutcome, DualBondError> {
    let socket = bind_udp_socket_to_interface(if_index, local_ip)?;
    let target = SockAddr::from(SocketAddr::new(gateway_ip, 33434));

    let before = snapshot_counters(candidate_if_indexes)?;

    // Pacchetti abbastanza grandi da lasciare un'impronta inequivocabile
    // nei contatori, pur restando un traffico locale trascurabile verso il
    // gateway (circa 24 KB/s con l'intervallo sotto).
    let payload = [0u8; 1200];
    let mut sent_payload_bytes = 0u64;
    let start = Instant::now();
    while start.elapsed() < duration {
        if let Ok(sent) = socket.send_to(&payload, &target) {
            sent_payload_bytes += sent as u64;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let after = snapshot_counters(candidate_if_indexes)?;
    Ok(evaluate_counters(
        if_index,
        sent_payload_bytes,
        &before,
        &after,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(u32, u64)]) -> HashMap<u32, u64> {
        pairs.iter().copied().collect()
    }

    #[test]
    fn expected_interface_is_the_only_one_that_transmitted() {
        let before = map(&[(1, 1000), (2, 5000)]);
        let after = map(&[(1, 1512), (2, 5000)]);
        let outcome = evaluate_counters(1, 400, &before, &after);
        assert_eq!(
            outcome,
            VerifyOutcome {
                expected_if_index: 1,
                winner_if_index: Some(1),
                expected_delta: 512,
                probe_payload_bytes: 400,
                bound_correctly: true,
            }
        );
    }

    #[test]
    fn wrong_interface_transmitted_more_detects_subnet_collision() {
        let before = map(&[(1, 1000), (2, 5000)]);
        let after = map(&[(1, 1050), (2, 6000)]); // if 2 ha trasmesso molto di piu'
        let outcome = evaluate_counters(1, 512, &before, &after);
        assert_eq!(outcome.winner_if_index, Some(2));
        assert!(!outcome.bound_correctly);
    }

    #[test]
    fn no_interface_transmitted_anything() {
        let before = map(&[(1, 1000), (2, 5000)]);
        let after = map(&[(1, 1000), (2, 5000)]);
        let outcome = evaluate_counters(1, 512, &before, &after);
        assert_eq!(outcome.winner_if_index, None);
        assert!(!outcome.bound_correctly);
    }

    #[test]
    fn a_busy_other_interface_does_not_invalidate_the_expected_probe() {
        let before = map(&[(1, 1000), (2, 5000)]);
        let after = map(&[(1, 2200), (2, 505_000)]);
        let outcome = evaluate_counters(1, 1200, &before, &after);
        assert_eq!(outcome.winner_if_index, Some(2));
        assert_eq!(outcome.expected_delta, 1200);
        assert!(outcome.bound_correctly);
    }

    #[test]
    fn interface_missing_from_after_snapshot_is_ignored() {
        let before = map(&[(1, 1000), (2, 5000)]);
        let after = map(&[(1, 1100)]); // if 2 sparita (es. disconnessa nel frattempo)
        let outcome = evaluate_counters(1, 80, &before, &after);
        assert_eq!(outcome.winner_if_index, Some(1));
        assert!(outcome.bound_correctly);
    }

    #[test]
    fn unrelated_background_on_expected_link_is_not_enough_without_the_probe_volume() {
        let before = map(&[(1, 1000), (2, 5000)]);
        let after = map(&[(1, 1050), (2, 5000)]);
        let outcome = evaluate_counters(1, 1200, &before, &after);
        assert_eq!(outcome.expected_delta, 50);
        assert!(!outcome.bound_correctly);
    }
}
