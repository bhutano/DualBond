//! Scheduler work-stealing (§4.4 della spec): il file è diviso in una coda
//! condivisa di segmenti; ogni worker libero preleva (lease) il prossimo
//! libero. Nessuna percentuale a priori — il rapporto tra link emerge dal
//! throughput reale. Logica pura in memoria: nessuna I/O, nessun accesso al
//! DB (la persistenza del watermark durevole resta responsabilità del
//! chiamante, esattamente come in Milestone 3 — `report_progress` va
//! chiamato solo dopo `sync_data()` + commit, mai prima).

/// Tentativi falliti totali (su qualunque link) oltre i quali un segmento
/// è dichiarato irraggiungibile invece di essere rimesso in coda
/// all'infinito. Non è un limite "per link" (quello lo gestisce il circuit
/// breaker, `circuit_breaker.rs`) — è l'argine finale: se anche provando
/// tutti i link disponibili un segmento non scende mai sotto questa soglia
/// di fallimenti, il file non è scaricabile così com'è, e va detto
/// all'utente invece di girare in silenzio per sempre (gap scoperto
/// testando con hardware reale, vedi `docs/milestone4-notes.md`).
pub const MAX_SEGMENT_ATTEMPTS: u32 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentState {
    Free,
    Leased,
    Done,
    /// Tentativi falliti >= `MAX_SEGMENT_ATTEMPTS`: irraggiungibile, il
    /// download va abortito con un errore chiaro (§4.7 non copre questo
    /// caso esplicitamente, ma "girare per sempre" non è un'opzione).
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub idx: i64,
    pub start: u64,
    pub end: u64, // inclusivo
    /// Byte già durevoli (fsync + commit) dall'inizio del segmento.
    pub written: u64,
    pub state: SegmentState,
    pub lease_link: Option<String>,
    /// Tentativi falliti totali su questo segmento, su qualunque link.
    pub attempts: u32,
}

impl Segment {
    pub fn len(&self) -> u64 {
        self.end - self.start + 1
    }

    /// Punto da cui riprendere una lease (nuova o rientrante dopo scadenza):
    /// mai da zero se byte durevoli sono già presenti (§4.5 "Ripresa").
    pub fn resume_offset(&self) -> u64 {
        self.start + self.written
    }

    fn residual(&self) -> u64 {
        self.end - self.resume_offset() + 1
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentLease {
    pub idx: i64,
    pub start: u64,
    pub end: u64,
    pub resume_from: u64,
}

/// Esito di un tail-split riuscito: oltre alla nuova lease, il chiamante
/// deve riflettere sul DB anche l'accorciamento del segmento vittima
/// (`victim_idx`/`victim_new_end`) — lo scheduler è in memoria, la
/// persistenza resta responsabilità del chiamante (stessa separazione di
/// `report_progress`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailSteal {
    pub lease: SegmentLease,
    pub victim_idx: i64,
    pub victim_new_end: u64,
}

pub struct Scheduler {
    segments: Vec<Segment>,
    next_idx: i64,
}

impl Scheduler {
    /// Divide `[0, total)` in segmenti di `initial_size` byte (l'ultimo può
    /// essere più corto). Il sizing successivo dei segmenti creati da
    /// tail-splitting usa la dimensione passata a `try_steal_tail`, non
    /// questa — il dimensionamento per-link è responsabilità del chiamante
    /// (vedi `chunk::segment_size_bytes`).
    pub fn new(total: u64, initial_size: u64) -> Self {
        assert!(initial_size > 0, "initial_size deve essere > 0");
        let mut segments = Vec::new();
        let mut start = 0u64;
        let mut idx = 0i64;
        while start < total {
            let end = (start + initial_size - 1).min(total - 1);
            segments.push(Segment {
                idx,
                start,
                end,
                written: 0,
                state: SegmentState::Free,
                lease_link: None,
                attempts: 0,
            });
            start = end + 1;
            idx += 1;
        }
        Self {
            segments,
            next_idx: idx,
        }
    }

    /// Costruisce lo scheduler da segmenti già esistenti (ripresa da DB
    /// dopo un riavvio: watermark e stato caricati da `chunk`, §4.5).
    pub fn from_segments(segments: Vec<Segment>) -> Self {
        let next_idx = segments.iter().map(|s| s.idx).max().map(|m| m + 1).unwrap_or(0);
        Self { segments, next_idx }
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn all_done(&self) -> bool {
        self.segments.iter().all(|s| s.state == SegmentState::Done)
    }

    fn find_idx(&self, idx: i64) -> Option<usize> {
        self.segments.iter().position(|s| s.idx == idx)
    }

    /// Preleva il prossimo segmento libero per `link`. `None` se la coda è
    /// vuota (nessun segmento `Free`) — a quel punto il chiamante valuta il
    /// tail-splitting.
    pub fn lease_next(&mut self, link: &str) -> Option<SegmentLease> {
        let seg = self.segments.iter_mut().find(|s| s.state == SegmentState::Free)?;
        seg.state = SegmentState::Leased;
        seg.lease_link = Some(link.to_string());
        Some(SegmentLease {
            idx: seg.idx,
            start: seg.start,
            end: seg.end,
            resume_from: seg.resume_offset(),
        })
    }

    /// Aggiorna il watermark durevole di un segmento in lease. Va chiamato
    /// **dopo** la barriera di durabilità (fsync + commit DB), mai prima —
    /// stessa invariante di Milestone 3.
    pub fn report_progress(&mut self, idx: i64, written: u64) {
        if let Some(i) = self.find_idx(idx) {
            let seg = &mut self.segments[i];
            debug_assert!(written <= seg.len(), "written non può superare la lunghezza del segmento");
            seg.written = written;
        }
    }

    pub fn complete(&mut self, idx: i64) {
        if let Some(i) = self.find_idx(idx) {
            self.segments[i].state = SegmentState::Done;
            self.segments[i].written = self.segments[i].len();
            self.segments[i].lease_link = None;
        }
    }

    /// I segmenti `Leased` la cui lease è scaduta tornano `Free`: verranno
    /// ripresi da `start + written`, mai da zero (§4.5 punto 4).
    pub fn release_lease(&mut self, idx: i64) {
        if let Some(i) = self.find_idx(idx) {
            let seg = &mut self.segments[i];
            if seg.state == SegmentState::Leased {
                seg.state = SegmentState::Free;
                seg.lease_link = None;
            }
        }
    }

    /// Un tentativo su questo segmento è fallito (trasporto, status
    /// inatteso, ETag divergente, corpo troncato — mai per un 429/503, che
    /// è temporaneo per natura e non conta come vero fallimento del
    /// segmento). Incrementa il contatore tentativi; se supera
    /// `MAX_SEGMENT_ATTEMPTS` il segmento diventa `Failed` invece di
    /// tornare `Free`. Ritorna lo stato risultante.
    pub fn record_failure(&mut self, idx: i64) -> SegmentState {
        let Some(i) = self.find_idx(idx) else {
            return SegmentState::Free; // segmento sconosciuto: nessuno stato sensato da riportare
        };
        let seg = &mut self.segments[i];
        seg.attempts += 1;
        seg.lease_link = None;
        seg.state = if seg.attempts >= MAX_SEGMENT_ATTEMPTS {
            SegmentState::Failed
        } else {
            SegmentState::Free
        };
        seg.state
    }

    /// Un segmento è `Failed` (irraggiungibile su tutti i link provati):
    /// il download va abortito, non ha senso continuare a schedulare gli
    /// altri segmenti. Ritorna il primo trovato, se ce n'è uno.
    pub fn any_failed(&self) -> Option<&Segment> {
        self.segments.iter().find(|s| s.state == SegmentState::Failed)
    }

    /// Tail-splitting (§4.4, mitigazione straggler): se la coda è vuota e un
    /// worker è idle, trova il segmento in volo col maggior tempo residuo
    /// stimato (qui: byte residui, proxy diretto a parità di link) e, se il
    /// residuo supera il doppio della dimensione tipica del link idle,
    /// tronca atomicamente il lease lento a metà del residuo e assegna la
    /// seconda metà al worker idle. Nessun byte duplicato, nessun buco.
    pub fn try_steal_tail(&mut self, idle_link: &str, idle_link_segment_size: u64) -> Option<TailSteal> {
        let victim_i = self
            .segments
            .iter()
            .enumerate()
            .filter(|(_, s)| s.state == SegmentState::Leased)
            .max_by_key(|(_, s)| s.residual())
            .map(|(i, _)| i)?;

        let residual = self.segments[victim_i].residual();
        if residual <= 2 * idle_link_segment_size {
            return None;
        }

        let resume_from = self.segments[victim_i].resume_offset();
        let half = residual / 2;
        let split_point = resume_from + half; // primo byte del nuovo segmento
        let original_end = self.segments[victim_i].end;
        let victim_idx = self.segments[victim_i].idx;

        self.segments[victim_i].end = split_point - 1;

        let new_idx = self.next_idx;
        self.next_idx += 1;
        let lease = SegmentLease {
            idx: new_idx,
            start: split_point,
            end: original_end,
            resume_from: split_point,
        };
        self.segments.push(Segment {
            idx: new_idx,
            start: split_point,
            end: original_end,
            written: 0,
            state: SegmentState::Leased,
            lease_link: Some(idle_link.to_string()),
            attempts: 0,
        });
        Some(TailSteal {
            lease,
            victim_idx,
            victim_new_end: split_point - 1,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges(sched: &Scheduler) -> Vec<(u64, u64)> {
        let mut v: Vec<(u64, u64)> = sched.segments().iter().map(|s| (s.start, s.end)).collect();
        v.sort();
        v
    }

    #[test]
    fn splits_total_into_fixed_size_segments_with_shorter_tail() {
        let sched = Scheduler::new(25, 10);
        assert_eq!(ranges(&sched), vec![(0, 9), (10, 19), (20, 24)]);
    }

    #[test]
    fn exact_multiple_has_no_remainder_segment() {
        let sched = Scheduler::new(20, 10);
        assert_eq!(ranges(&sched), vec![(0, 9), (10, 19)]);
    }

    #[test]
    fn lease_next_returns_segments_in_order_and_marks_leased() {
        let mut sched = Scheduler::new(20, 10);
        let l1 = sched.lease_next("link-a").unwrap();
        assert_eq!((l1.start, l1.end, l1.resume_from), (0, 9, 0));
        assert_eq!(sched.segments()[0].state, SegmentState::Leased);

        let l2 = sched.lease_next("link-b").unwrap();
        assert_eq!((l2.start, l2.end), (10, 19));
    }

    #[test]
    fn lease_next_returns_none_when_queue_empty() {
        let mut sched = Scheduler::new(10, 10);
        assert!(sched.lease_next("a").is_some());
        assert!(sched.lease_next("b").is_none());
    }

    #[test]
    fn complete_marks_done_and_fills_written_to_segment_length() {
        let mut sched = Scheduler::new(10, 10);
        let lease = sched.lease_next("a").unwrap();
        sched.complete(lease.idx);

        assert!(sched.all_done());
        assert_eq!(sched.segments()[0].written, 10);
    }

    #[test]
    fn released_lease_becomes_free_again_and_resumes_from_watermark() {
        let mut sched = Scheduler::new(10, 10);
        let lease = sched.lease_next("a").unwrap();
        sched.report_progress(lease.idx, 4);
        sched.release_lease(lease.idx);

        assert_eq!(sched.segments()[0].state, SegmentState::Free);
        let re_lease = sched.lease_next("b").unwrap();
        assert_eq!(re_lease.resume_from, 4, "deve riprendere dal watermark, non da zero");
    }

    #[test]
    fn record_failure_returns_segment_to_free_below_the_cap() {
        let mut sched = Scheduler::new(10, 10);
        let lease = sched.lease_next("a").unwrap();
        let state = sched.record_failure(lease.idx);
        assert_eq!(state, SegmentState::Free);
        assert_eq!(sched.segments()[0].attempts, 1);
        assert!(sched.any_failed().is_none());
    }

    #[test]
    fn record_failure_marks_segment_failed_at_the_cap() {
        let mut sched = Scheduler::new(10, 10);
        for _ in 0..MAX_SEGMENT_ATTEMPTS {
            let lease = sched.lease_next("a").unwrap();
            let state = sched.record_failure(lease.idx);
            if sched.segments()[0].attempts < MAX_SEGMENT_ATTEMPTS {
                assert_eq!(state, SegmentState::Free);
            }
        }
        assert_eq!(sched.segments()[0].state, SegmentState::Failed);
        assert!(sched.any_failed().is_some());
    }

    #[test]
    fn failed_segment_is_never_leased_or_stolen_again() {
        let mut sched = Scheduler::new(10, 10);
        let lease = sched.lease_next("a").unwrap();
        for _ in 0..MAX_SEGMENT_ATTEMPTS {
            sched.record_failure(lease.idx);
        }
        assert!(sched.lease_next("b").is_none());
        assert!(sched.try_steal_tail("b", 1).is_none());
    }

    #[test]
    fn release_lease_is_noop_on_free_or_done_segment() {
        let mut sched = Scheduler::new(10, 10);
        sched.release_lease(0); // ancora Free, mai leaseato
        assert_eq!(sched.segments()[0].state, SegmentState::Free);

        let lease = sched.lease_next("a").unwrap();
        sched.complete(lease.idx);
        sched.release_lease(lease.idx); // Done non deve tornare Free
        assert_eq!(sched.segments()[0].state, SegmentState::Done);
    }

    #[test]
    fn tail_splitting_shrinks_victim_and_creates_leased_segment_for_idle_link() {
        // Un solo segmento enorme, un solo worker occupato: coda vuota,
        // idle_link con size tipica piccola => residuo va spaccato in due.
        let mut sched = Scheduler::new(1000, 1000);
        let lease = sched.lease_next("slow-link").unwrap();
        sched.report_progress(lease.idx, 0); // nessun progresso ancora

        let steal = sched.try_steal_tail("idle-link", 100).unwrap();

        let victim = &sched.segments()[0];
        assert_eq!(victim.end, steal.lease.start - 1, "nessun buco tra i due segmenti");
        assert_eq!(victim.end, steal.victim_new_end);
        assert_eq!(steal.victim_idx, lease.idx);
        assert_eq!(steal.lease.end, 999, "il nuovo segmento arriva fino alla fine originale");
        assert_eq!(steal.lease.resume_from, steal.lease.start, "nuovo segmento parte da zero byte scritti");

        // Nessuna sovrapposizione: unione dei due copre esattamente [0,999].
        let mut covered: Vec<(u64, u64)> = sched.segments().iter().map(|s| (s.start, s.end)).collect();
        covered.sort();
        assert_eq!(covered, vec![(0, victim.end), (steal.lease.start, 999)]);
    }

    #[test]
    fn tail_splitting_accounts_for_bytes_already_written_in_victim() {
        let mut sched = Scheduler::new(1000, 1000);
        let lease = sched.lease_next("slow-link").unwrap();
        sched.report_progress(lease.idx, 400); // 600 byte residui

        let steal = sched.try_steal_tail("idle-link", 100).unwrap();

        // residuo = 600, metà = 300, split_point = 400 + 300 = 700
        assert_eq!(sched.segments()[0].end, 699);
        assert_eq!(steal.lease.start, 700);
        assert_eq!(steal.lease.end, 999);
    }

    #[test]
    fn tail_splitting_refuses_when_residual_not_worth_it() {
        let mut sched = Scheduler::new(1000, 1000);
        let lease = sched.lease_next("slow-link").unwrap();
        sched.report_progress(lease.idx, 850); // residuo 150, soglia 2*100=200

        assert!(sched.try_steal_tail("idle-link", 100).is_none());
    }

    #[test]
    fn tail_splitting_returns_none_with_no_in_flight_segments() {
        let mut sched = Scheduler::new(1000, 1000);
        assert!(sched.try_steal_tail("idle-link", 100).is_none()); // tutto Free, nulla in volo

        let lease = sched.lease_next("a").unwrap();
        sched.complete(lease.idx);
        assert!(sched.try_steal_tail("idle-link", 100).is_none()); // tutto Done
    }

    #[test]
    fn tail_splitting_picks_the_segment_with_largest_residual() {
        let mut sched = Scheduler::new(2000, 1000); // due segmenti da 1000
        let l0 = sched.lease_next("a").unwrap(); // [0,999]
        let l1 = sched.lease_next("b").unwrap(); // [1000,1999]
        sched.report_progress(l0.idx, 900); // residuo 100
        sched.report_progress(l1.idx, 100); // residuo 900

        let steal = sched.try_steal_tail("idle", 50).unwrap();
        assert!(
            steal.lease.start >= 1000,
            "deve rubare dal segmento con più byte residui (l1), non da l0"
        );
        assert_eq!(steal.victim_idx, l1.idx);
    }

    #[test]
    fn from_segments_resumes_next_idx_above_existing_max() {
        let existing = vec![Segment {
            idx: 5,
            start: 0,
            end: 99,
            written: 0,
            state: SegmentState::Free,
            lease_link: None,
            attempts: 0,
        }];
        let mut sched = Scheduler::from_segments(existing);
        let lease = sched.lease_next("a").unwrap();
        sched.report_progress(lease.idx, 0);
        let steal = sched.try_steal_tail("idle", 1).unwrap();
        assert!(steal.lease.idx > 5, "il nuovo segmento non deve collidere con idx esistenti");
    }
}
