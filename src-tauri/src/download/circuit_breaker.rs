//! Circuit breaker per link (§4.4 della spec): `weight(link) = ewma_goodput
//! × (1 − error_penalty)`. Un link con errori ripetuti smette di ricevere
//! lease e viene riprovato con probe leggeri a intervalli crescenti
//! (5s → 60s) invece di continuare a intasare la coda con tentativi che
//! quasi certamente falliranno di nuovo.
//!
//! Stati: `Closed` (normale) → 3 errori consecutivi → `Open` (esclude il
//! link) → allo scadere del backoff, `HalfOpen` (un solo tentativo di
//! sondaggio) → successo torna `Closed` con backoff azzerato, fallimento
//! torna `Open` con backoff raddoppiato (cap 60s).

use std::time::{Duration, Instant};

const FAILURE_THRESHOLD: u32 = 3;
const INITIAL_BACKOFF: Duration = Duration::from_secs(5);
const MAX_BACKOFF: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Closed,
    /// Il link è escluso fino a `retry_at`; oltre quell'istante è concesso
    /// un solo tentativo di sondaggio (mezzo-aperto), non gestito come
    /// stato a parte per semplicità: `is_available` lo calcola al volo.
    Open { retry_at: Instant },
}

pub struct CircuitBreaker {
    state: State,
    consecutive_failures: u32,
    next_backoff: Duration,
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new()
    }
}

impl CircuitBreaker {
    pub fn new() -> Self {
        Self {
            state: State::Closed,
            consecutive_failures: 0,
            next_backoff: INITIAL_BACKOFF,
        }
    }

    /// `true` se il link può ricevere una nuova lease adesso: chiuso, oppure
    /// aperto ma con backoff scaduto (concede un tentativo di sondaggio).
    pub fn is_available(&self, now: Instant) -> bool {
        match self.state {
            State::Closed => true,
            State::Open { retry_at } => now >= retry_at,
        }
    }

    /// Un tentativo è riuscito: richiude il breaker e azzera il backoff.
    pub fn record_success(&mut self) {
        self.state = State::Closed;
        self.consecutive_failures = 0;
        self.next_backoff = INITIAL_BACKOFF;
    }

    /// Un tentativo è fallito (fallimento di trasporto, non un 429/503 che
    /// è responsabilità del server, non del link — vedi `worker.rs`).
    pub fn record_failure(&mut self, now: Instant) {
        self.consecutive_failures += 1;
        if self.consecutive_failures >= FAILURE_THRESHOLD {
            self.state = State::Open {
                retry_at: now + self.next_backoff,
            };
            self.next_backoff = (self.next_backoff * 2).min(MAX_BACKOFF);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_closed_and_available() {
        let cb = CircuitBreaker::new();
        assert!(cb.is_available(Instant::now()));
    }

    #[test]
    fn stays_closed_below_failure_threshold() {
        let mut cb = CircuitBreaker::new();
        let now = Instant::now();
        cb.record_failure(now);
        cb.record_failure(now);
        assert!(cb.is_available(now), "2 fallimenti < soglia 3, deve restare chiuso");
    }

    #[test]
    fn opens_at_failure_threshold_and_excludes_link() {
        let mut cb = CircuitBreaker::new();
        let now = Instant::now();
        cb.record_failure(now);
        cb.record_failure(now);
        cb.record_failure(now);
        assert!(!cb.is_available(now), "3° fallimento consecutivo deve aprire il breaker");
    }

    #[test]
    fn becomes_available_again_after_backoff_expires() {
        let mut cb = CircuitBreaker::new();
        let t0 = Instant::now();
        for _ in 0..3 {
            cb.record_failure(t0);
        }
        assert!(!cb.is_available(t0 + Duration::from_secs(4)));
        assert!(cb.is_available(t0 + Duration::from_secs(5)), "backoff iniziale è 5s");
    }

    #[test]
    fn success_resets_to_closed_and_clears_backoff() {
        let mut cb = CircuitBreaker::new();
        let t0 = Instant::now();
        for _ in 0..3 {
            cb.record_failure(t0);
        }
        cb.record_success();
        assert!(cb.is_available(t0), "un successo deve richiudere subito il breaker");

        // Backoff azzerato: la prossima serie di fallimenti riparte da 5s,
        // non continua a raddoppiare da dove si era interrotta.
        for _ in 0..3 {
            cb.record_failure(t0);
        }
        assert!(!cb.is_available(t0 + Duration::from_secs(4)));
        assert!(cb.is_available(t0 + Duration::from_secs(5)));
    }

    #[test]
    fn backoff_doubles_on_repeated_open_cycles_and_caps_at_60s() {
        let mut cb = CircuitBreaker::new();
        let mut t = Instant::now();

        // 1° apertura: backoff 5s.
        for _ in 0..3 {
            cb.record_failure(t);
        }
        t += Duration::from_secs(5);
        assert!(cb.is_available(t), "backoff iniziale 5s scaduto");

        // Il sondaggio fallisce di nuovo: soglia già raggiunta, riapre con
        // backoff raddoppiato a 10s.
        cb.record_failure(t);
        assert!(!cb.is_available(t + Duration::from_secs(9)));
        assert!(cb.is_available(t + Duration::from_secs(10)), "2° apertura: backoff raddoppiato a 10s");
        t += Duration::from_secs(10);

        // 3° fallimento del sondaggio: backoff raddoppia ancora, a 20s.
        cb.record_failure(t);
        assert!(!cb.is_available(t + Duration::from_secs(19)));
        assert!(cb.is_available(t + Duration::from_secs(20)), "3° apertura: backoff raddoppiato a 20s");
        t += Duration::from_secs(20);

        // 4° fallimento: usa il backoff corrente, 40s (frutto del
        // raddoppio precedente — il raddoppio si applica al *prossimo*
        // ciclo, non a quello in corso).
        cb.record_failure(t);
        assert!(!cb.is_available(t + Duration::from_secs(39)));
        assert!(cb.is_available(t + Duration::from_secs(40)), "4° apertura: backoff 40s");
        t += Duration::from_secs(40);

        // 5° fallimento: il raddoppio 40s->80s viene catturato dal tetto,
        // resta a 60s.
        cb.record_failure(t);
        assert!(!cb.is_available(t + Duration::from_secs(59)));
        assert!(cb.is_available(t + Duration::from_secs(60)), "5° apertura: raddoppio oltre 80s catturato dal tetto a 60s");
        t += Duration::from_secs(60);

        // 6° fallimento: già al tetto, deve restarci (non continuare a
        // "raddoppiare" 60*2=120 senza cap).
        cb.record_failure(t);
        assert!(!cb.is_available(t + Duration::from_secs(59)));
        assert!(cb.is_available(t + Duration::from_secs(60)), "il backoff resta fermo al tetto di 60s");
    }
}
