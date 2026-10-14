//! Log eventi in memoria (§6.5 della spec: "striscia con gli ultimi
//! eventi rilevanti"). Buffer circolare globale, capacità fissa — non è
//! il log su file completo (`tracing`/`core/logging.rs` se ne occupa
//! già), è solo la finestra recente che la UI mostra senza dover aprire
//! nulla.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventEntry {
    pub ts: i64,
    pub level: String,
    pub text: String,
}

pub struct EventLog {
    entries: Mutex<VecDeque<EventEntry>>,
    capacity: usize,
}

impl EventLog {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
        }
    }

    pub fn push(&self, level: &str, text: impl Into<String>) {
        self.push_at(now_unix_ms(), level, text);
    }

    /// Variante con timestamp esplicito, usata dai test per evitare
    /// dipendenze dall'orologio di sistema.
    pub fn push_at(&self, ts: i64, level: &str, text: impl Into<String>) {
        let mut entries = self.entries.lock().unwrap();
        if entries.len() >= self.capacity {
            entries.pop_front();
        }
        entries.push_back(EventEntry {
            ts,
            level: level.to_string(),
            text: text.into(),
        });
    }

    /// Gli ultimi `n` eventi, dal più vecchio al più recente (ordine
    /// naturale per una striscia che si legge da sinistra a destra).
    pub fn recent(&self, n: usize) -> Vec<EventEntry> {
        let entries = self.entries.lock().unwrap();
        entries.iter().rev().take(n).rev().cloned().collect()
    }
}

fn now_unix_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

static GLOBAL: OnceLock<EventLog> = OnceLock::new();

/// Log globale di processo (§ del file, capacità 200) — accessibile da
/// ovunque senza dover infilare un `Arc<EventLog>` in ogni struct che
/// potrebbe voler segnalare qualcosa (worker, orchestrator, monitor).
pub fn global() -> &'static EventLog {
    GLOBAL.get_or_init(|| EventLog::new(200))
}

pub fn push(level: &str, text: impl Into<String>) {
    global().push(level, text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_returns_entries_oldest_to_newest() {
        let log = EventLog::new(10);
        log.push_at(1, "info", "a");
        log.push_at(2, "warn", "b");
        log.push_at(3, "error", "c");

        let recent = log.recent(10);
        assert_eq!(recent.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(), vec!["a", "b", "c"]);
    }

    #[test]
    fn recent_limits_to_requested_count_keeping_the_newest() {
        let log = EventLog::new(10);
        for i in 0..5 {
            log.push_at(i, "info", format!("e{i}"));
        }
        let recent = log.recent(2);
        assert_eq!(recent.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(), vec!["e3", "e4"]);
    }

    #[test]
    fn capacity_drops_the_oldest_entry_first() {
        let log = EventLog::new(3);
        for i in 0..5 {
            log.push_at(i, "info", format!("e{i}"));
        }
        let all = log.recent(10);
        assert_eq!(all.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(), vec!["e2", "e3", "e4"]);
    }

    #[test]
    fn empty_log_returns_empty_vec() {
        let log = EventLog::new(10);
        assert!(log.recent(5).is_empty());
    }
}
