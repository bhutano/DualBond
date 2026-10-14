//! Controllo adattivo della concorrenza per link (AIMD, §4.4 della spec):
//! parte da 4 connessioni, sale di 1 quando il goodput migliora senza
//! peggiorare RTT/errori, dimezza (mai sotto 2) su errori o regressione di
//! goodput, altrimenti resta invariato (isteresi al plateau).

const INITIAL_CONNECTIONS: u32 = 4;
const MIN_CONNECTIONS: u32 = 2;
const MAX_CONNECTIONS: u32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AimdController {
    connections: u32,
}

/// Delta e tassi misurati nell'intervallo di controllo (10 s, §4.4):
/// percentuali espresse come frazioni (`0.07` = 7%), non come 0-100.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntervalStats {
    pub delta_goodput: f64,
    pub err_rate: f64,
    pub delta_rtt: f64,
    /// Risposta 429/503 osservata nell'intervallo: forza il dimezzamento
    /// indipendentemente dagli altri valori (§4.4, §4.7).
    pub saw_429_or_503: bool,
}

impl Default for AimdController {
    fn default() -> Self {
        Self::new()
    }
}

impl AimdController {
    pub fn new() -> Self {
        Self {
            connections: INITIAL_CONNECTIONS,
        }
    }

    pub fn connections(&self) -> u32 {
        self.connections
    }

    /// Applica una decisione AIMD per l'intervallo trascorso. Le due
    /// condizioni sono valutate nell'ordine della spec: prima il
    /// peggioramento (dimezza), solo se non peggiora si valuta il
    /// miglioramento (incrementa) — un intervallo non può fare entrambe le
    /// cose.
    pub fn update(&mut self, stats: IntervalStats) {
        let should_halve =
            stats.delta_goodput < -0.03 || stats.err_rate > 0.05 || stats.saw_429_or_503;
        let should_grow =
            stats.delta_goodput > 0.07 && stats.err_rate < 0.02 && stats.delta_rtt < 0.30;

        if should_halve {
            self.connections = MIN_CONNECTIONS.max(self.connections / 2);
        } else if should_grow {
            self.connections = MAX_CONNECTIONS.min(self.connections + 1);
        }
        // altrimenti invariato: isteresi al plateau
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stable() -> IntervalStats {
        IntervalStats {
            delta_goodput: 0.0,
            err_rate: 0.0,
            delta_rtt: 0.0,
            saw_429_or_503: false,
        }
    }

    #[test]
    fn starts_at_4_connections() {
        assert_eq!(AimdController::new().connections(), 4);
    }

    #[test]
    fn grows_by_one_on_clear_improvement() {
        let mut c = AimdController::new();
        c.update(IntervalStats {
            delta_goodput: 0.10,
            err_rate: 0.0,
            delta_rtt: 0.05,
            saw_429_or_503: false,
        });
        assert_eq!(c.connections(), 5);
    }

    #[test]
    fn never_grows_past_16() {
        let mut c = AimdController::new();
        for _ in 0..30 {
            c.update(IntervalStats {
                delta_goodput: 0.50,
                err_rate: 0.0,
                delta_rtt: 0.0,
                saw_429_or_503: false,
            });
        }
        assert_eq!(c.connections(), 16);
    }

    #[test]
    fn halves_on_goodput_regression() {
        let mut c = AimdController::new(); // 4
        c.update(IntervalStats {
            delta_goodput: -0.10,
            ..stable()
        });
        assert_eq!(c.connections(), 2);
    }

    #[test]
    fn halves_on_high_error_rate() {
        let mut c = AimdController::new();
        c.update(IntervalStats {
            err_rate: 0.10,
            ..stable()
        });
        assert_eq!(c.connections(), 2);
    }

    #[test]
    fn halves_on_429_or_503_regardless_of_other_stats() {
        let mut c = AimdController::new();
        c.update(IntervalStats {
            delta_goodput: 0.50, // altrimenti crescerebbe
            saw_429_or_503: true,
            ..stable()
        });
        assert_eq!(c.connections(), 2);
    }

    #[test]
    fn never_drops_below_2() {
        let mut c = AimdController::new();
        c.update(IntervalStats {
            err_rate: 0.10,
            ..stable()
        }); // 4 -> 2
        c.update(IntervalStats {
            err_rate: 0.10,
            ..stable()
        }); // 2 -> max(2, 1) = 2
        assert_eq!(c.connections(), 2);
    }

    #[test]
    fn stays_flat_on_plateau() {
        let mut c = AimdController::new();
        c.update(stable());
        assert_eq!(c.connections(), 4, "isteresi: nessun cambiamento nel plateau");
    }

    #[test]
    fn small_improvement_below_threshold_does_not_grow() {
        let mut c = AimdController::new();
        c.update(IntervalStats {
            delta_goodput: 0.05, // sotto la soglia +7%
            ..stable()
        });
        assert_eq!(c.connections(), 4);
    }

    #[test]
    fn rtt_regression_blocks_growth_even_with_good_goodput() {
        let mut c = AimdController::new();
        c.update(IntervalStats {
            delta_goodput: 0.20,
            err_rate: 0.0,
            delta_rtt: 0.40, // sopra la soglia +30%
            saw_429_or_503: false,
        });
        assert_eq!(c.connections(), 4, "RTT in peggioramento deve bloccare la crescita");
    }
}
