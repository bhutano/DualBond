//! Dimensionamento adattivo dei segmenti (§4.4 della spec): un segmento
//! troppo piccolo su un link ad alta latenza non esce mai dallo slow start
//! (termine BDP nel limite inferiore); uno troppo grande su un link lento
//! blocca il tail-splitting troppo a lungo (limite superiore fisso).

use std::time::Duration;

const TARGET_TIME_SECS: f64 = 4.0;
const MIN_SEGMENT_BYTES: f64 = 1024.0 * 1024.0; // 1 MiB
const MAX_SEGMENT_BYTES: f64 = 64.0 * 1024.0 * 1024.0; // 64 MiB
const BDP_MULTIPLIER: f64 = 8.0;

/// `ewma_bps`: throughput osservato del link, in bit/s (media mobile
/// esponenziale). `rtt`: round-trip time osservato del link.
pub fn segment_size_bytes(ewma_bps: f64, rtt: Duration) -> u64 {
    let bytes_per_sec = ewma_bps / 8.0;
    let bdp_bytes = bytes_per_sec * rtt.as_secs_f64();
    // Se il termine BDP supera pure il tetto (banda e RTT entrambi enormi),
    // il limite inferiore va comunque tenuto <= al superiore: `f64::clamp`
    // richiede min <= max, altrimenti panica.
    let lower_bound = MIN_SEGMENT_BYTES.max(BDP_MULTIPLIER * bdp_bytes).min(MAX_SEGMENT_BYTES);
    let target = bytes_per_sec * TARGET_TIME_SECS;

    target.clamp(lower_bound, MAX_SEGMENT_BYTES).round() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_bandwidth_floors_to_minimum_segment() {
        let size = segment_size_bytes(0.0, Duration::from_millis(0));
        assert_eq!(size, 1024 * 1024);
    }

    #[test]
    fn moderate_link_uses_target_time_sizing() {
        // 100 Mbps, RTT 10 ms: 8*bdp (~1MB) resta sotto il minimo di 1 MiB,
        // quindi il limite inferiore è il floor fisso, non il BDP.
        let size = segment_size_bytes(100_000_000.0, Duration::from_millis(10));
        assert_eq!(size, 50_000_000); // 12.5 MB/s * 4s
    }

    #[test]
    fn high_bandwidth_high_latency_link_caps_at_64_mib() {
        // 500 Mbps, RTT 45 ms (5G): target_time supera di gran lunga il tetto.
        let size = segment_size_bytes(500_000_000.0, Duration::from_millis(45));
        assert_eq!(size, 64 * 1024 * 1024);
    }

    #[test]
    fn high_latency_raises_floor_above_target_time_when_needed() {
        // Banda enorme ma RTT enorme: 8*bdp supera pure il tetto di 64 MiB,
        // il clamp deve comunque restare al tetto (mai lower_bound > upper).
        let size = segment_size_bytes(1_000_000_000_000.0, Duration::from_secs(10));
        assert_eq!(size, 64 * 1024 * 1024);
    }

    #[test]
    fn size_never_exceeds_64_mib_upper_bound() {
        let size = segment_size_bytes(10_000_000_000.0, Duration::from_millis(5));
        assert!(size <= 64 * 1024 * 1024);
    }

    #[test]
    fn size_never_below_1_mib_lower_bound() {
        let size = segment_size_bytes(1000.0, Duration::from_millis(1));
        assert!(size >= 1024 * 1024);
    }
}
