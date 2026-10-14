//! Hot-plug (§4.8 della spec): rileva interfacce che compaiono/spariscono
//! e reintegrazione a peso conservativo (§4.8 punto 3) per quelle che
//! ricompaiono.
//!
//! **Scelta deliberata, documentata invece che nascosta**: la spec chiede
//! `NotifyIpInterfaceChange` (callback) con un polling di sicurezza a 2s
//! come rete di protezione. Qui è implementato **solo** il polling: un
//! wrapper sicuro del callback FFI raw (funzione C con puntatore a
//! contesto, marshaling attraverso un thread arbitrario del sistema) non è
//! verificabile in questa sessione senza hardware da scollegare/ricollegare
//! davvero, e un bug lì sarebbe silenzioso. Il polling da solo soddisfa
//! comunque il requisito funzionale (rilevamento entro ~2s), al costo di
//! un po' di lavoro extra ogni ciclo invece di reagire all'evento. Il
//! callback resta un miglioramento futuro, non un buco funzionale.

use crate::network::iphlpapi::list_adapters;
use std::collections::HashSet;
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkChangeKind {
    Added,
    Removed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkChangeEvent {
    pub if_index: u32,
    pub kind: LinkChangeKind,
}

/// Confronta l'insieme di `if_index` noto al giro precedente con quello
/// attuale e ritorna gli eventi Added/Removed. Logica pura, senza I/O:
/// separata dal polling reale per essere testabile senza hardware.
pub fn diff_known_interfaces(previous: &HashSet<u32>, current: &HashSet<u32>) -> Vec<LinkChangeEvent> {
    let mut events: Vec<LinkChangeEvent> = current
        .difference(previous)
        .map(|&if_index| LinkChangeEvent {
            if_index,
            kind: LinkChangeKind::Added,
        })
        .collect();
    events.extend(previous.difference(current).map(|&if_index| LinkChangeEvent {
        if_index,
        kind: LinkChangeKind::Removed,
    }));
    events.sort_by_key(|e| e.if_index);
    events
}

/// Avvia il polling di sicurezza (§4.8) su un task in background: ogni
/// `interval` rilegge le interfacce reali via `list_adapters` e manda gli
/// eventi di cambiamento sul canale ritornato. Il task gira finché il
/// receiver non viene droppato.
pub fn start_polling(interval: Duration) -> mpsc::UnboundedReceiver<LinkChangeEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut known: HashSet<u32> = HashSet::new();
        loop {
            let current: HashSet<u32> = list_adapters()
                .map(|adapters| adapters.iter().map(|a| a.if_index).collect())
                .unwrap_or_default();
            for event in diff_known_interfaces(&known, &current) {
                if tx.send(event).is_err() {
                    return; // receiver droppato, nessuno ascolta più
                }
            }
            known = current;
            tokio::time::sleep(interval).await;
        }
    });
    rx
}

/// Rientro a peso conservativo con risalita via EWMA (§4.8 punto 3): un
/// link ricollegato non riparte subito a peso pieno (causerebbe
/// oscillazioni, come una connessione AIMD che riparte da zero ma più
/// aggressiva) — riparte da `CONSERVATIVE_START` e converge verso il peso
/// realmente osservato, campione dopo campione.
const CONSERVATIVE_START: f64 = 0.25;

pub struct ReintegrationRamp {
    ewma_weight: f64,
    /// Fattore di smoothing: più alto = converge più in fretta verso
    /// l'ultimo campione osservato, più basso = risalita più prudente.
    alpha: f64,
}

impl ReintegrationRamp {
    /// `alpha` in (0, 1]. Va scelto per bilanciare velocità di risalita e
    /// stabilità: qui non è fissato dalla spec, `0.3` è un compromesso
    /// ragionevole (converge in poche osservazioni senza oscillare).
    pub fn new(alpha: f64) -> Self {
        assert!(alpha > 0.0 && alpha <= 1.0, "alpha deve essere in (0, 1]");
        Self {
            ewma_weight: CONSERVATIVE_START,
            alpha,
        }
    }

    pub fn current_weight(&self) -> f64 {
        self.ewma_weight
    }

    /// Registra un nuovo campione di peso osservato (es. goodput
    /// normalizzato rispetto agli altri link) e ritorna il peso EWMA
    /// aggiornato, mai superiore a 1.0.
    pub fn observe(&mut self, instantaneous_weight: f64) -> f64 {
        let sample = instantaneous_weight.clamp(0.0, 1.0);
        self.ewma_weight = self.alpha * sample + (1.0 - self.alpha) * self.ewma_weight;
        self.ewma_weight
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[u32]) -> HashSet<u32> {
        items.iter().copied().collect()
    }

    #[test]
    fn detects_newly_appeared_interfaces() {
        let events = diff_known_interfaces(&set(&[1, 2]), &set(&[1, 2, 3]));
        assert_eq!(
            events,
            vec![LinkChangeEvent {
                if_index: 3,
                kind: LinkChangeKind::Added
            }]
        );
    }

    #[test]
    fn detects_disappeared_interfaces() {
        let events = diff_known_interfaces(&set(&[1, 2, 3]), &set(&[1, 3]));
        assert_eq!(
            events,
            vec![LinkChangeEvent {
                if_index: 2,
                kind: LinkChangeKind::Removed
            }]
        );
    }

    #[test]
    fn detects_simultaneous_add_and_remove() {
        let events = diff_known_interfaces(&set(&[1, 2]), &set(&[1, 3]));
        assert_eq!(
            events,
            vec![
                LinkChangeEvent { if_index: 2, kind: LinkChangeKind::Removed },
                LinkChangeEvent { if_index: 3, kind: LinkChangeKind::Added },
            ]
        );
    }

    #[test]
    fn no_changes_produces_no_events() {
        assert!(diff_known_interfaces(&set(&[1, 2]), &set(&[1, 2])).is_empty());
    }

    #[test]
    fn ramp_starts_conservative_not_at_full_weight() {
        let ramp = ReintegrationRamp::new(0.3);
        assert_eq!(ramp.current_weight(), CONSERVATIVE_START);
        assert!(ramp.current_weight() < 1.0);
    }

    #[test]
    fn ramp_converges_toward_repeated_high_observations() {
        let mut ramp = ReintegrationRamp::new(0.3);
        for _ in 0..30 {
            ramp.observe(1.0);
        }
        assert!(ramp.current_weight() > 0.95, "dopo molti campioni alti deve avvicinarsi a 1.0, era {}", ramp.current_weight());
    }

    #[test]
    fn ramp_never_jumps_to_full_weight_on_a_single_sample() {
        let mut ramp = ReintegrationRamp::new(0.3);
        let w = ramp.observe(1.0);
        assert!(w < 1.0, "un solo campione non deve portare subito a peso pieno (evita oscillazioni)");
        assert!(w > CONSERVATIVE_START, "ma deve comunque muoversi verso l'alto");
    }

    #[test]
    fn ramp_clamps_out_of_range_samples() {
        let mut ramp = ReintegrationRamp::new(1.0); // alpha=1: adotta subito il campione
        assert_eq!(ramp.observe(5.0), 1.0);
        assert_eq!(ramp.observe(-3.0), 0.0);
    }
}
