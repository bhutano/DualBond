//! Campionamento del throughput (§6.5 della spec: "Campionamento core
//! 250 ms"). Logica pura, parametrizzata sul tempo trascorso invece che
//! sull'orologio di sistema, per restare testabile in modo deterministico.

use std::collections::VecDeque;
use std::time::Duration;

// Il valore principale deve restare leggibile anche con campioni a 4 Hz:
// un alpha basso assorbe i micro-burst dei contatori di Windows senza
// rendere il display lento a reagire a un trasferimento reale.
const EWMA_ALPHA: f64 = 0.18;
const AVERAGE_WINDOW: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LinkSample {
    pub mbps: f64,
    pub avg_mbps: f64,
    pub peak_mbps: f64,
}

/// Traccia byte cumulativi -> Mbps istantanei/medi(EWMA)/di picco per un
/// singolo link. Un'istanza per link, vive quanto la sessione di monitor
/// (non solo un download: il monitor è vivo anche a riposo, §6.1).
pub struct LinkSampler {
    prev_bytes: Option<u64>,
    ewma_mbps: f64,
    peak_mbps: f64,
    rate_initialized: bool,
    recent_rates: VecDeque<(f64, Duration)>,
    recent_elapsed: Duration,
}

impl Default for LinkSampler {
    fn default() -> Self {
        Self::new()
    }
}

impl LinkSampler {
    pub fn new() -> Self {
        Self {
            prev_bytes: None,
            ewma_mbps: 0.0,
            peak_mbps: 0.0,
            rate_initialized: false,
            recent_rates: VecDeque::new(),
            recent_elapsed: Duration::ZERO,
        }
    }

    /// `cumulative_bytes`: totale byte osservati su questo link finora
    /// (mai decresce all'interno di una sessione). `elapsed`: tempo dal
    /// campione precedente — di norma ~250ms, ma il chiamante lo misura
    /// per davvero invece di assumerlo, per restare corretto anche con
    /// jitter dello scheduler del sistema operativo.
    pub fn sample(&mut self, cumulative_bytes: u64, elapsed: Duration) -> LinkSample {
        let Some(previous) = self.prev_bytes.replace(cumulative_bytes) else {
            // I contatori di Windows sono cumulativi dalla salita
            // dell'interfaccia. Il primo valore è solo una baseline: usarlo
            // come delta produceva il grande picco osservato all'avvio.
            return LinkSample {
                mbps: 0.0,
                avg_mbps: 0.0,
                peak_mbps: 0.0,
            };
        };
        let delta_bytes = cumulative_bytes.saturating_sub(previous);

        let secs = elapsed.as_secs_f64().max(0.001);
        let instantaneous_mbps = (delta_bytes as f64 * 8.0) / secs / 1_000_000.0;

        if self.rate_initialized {
            self.ewma_mbps = EWMA_ALPHA * instantaneous_mbps + (1.0 - EWMA_ALPHA) * self.ewma_mbps;
        } else {
            self.ewma_mbps = instantaneous_mbps;
            self.rate_initialized = true;
        }
        self.peak_mbps = self.peak_mbps.max(instantaneous_mbps);

        self.recent_rates.push_back((instantaneous_mbps, elapsed));
        self.recent_elapsed += elapsed;
        while self.recent_elapsed > AVERAGE_WINDOW {
            let Some((_, oldest_elapsed)) = self.recent_rates.front().copied() else {
                break;
            };
            if self.recent_rates.len() == 1 {
                break;
            }
            self.recent_rates.pop_front();
            self.recent_elapsed = self.recent_elapsed.saturating_sub(oldest_elapsed);
        }
        let weighted_sum: f64 = self
            .recent_rates
            .iter()
            .map(|(mbps, duration)| mbps * duration.as_secs_f64())
            .sum();
        let average_mbps = weighted_sum / self.recent_elapsed.as_secs_f64().max(0.001);

        LinkSample {
            mbps: self.ewma_mbps,
            avg_mbps: average_mbps,
            peak_mbps: self.peak_mbps,
        }
    }

    /// Azzera tutte le velocità mantenendo il contatore corrente come nuova
    /// baseline. Serve quando il link viene escluso o il download si ferma:
    /// alla riattivazione non devono riapparire EWMA/media precedenti né un
    /// falso picco costruito dai byte trascorsi durante lo stop.
    pub fn reset_at(&mut self, cumulative_bytes: u64) {
        self.prev_bytes = Some(cumulative_bytes);
        self.ewma_mbps = 0.0;
        self.peak_mbps = 0.0;
        self.rate_initialized = false;
        self.recent_rates.clear();
        self.recent_elapsed = Duration::ZERO;
    }
}

/// Scaletta dei fondoscala del gauge (§6.3).
///
/// Non è un limite di velocità: è solo la scala di visualizzazione. Arriva a
/// 1 Tbps perché non si può sapere quali connessioni userà l'utente.
pub const SCALE_STEPS: &[f64] = &[
    10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1_000.0, 2_500.0, 5_000.0, 10_000.0, 25_000.0, 50_000.0,
    100_000.0, 250_000.0, 500_000.0, 1_000_000.0,
];

const RISE_HOLD: Duration = Duration::from_secs(2);
const FALL_HOLD: Duration = Duration::from_secs(10);
const FALL_THRESHOLD_FRACTION: f64 = 0.6;

/// Fondoscala del gauge con isteresi (§6.3): sale solo se il totale supera
/// il fondoscala per più di 2s consecutivi, scende solo dopo 10s
/// consecutivi sotto il 60% del fondoscala. Senza isteresi il gauge
/// "respira" a ogni piccola oscillazione del traffico.
pub struct ScaleMaxTracker {
    current: f64,
    time_over: Duration,
    time_under_60pct: Duration,
}

impl Default for ScaleMaxTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ScaleMaxTracker {
    pub fn new() -> Self {
        Self {
            current: SCALE_STEPS[0],
            time_over: Duration::ZERO,
            time_under_60pct: Duration::ZERO,
        }
    }

    pub fn current(&self) -> f64 {
        self.current
    }

    /// Aggiorna il fondoscala con un nuovo campione di Mbps totali e il
    /// tempo trascorso dal campione precedente. Ritorna il fondoscala
    /// (eventualmente) aggiornato.
    pub fn observe(&mut self, total_mbps: f64, elapsed: Duration) -> f64 {
        if total_mbps > self.current {
            self.time_over += elapsed;
            self.time_under_60pct = Duration::ZERO;
            if self.time_over >= RISE_HOLD {
                self.current = next_step_above(self.current, total_mbps);
                self.time_over = Duration::ZERO;
            }
        } else {
            self.time_over = Duration::ZERO;
            if total_mbps < self.current * FALL_THRESHOLD_FRACTION {
                self.time_under_60pct += elapsed;
                if self.time_under_60pct >= FALL_HOLD {
                    self.current = prev_step_covering(total_mbps);
                    self.time_under_60pct = Duration::ZERO;
                }
            } else {
                self.time_under_60pct = Duration::ZERO;
            }
        }
        self.current
    }
}

fn next_step_above(current: f64, value: f64) -> f64 {
    SCALE_STEPS
        .iter()
        .copied()
        .find(|&s| s > current && s >= value)
        .unwrap_or(*SCALE_STEPS.last().unwrap())
}

fn prev_step_covering(value: f64) -> f64 {
    SCALE_STEPS
        .iter()
        .copied()
        .find(|&s| s >= value)
        .unwrap_or(*SCALE_STEPS.last().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_keeps_growing_beyond_2_5_gbps() {
        // 1 Gbps + 700 Mbps + 600 Mbps = 2,3 Gbps, ma anche 10 GbE e oltre
        // devono avere un fondoscala adeguato, non fermarsi a 2.500.
        assert_eq!(next_step_above(1000.0, 2300.0), 2500.0);
        assert_eq!(next_step_above(2500.0, 7000.0), 10_000.0);
        assert_eq!(next_step_above(10_000.0, 30_000.0), 50_000.0);
        assert_eq!(prev_step_covering(8_000.0), 10_000.0);
    }


    #[test]
    fn sampler_computes_instantaneous_mbps_from_byte_delta() {
        let mut s = LinkSampler::new();
        s.sample(10_000_000, Duration::from_millis(250)); // baseline
                                                          // 250ms, 1,000,000 byte -> 8,000,000 bit / 0.25s = 32 Mbps
        let sample = s.sample(11_000_000, Duration::from_millis(250));
        assert!((sample.mbps - 32.0).abs() < 0.01, "mbps={}", sample.mbps);
    }

    #[test]
    fn first_counter_value_is_only_a_baseline_and_never_a_speed_spike() {
        let mut s = LinkSampler::new();
        let sample = s.sample(900_000_000_000, Duration::from_millis(250));
        assert_eq!(sample.mbps, 0.0);
        assert_eq!(sample.avg_mbps, 0.0);
        assert_eq!(sample.peak_mbps, 0.0);
    }

    #[test]
    fn sampler_tracks_peak_across_samples() {
        let mut s = LinkSampler::new();
        s.sample(0, Duration::from_millis(250)); // baseline
        s.sample(1_000_000, Duration::from_millis(250)); // 32 Mbps
        s.sample(1_000_000 + 5_000_000, Duration::from_millis(250)); // molto più alto
        let sample = s.sample(1_000_000 + 5_000_000, Duration::from_millis(250)); // 0 Mbps ora
        assert!(
            sample.peak_mbps > 100.0,
            "il picco deve restare quello del campione precedente"
        );
        assert!(
            sample.mbps > 0.0 && sample.mbps < sample.peak_mbps,
            "la velocità mostrata deve scendere in modo smussato dopo il picco"
        );
    }

    #[test]
    fn sampler_ewma_smooths_toward_repeated_samples() {
        let mut s = LinkSampler::new();
        let mut last = LinkSample {
            mbps: 0.0,
            avg_mbps: 0.0,
            peak_mbps: 0.0,
        };
        s.sample(0, Duration::from_millis(250));
        for i in 0..45 {
            last = s.sample((i + 1) * 1_000_000, Duration::from_millis(250));
        }
        assert!(
            (last.avg_mbps - 32.0).abs() < 0.01,
            "la media mobile deve convergere a 32 Mbps"
        );
        assert!(
            (last.mbps - 32.0).abs() < 0.01,
            "l'EWMA deve convergere a 32 Mbps"
        );
    }

    #[test]
    fn zero_bytes_produces_zero_mbps_not_a_crash() {
        let mut s = LinkSampler::new();
        let sample = s.sample(0, Duration::from_millis(250));
        assert_eq!(sample.mbps, 0.0);
    }

    #[test]
    fn reset_clears_current_average_and_peak_without_losing_the_baseline() {
        let mut s = LinkSampler::new();
        s.sample(1_000, Duration::from_secs(1));
        let active = s.sample(1_001_000, Duration::from_secs(1));
        assert!(active.mbps > 0.0);
        assert!(active.avg_mbps > 0.0);
        assert!(active.peak_mbps > 0.0);

        s.reset_at(2_000_000);
        let resumed = s.sample(2_000_000, Duration::from_secs(1));
        assert_eq!(resumed, LinkSample::default());
    }

    #[test]
    fn scale_tracker_starts_at_lowest_step() {
        let t = ScaleMaxTracker::new();
        assert_eq!(t.current(), SCALE_STEPS[0]);
    }

    #[test]
    fn scale_tracker_does_not_rise_on_a_brief_spike() {
        let mut t = ScaleMaxTracker::new();
        t.observe(50.0, Duration::from_secs(1)); // supera i 10 Mbps ma solo 1s
        assert_eq!(
            t.current(),
            SCALE_STEPS[0],
            "1s sotto la soglia di 2s non deve far salire il fondoscala"
        );
    }

    #[test]
    fn scale_tracker_rises_after_sustained_excess() {
        let mut t = ScaleMaxTracker::new();
        t.observe(50.0, Duration::from_secs(1));
        t.observe(50.0, Duration::from_secs(1));
        let after = t.observe(50.0, Duration::from_secs(1));
        assert!(
            after > SCALE_STEPS[0],
            "oltre 2s consecutivi sopra il fondoscale deve salire"
        );
        assert!(
            after >= 50.0,
            "il nuovo fondoscala deve coprire il valore osservato"
        );
    }

    #[test]
    fn scale_tracker_does_not_fall_on_a_brief_dip() {
        let mut t = ScaleMaxTracker::new();
        t.observe(50.0, Duration::from_secs(1));
        t.observe(50.0, Duration::from_secs(1));
        let risen = t.observe(50.0, Duration::from_secs(1));
        assert!(risen > SCALE_STEPS[0]);

        t.observe(1.0, Duration::from_secs(5)); // sotto il 60%, ma solo 5s
        assert_eq!(
            t.current(),
            risen,
            "5s sotto il 60% (soglia 10s) non deve far scendere il fondoscala"
        );
    }

    #[test]
    fn scale_tracker_falls_after_sustained_low_traffic() {
        let mut t = ScaleMaxTracker::new();
        t.observe(50.0, Duration::from_secs(1));
        t.observe(50.0, Duration::from_secs(1));
        let risen = t.observe(50.0, Duration::from_secs(1));
        assert!(risen > SCALE_STEPS[0]);

        t.observe(1.0, Duration::from_secs(5));
        let after = t.observe(1.0, Duration::from_secs(6));
        assert!(
            after < risen,
            "oltre 10s consecutivi sotto il 60% deve scendere"
        );
    }

    #[test]
    fn scale_tracker_resets_the_hold_timer_on_a_recovery() {
        let mut t = ScaleMaxTracker::new();
        t.observe(50.0, Duration::from_secs(1));
        t.observe(50.0, Duration::from_secs(1));
        let risen = t.observe(50.0, Duration::from_secs(1));

        t.observe(1.0, Duration::from_secs(8)); // quasi ai 10s
        t.observe(50.0, Duration::from_secs(1)); // risale sopra il 60%: azzera il timer
        let after = t.observe(1.0, Duration::from_secs(8));
        assert_eq!(
            after, risen,
            "una risalita in mezzo deve azzerare il conto alla rovescia della discesa"
        );
    }
}
