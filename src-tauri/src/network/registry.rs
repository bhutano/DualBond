//! Registro persistente delle identità dei link (§3.2, §7 della spec):
//! nome scelto dall'utente, colore, icona, flag a consumo, abilitazione.
//! Sopravvive a scollega/ricollega, cambio porta USB, riavvio del PC —
//! tutto legato alla chiave stabile, mai all'indice di interfaccia
//! volatile. Vive in un DB separato da `storage::StateDb` (quello è stato
//! per-download, questo è configurazione app-wide con un ciclo di vita
//! indipendente).

use crate::core::errors::DualBondError;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;

const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS link_identity (
  key TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  display_name TEXT NOT NULL,
  auto_name TEXT,
  color TEXT NOT NULL,
  icon TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  metered INTEGER NOT NULL DEFAULT 0,
  first_seen INTEGER,
  last_seen INTEGER,
  total_bytes INTEGER NOT NULL DEFAULT 0
);
";

/// Palette categorica per l'assegnazione automatica del colore al primo
/// rilevamento (§7: "Il colore è preassegnato dalla palette in ordine").
/// Include l'accent cyan/viola scelto dall'utente per l'icona dell'app
/// come primi due colori, poi si allarga a tinte ben distinguibili tra
/// loro e leggibili sia in chiaro che in scuro.
pub const COLOR_PALETTE: &[&str] = &[
    "#00E8FF", "#B026FF", "#FF6B35", "#4CD964", "#FFD60A", "#FF375F", "#5E5CE6", "#64D2FF",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkIdentity {
    pub key: String,
    pub kind: String,
    pub display_name: String,
    pub auto_name: Option<String>,
    pub color: String,
    pub icon: String,
    pub enabled: bool,
    pub metered: bool,
    pub first_seen: i64,
    pub last_seen: i64,
    pub total_bytes: u64,
}

pub struct LinkRegistry {
    conn: Connection,
}

impl LinkRegistry {
    pub fn open(path: &Path) -> Result<Self, DualBondError> {
        let conn = Connection::open(path)?;
        Self::from_connection(conn)
    }

    pub fn open_in_memory() -> Result<Self, DualBondError> {
        let conn = Connection::open_in_memory()?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> Result<Self, DualBondError> {
        conn.execute_batch(SCHEMA_SQL)?;
        Ok(Self { conn })
    }

    pub fn get(&self, key: &str) -> Result<Option<LinkIdentity>, DualBondError> {
        self.conn
            .query_row(
                "SELECT key, kind, display_name, auto_name, color, icon, enabled, metered, \
                 first_seen, last_seen, total_bytes FROM link_identity WHERE key = ?1",
                params![key],
                row_to_identity,
            )
            .optional()
            .map_err(DualBondError::from)
    }

    /// Primo rilevamento o ricollegamento (§7, §4.8): se la chiave esiste
    /// già, **non tocca** nome/colore/metered/enabled scelti dall'utente —
    /// aggiorna solo `last_seen`. Se è nuova, la crea con `display_name`
    /// preimpostato ad `auto_name` (già editabile senza click aggiuntivi,
    /// §7) e il prossimo colore libero della palette in ordine.
    pub fn upsert_seen(
        &self,
        key: &str,
        kind: &str,
        auto_name: Option<&str>,
        icon: &str,
        metered_hint: bool,
        now: i64,
    ) -> Result<LinkIdentity, DualBondError> {
        if let Some(existing) = self.get(key)? {
            self.conn.execute(
                "UPDATE link_identity SET last_seen = ?1 WHERE key = ?2",
                params![now, key],
            )?;
            return Ok(LinkIdentity {
                last_seen: now,
                ..existing
            });
        }

        let color = self.next_palette_color()?;
        let display_name = auto_name.unwrap_or(key).to_string();
        // Le reti appena rilevate entrano automaticamente nel pool. Un
        // ricollegamento passa dal ramo `UPDATE` e conserva comunque
        // un'eventuale esclusione manuale dell'utente.
        self.conn.execute(
            "INSERT INTO link_identity (key, kind, display_name, auto_name, color, icon, \
             enabled, metered, first_seen, last_seen, total_bytes) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8, ?8, 0)",
            params![key, kind, display_name, auto_name, color, icon, metered_hint as i64, now],
        )?;
        Ok(LinkIdentity {
            key: key.to_string(),
            kind: kind.to_string(),
            display_name,
            auto_name: auto_name.map(str::to_string),
            color,
            icon: icon.to_string(),
            enabled: true,
            metered: metered_hint,
            first_seen: now,
            last_seen: now,
            total_bytes: 0,
        })
    }

    fn next_palette_color(&self) -> Result<String, DualBondError> {
        let count: i64 = self.conn.query_row("SELECT COUNT(*) FROM link_identity", [], |r| r.get(0))?;
        let idx = (count as usize) % COLOR_PALETTE.len();
        Ok(COLOR_PALETTE[idx].to_string())
    }

    pub fn set_display_name(&self, key: &str, name: &str) -> Result<(), DualBondError> {
        self.conn
            .execute("UPDATE link_identity SET display_name = ?1 WHERE key = ?2", params![name, key])?;
        Ok(())
    }

    pub fn set_color(&self, key: &str, color: &str) -> Result<(), DualBondError> {
        self.conn
            .execute("UPDATE link_identity SET color = ?1 WHERE key = ?2", params![color, key])?;
        Ok(())
    }

    /// `enabled = false` è un'esclusione manuale persistente: il link
    /// resta memorizzato ma non entra nello scheduler.
    pub fn set_enabled(&self, key: &str, enabled: bool) -> Result<(), DualBondError> {
        self.conn
            .execute("UPDATE link_identity SET enabled = ?1 WHERE key = ?2", params![enabled as i64, key])?;
        Ok(())
    }

    pub fn set_metered(&self, key: &str, metered: bool) -> Result<(), DualBondError> {
        self.conn
            .execute("UPDATE link_identity SET metered = ?1 WHERE key = ?2", params![metered as i64, key])?;
        Ok(())
    }

    pub fn add_bytes(&self, key: &str, bytes: u64) -> Result<(), DualBondError> {
        self.conn.execute(
            "UPDATE link_identity SET total_bytes = total_bytes + ?1 WHERE key = ?2",
            params![bytes as i64, key],
        )?;
        Ok(())
    }

    /// Pulsante "Dimentica" (§6.7): rimuove il link dallo storico. Un
    /// ricollegamento successivo lo tratta come un primo rilevamento.
    pub fn forget(&self, key: &str) -> Result<(), DualBondError> {
        self.conn.execute("DELETE FROM link_identity WHERE key = ?1", params![key])?;
        Ok(())
    }

    /// Rimuove le voci create in passato per l'adattatore virtuale del tunnel
    /// ("DualBond", tipo "other"): non è una connessione e prima della
    /// correzione veniva registrato una volta per ogni installazione.
    /// Un link fisico rinominato a mano "DualBond" non viene toccato.
    pub fn forget_own_tunnel_entries(&self) -> Result<usize, DualBondError> {
        let removed = self.conn.execute(
            "DELETE FROM link_identity WHERE kind = 'other' AND auto_name = ?1",
            params![crate::network::discovery::TUNNEL_ADAPTER_NAME],
        )?;
        Ok(removed)
    }

    pub fn list_all(&self) -> Result<Vec<LinkIdentity>, DualBondError> {
        let mut stmt = self.conn.prepare(
            "SELECT key, kind, display_name, auto_name, color, icon, enabled, metered, \
             first_seen, last_seen, total_bytes FROM link_identity ORDER BY first_seen",
        )?;
        let rows = stmt.query_map([], row_to_identity)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}

fn row_to_identity(row: &rusqlite::Row) -> rusqlite::Result<LinkIdentity> {
    Ok(LinkIdentity {
        key: row.get(0)?,
        kind: row.get(1)?,
        display_name: row.get(2)?,
        auto_name: row.get(3)?,
        color: row.get(4)?,
        icon: row.get(5)?,
        enabled: row.get::<_, i64>(6)? != 0,
        metered: row.get::<_, i64>(7)? != 0,
        first_seen: row.get(8)?,
        last_seen: row.get(9)?,
        total_bytes: row.get::<_, i64>(10)? as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forgets_only_the_own_tunnel_adapter_entries() {
        let reg = LinkRegistry::open_in_memory().unwrap();
        reg.upsert_seen("guid:{A}", "other", Some("DualBond"), "ethernet", false, 1).unwrap();
        reg.upsert_seen("guid:{B}", "other", Some("DualBond"), "ethernet", false, 2).unwrap();
        reg.upsert_seen("mac:aa:bb:cc:dd:ee:01", "wifi", Some("Wi-Fi"), "wifi", false, 3).unwrap();
        // link fisico rinominato dall'utente "DualBond": resta
        reg.upsert_seen("mac:aa:bb:cc:dd:ee:02", "ethernet", Some("Ethernet"), "ethernet", false, 4).unwrap();
        reg.set_display_name("mac:aa:bb:cc:dd:ee:02", "DualBond").unwrap();

        assert_eq!(reg.forget_own_tunnel_entries().unwrap(), 2);
        let keys: Vec<String> = reg.list_all().unwrap().into_iter().map(|id| id.key).collect();
        assert_eq!(keys, vec!["mac:aa:bb:cc:dd:ee:01".to_string(), "mac:aa:bb:cc:dd:ee:02".to_string()]);
    }

    #[test]
    fn first_seen_creates_with_auto_name_as_display_name_and_first_palette_color() {
        let reg = LinkRegistry::open_in_memory().unwrap();
        let id = reg.upsert_seen("usb:18d1:4ee7:abc", "usb", Some("moto g84 5G"), "phone", false, 100).unwrap();

        assert_eq!(id.display_name, "moto g84 5G");
        assert_eq!(id.auto_name.as_deref(), Some("moto g84 5G"));
        assert_eq!(id.color, COLOR_PALETTE[0]);
        assert!(id.enabled, "una nuova connessione deve entrare automaticamente nel pool");
        assert_eq!(id.first_seen, 100);
        assert_eq!(id.last_seen, 100);
    }

    #[test]
    fn missing_auto_name_falls_back_to_the_stable_key() {
        let reg = LinkRegistry::open_in_memory().unwrap();
        let id = reg.upsert_seen("mac:aa:bb:cc:dd:ee:ff", "ethernet", None, "ethernet", false, 1).unwrap();
        assert_eq!(id.display_name, "mac:aa:bb:cc:dd:ee:ff");
    }

    #[test]
    fn successive_links_get_successive_palette_colors() {
        let reg = LinkRegistry::open_in_memory().unwrap();
        let a = reg.upsert_seen("wifi:home", "wifi", Some("home"), "wifi", false, 1).unwrap();
        let b = reg.upsert_seen("wifi:office", "wifi", Some("office"), "wifi", false, 1).unwrap();
        assert_eq!(a.color, COLOR_PALETTE[0]);
        assert_eq!(b.color, COLOR_PALETTE[1]);
    }

    #[test]
    fn reconnecting_never_overwrites_user_chosen_name_or_color() {
        let reg = LinkRegistry::open_in_memory().unwrap();
        reg.upsert_seen("usb:1:1:s", "usb", Some("Telefono"), "phone", false, 1).unwrap();
        reg.set_display_name("usb:1:1:s", "Il mio telefono").unwrap();
        reg.set_color("usb:1:1:s", "#123456").unwrap();

        // Ricollegato più tardi, magari a un'altra porta USB (stessa
        // chiave stabile, §3.2): il nome/colore scelti restano.
        let id = reg.upsert_seen("usb:1:1:s", "usb", Some("Telefono"), "phone", false, 999).unwrap();

        assert_eq!(id.display_name, "Il mio telefono");
        assert_eq!(id.color, "#123456");
        assert_eq!(id.last_seen, 999, "last_seen va comunque aggiornato al ricollegamento");
    }

    #[test]
    fn confirmed_link_stays_enabled_across_reconnects() {
        let reg = LinkRegistry::open_in_memory().unwrap();
        reg.upsert_seen("usb:1:1:s", "usb", Some("Telefono"), "phone", false, 1).unwrap();
        reg.set_enabled("usb:1:1:s", true).unwrap();

        let id = reg.upsert_seen("usb:1:1:s", "usb", Some("Telefono"), "phone", false, 2).unwrap();
        assert!(id.enabled, "la preferenza deve sopravvivere al ricollegamento");
    }

    #[test]
    fn disabled_link_stays_disabled_across_reconnects() {
        let reg = LinkRegistry::open_in_memory().unwrap();
        reg.upsert_seen("wifi:guest", "wifi", Some("guest"), "wifi", false, 1).unwrap();
        reg.set_enabled("wifi:guest", false).unwrap();

        let id = reg.upsert_seen("wifi:guest", "wifi", Some("guest"), "wifi", false, 2).unwrap();
        assert!(!id.enabled, "l'esclusione manuale deve sopravvivere al ricollegamento");
    }

    #[test]
    fn add_bytes_accumulates_across_calls() {
        let reg = LinkRegistry::open_in_memory().unwrap();
        reg.upsert_seen("wifi:x", "wifi", Some("x"), "wifi", false, 1).unwrap();
        reg.add_bytes("wifi:x", 1000).unwrap();
        reg.add_bytes("wifi:x", 500).unwrap();
        assert_eq!(reg.get("wifi:x").unwrap().unwrap().total_bytes, 1500);
    }

    #[test]
    fn forget_removes_the_link_and_next_seen_starts_fresh() {
        let reg = LinkRegistry::open_in_memory().unwrap();
        reg.upsert_seen("wifi:x", "wifi", Some("x"), "wifi", false, 1).unwrap();
        reg.set_display_name("wifi:x", "custom").unwrap();
        reg.forget("wifi:x").unwrap();

        assert!(reg.get("wifi:x").unwrap().is_none());
        let id = reg.upsert_seen("wifi:x", "wifi", Some("x"), "wifi", false, 2).unwrap();
        assert_eq!(id.display_name, "x", "dopo Dimentica riparte come primo rilevamento");
    }

    #[test]
    fn metered_hint_is_only_a_default_and_can_be_overridden() {
        let reg = LinkRegistry::open_in_memory().unwrap();
        let id = reg.upsert_seen("wifi:hotspot", "wifi", Some("hotspot"), "wifi", true, 1).unwrap();
        assert!(id.metered, "suggerimento NetworkCostType (§3.3) preimposta il flag");

        reg.set_metered("wifi:hotspot", false).unwrap();
        assert!(!reg.get("wifi:hotspot").unwrap().unwrap().metered);
    }

    #[test]
    fn list_all_returns_every_known_link_ordered_by_first_seen() {
        let reg = LinkRegistry::open_in_memory().unwrap();
        reg.upsert_seen("b", "wifi", Some("b"), "wifi", false, 20).unwrap();
        reg.upsert_seen("a", "wifi", Some("a"), "wifi", false, 10).unwrap();

        let all = reg.list_all().unwrap();
        assert_eq!(all.iter().map(|l| l.key.as_str()).collect::<Vec<_>>(), vec!["a", "b"]);
    }
}
