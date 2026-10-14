use crate::core::errors::DualBondError;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

/// Schema minimo per Milestone 3: `download` + `chunk` (§4.5 della spec).
/// `link_stat`/`event` arrivano con multi-link (Milestone 4) e la UI
/// (Milestone 6), quando ci sarà qualcosa che li popola davvero.
const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS download (
  id INTEGER PRIMARY KEY,
  url TEXT NOT NULL,
  final_url TEXT,
  dest_path TEXT NOT NULL,
  total_size INTEGER,
  etag TEXT,
  last_modified TEXT,
  status TEXT NOT NULL,
  expected_sha256 TEXT,
  actual_sha256 TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS chunk (
  download_id INTEGER NOT NULL,
  idx INTEGER NOT NULL,
  start INTEGER NOT NULL,
  end INTEGER NOT NULL,
  written INTEGER NOT NULL DEFAULT 0,
  state TEXT NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (download_id, idx)
);
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadRow {
    pub id: i64,
    pub url: String,
    pub final_url: Option<String>,
    pub dest_path: String,
    pub total_size: Option<u64>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub status: String,
    pub expected_sha256: Option<String>,
    pub actual_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkRow {
    pub download_id: i64,
    pub idx: i64,
    pub start: u64,
    pub end: u64,
    pub written: u64,
    pub state: String,
}

pub struct StateDb {
    conn: Connection,
}

impl StateDb {
    pub fn open(path: &Path) -> Result<Self, DualBondError> {
        let conn = Connection::open(path)?;
        Self::from_connection(conn)
    }

    /// Usata nei test: stessa logica di `open` ma senza toccare il
    /// filesystem, per test rapidi che non riguardano la durabilità reale.
    pub fn open_in_memory() -> Result<Self, DualBondError> {
        let conn = Connection::open_in_memory()?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> Result<Self, DualBondError> {
        // WAL non è applicabile in modalità :memory: (torna a `memory`),
        // rusqlite non fallisce ma il valore effettivo differisce: nessun
        // problema, non è quello che i test verificano.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.execute_batch(SCHEMA_SQL)?;
        Ok(Self { conn })
    }

    /// Crea una nuova riga `download` in stato `running` con un unico
    /// chunk `free` che copre l'intero file (§4.5; lo scheduler multi-chunk
    /// con lease arriva in Milestone 4).
    pub fn create_download(
        &self,
        url: &str,
        final_url: &str,
        dest_path: &str,
        total_size: u64,
        etag: Option<&str>,
        last_modified: Option<&str>,
        expected_sha256: Option<&str>,
    ) -> Result<i64, DualBondError> {
        let now = now_unix();
        self.conn.execute(
            "INSERT INTO download (url, final_url, dest_path, total_size, etag, last_modified, \
             status, expected_sha256, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'running', ?7, ?8, ?8)",
            params![url, final_url, dest_path, total_size as i64, etag, last_modified, expected_sha256, now],
        )?;
        let download_id = self.conn.last_insert_rowid();
        self.conn.execute(
            "INSERT INTO chunk (download_id, idx, start, end, written, state) \
             VALUES (?1, 0, 0, ?2, 0, 'free')",
            params![download_id, (total_size as i64) - 1],
        )?;
        Ok(download_id)
    }

    /// Come `create_download`, ma con N chunk `free` espliciti invece di
    /// uno solo che copre l'intero file — usata dal downloader multi-link
    /// (Milestone 4), dove i confini dei segmenti sono decisi da
    /// `download::scheduler::Scheduler::new` prima ancora di scrivere su DB.
    pub fn create_download_with_segments(
        &self,
        url: &str,
        final_url: &str,
        dest_path: &str,
        total_size: u64,
        etag: Option<&str>,
        last_modified: Option<&str>,
        expected_sha256: Option<&str>,
        segments: &[(u64, u64)],
    ) -> Result<i64, DualBondError> {
        let now = now_unix();
        self.conn.execute(
            "INSERT INTO download (url, final_url, dest_path, total_size, etag, last_modified, \
             status, expected_sha256, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'running', ?7, ?8, ?8)",
            params![url, final_url, dest_path, total_size as i64, etag, last_modified, expected_sha256, now],
        )?;
        let download_id = self.conn.last_insert_rowid();
        for (idx, (start, end)) in segments.iter().enumerate() {
            self.conn.execute(
                "INSERT INTO chunk (download_id, idx, start, end, written, state) \
                 VALUES (?1, ?2, ?3, ?4, 0, 'free')",
                params![download_id, idx as i64, *start as i64, *end as i64],
            )?;
        }
        Ok(download_id)
    }

    pub fn list_chunks(&self, download_id: i64) -> Result<Vec<ChunkRow>, DualBondError> {
        let mut stmt = self.conn.prepare(
            "SELECT download_id, idx, start, end, written, state FROM chunk \
             WHERE download_id = ?1 ORDER BY idx",
        )?;
        let rows = stmt
            .query_map(params![download_id], row_to_chunk)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Nuovo segmento creato da tail-splitting (§4.4): la seconda metà del
    /// residuo rubato al link lento, assegnata subito al link idle.
    pub fn insert_chunk(
        &self,
        download_id: i64,
        idx: i64,
        start: u64,
        end: u64,
        written: u64,
        state: &str,
    ) -> Result<(), DualBondError> {
        self.conn.execute(
            "INSERT INTO chunk (download_id, idx, start, end, written, state) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![download_id, idx, start as i64, end as i64, written as i64, state],
        )?;
        Ok(())
    }

    /// Accorcia il segmento vittima dopo un tail-split (§4.4): il worker
    /// lento continua sullo stesso `idx`, ma fino al nuovo `end`.
    pub fn update_chunk_end(&self, download_id: i64, idx: i64, new_end: u64) -> Result<(), DualBondError> {
        self.conn.execute(
            "UPDATE chunk SET end = ?1 WHERE download_id = ?2 AND idx = ?3",
            params![new_end as i64, download_id, idx],
        )?;
        Ok(())
    }

    pub fn set_chunk_state(&self, download_id: i64, idx: i64, state: &str) -> Result<(), DualBondError> {
        self.conn.execute(
            "UPDATE chunk SET state = ?1 WHERE download_id = ?2 AND idx = ?3",
            params![state, download_id, idx],
        )?;
        Ok(())
    }

    /// Cerca un download non completato per (url, dest_path): è la base
    /// del resume (§4.5 "Ripresa"). Il chiamante deve poi rivalidare
    /// `ETag`/`Last-Modified`/`Content-Length` prima di fidarsi di
    /// `written`.
    pub fn find_resumable(&self, url: &str, dest_path: &str) -> Result<Option<DownloadRow>, DualBondError> {
        self.conn
            .query_row(
                "SELECT id, url, final_url, dest_path, total_size, etag, last_modified, status, \
                 expected_sha256, actual_sha256 FROM download \
                 WHERE url = ?1 AND dest_path = ?2 AND status != 'done' \
                 ORDER BY id DESC LIMIT 1",
                params![url, dest_path],
                row_to_download,
            )
            .optional()
            .map_err(DualBondError::from)
    }

    pub fn get_download(&self, id: i64) -> Result<Option<DownloadRow>, DualBondError> {
        self.conn
            .query_row(
                "SELECT id, url, final_url, dest_path, total_size, etag, last_modified, status, \
                 expected_sha256, actual_sha256 FROM download WHERE id = ?1",
                params![id],
                row_to_download,
            )
            .optional()
            .map_err(DualBondError::from)
    }

    pub fn get_chunk(&self, download_id: i64, idx: i64) -> Result<Option<ChunkRow>, DualBondError> {
        self.conn
            .query_row(
                "SELECT download_id, idx, start, end, written, state FROM chunk \
                 WHERE download_id = ?1 AND idx = ?2",
                params![download_id, idx],
                row_to_chunk,
            )
            .optional()
            .map_err(DualBondError::from)
    }

    /// Barriera di durabilità (§4.5): va chiamata **dopo** `sync_data()`
    /// sul file, mai prima. `written` nel DB deve sempre essere ≤ ai byte
    /// realmente durevoli, mai il contrario.
    pub fn update_chunk_written(&self, download_id: i64, idx: i64, written: u64) -> Result<(), DualBondError> {
        self.conn.execute(
            "UPDATE chunk SET written = ?1 WHERE download_id = ?2 AND idx = ?3",
            params![written as i64, download_id, idx],
        )?;
        Ok(())
    }

    pub fn mark_chunk_done(&self, download_id: i64, idx: i64) -> Result<(), DualBondError> {
        self.conn.execute(
            "UPDATE chunk SET state = 'done' WHERE download_id = ?1 AND idx = ?2",
            params![download_id, idx],
        )?;
        Ok(())
    }

    pub fn mark_download_status(&self, id: i64, status: &str) -> Result<(), DualBondError> {
        self.conn.execute(
            "UPDATE download SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status, now_unix(), id],
        )?;
        Ok(())
    }

    pub fn set_actual_sha256(&self, id: i64, sha256: &str) -> Result<(), DualBondError> {
        self.conn.execute(
            "UPDATE download SET actual_sha256 = ?1, updated_at = ?2 WHERE id = ?3",
            params![sha256, now_unix(), id],
        )?;
        Ok(())
    }

    /// Ripulisce un download stale (metadati non rivalidati, §4.5 punto 2)
    /// prima di ricominciare da zero: mai fondere byte vecchi e nuovi.
    pub fn discard_download(&self, id: i64) -> Result<(), DualBondError> {
        self.conn.execute("DELETE FROM chunk WHERE download_id = ?1", params![id])?;
        self.conn.execute("DELETE FROM download WHERE id = ?1", params![id])?;
        Ok(())
    }
}

fn row_to_download(row: &rusqlite::Row) -> rusqlite::Result<DownloadRow> {
    Ok(DownloadRow {
        id: row.get(0)?,
        url: row.get(1)?,
        final_url: row.get(2)?,
        dest_path: row.get(3)?,
        total_size: row.get::<_, Option<i64>>(4)?.map(|v| v as u64),
        etag: row.get(5)?,
        last_modified: row.get(6)?,
        status: row.get(7)?,
        expected_sha256: row.get(8)?,
        actual_sha256: row.get(9)?,
    })
}

fn row_to_chunk(row: &rusqlite::Row) -> rusqlite::Result<ChunkRow> {
    Ok(ChunkRow {
        download_id: row.get(0)?,
        idx: row.get(1)?,
        start: row.get::<_, i64>(2)? as u64,
        end: row.get::<_, i64>(3)? as u64,
        written: row.get::<_, i64>(4)? as u64,
        state: row.get(5)?,
    })
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_download_also_creates_a_single_covering_chunk() {
        let db = StateDb::open_in_memory().unwrap();
        let id = db
            .create_download("http://x/f", "http://x/f", "C:/f.bin", 1000, Some("\"etag\""), None, None)
            .unwrap();

        let chunk = db.get_chunk(id, 0).unwrap().unwrap();
        assert_eq!(chunk.start, 0);
        assert_eq!(chunk.end, 999);
        assert_eq!(chunk.written, 0);
        assert_eq!(chunk.state, "free");
    }

    #[test]
    fn find_resumable_ignores_done_downloads() {
        let db = StateDb::open_in_memory().unwrap();
        let id = db
            .create_download("http://x/f", "http://x/f", "C:/f.bin", 1000, None, None, None)
            .unwrap();
        db.mark_download_status(id, "done").unwrap();

        assert!(db.find_resumable("http://x/f", "C:/f.bin").unwrap().is_none());
    }

    #[test]
    fn find_resumable_returns_running_download() {
        let db = StateDb::open_in_memory().unwrap();
        let id = db
            .create_download("http://x/f", "http://x/f", "C:/f.bin", 1000, Some("\"e1\""), None, None)
            .unwrap();

        let found = db.find_resumable("http://x/f", "C:/f.bin").unwrap().unwrap();
        assert_eq!(found.id, id);
        assert_eq!(found.etag.as_deref(), Some("\"e1\""));
    }

    #[test]
    fn update_chunk_written_persists() {
        let db = StateDb::open_in_memory().unwrap();
        let id = db
            .create_download("http://x/f", "http://x/f", "C:/f.bin", 1000, None, None, None)
            .unwrap();

        db.update_chunk_written(id, 0, 512).unwrap();

        assert_eq!(db.get_chunk(id, 0).unwrap().unwrap().written, 512);
    }

    #[test]
    fn create_download_with_segments_inserts_one_free_chunk_per_segment() {
        let db = StateDb::open_in_memory().unwrap();
        let id = db
            .create_download_with_segments(
                "http://x/f",
                "http://x/f",
                "C:/f.bin",
                25,
                Some("\"etag\""),
                None,
                None,
                &[(0, 9), (10, 19), (20, 24)],
            )
            .unwrap();

        let chunks = db.list_chunks(id).unwrap();
        assert_eq!(chunks.len(), 3);
        assert_eq!((chunks[0].start, chunks[0].end), (0, 9));
        assert_eq!((chunks[1].start, chunks[1].end), (10, 19));
        assert_eq!((chunks[2].start, chunks[2].end), (20, 24));
        assert!(chunks.iter().all(|c| c.state == "free" && c.written == 0));
    }

    #[test]
    fn insert_chunk_adds_a_new_row_for_tail_splitting() {
        let db = StateDb::open_in_memory().unwrap();
        let id = db
            .create_download_with_segments("http://x/f", "http://x/f", "C:/f.bin", 1000, None, None, None, &[(0, 999)])
            .unwrap();

        db.insert_chunk(id, 1, 700, 999, 0, "leased").unwrap();

        let chunks = db.list_chunks(id).unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[1].idx, 1);
        assert_eq!((chunks[1].start, chunks[1].end), (700, 999));
        assert_eq!(chunks[1].state, "leased");
    }

    #[test]
    fn update_chunk_end_shrinks_the_victim_segment_after_a_steal() {
        let db = StateDb::open_in_memory().unwrap();
        let id = db
            .create_download_with_segments("http://x/f", "http://x/f", "C:/f.bin", 1000, None, None, None, &[(0, 999)])
            .unwrap();

        db.update_chunk_end(id, 0, 699).unwrap();

        assert_eq!(db.get_chunk(id, 0).unwrap().unwrap().end, 699);
    }

    #[test]
    fn set_chunk_state_updates_state_column() {
        let db = StateDb::open_in_memory().unwrap();
        let id = db
            .create_download_with_segments("http://x/f", "http://x/f", "C:/f.bin", 1000, None, None, None, &[(0, 999)])
            .unwrap();

        db.set_chunk_state(id, 0, "leased").unwrap();

        assert_eq!(db.get_chunk(id, 0).unwrap().unwrap().state, "leased");
    }

    #[test]
    fn discard_download_removes_download_and_chunks() {
        let db = StateDb::open_in_memory().unwrap();
        let id = db
            .create_download("http://x/f", "http://x/f", "C:/f.bin", 1000, None, None, None)
            .unwrap();

        db.discard_download(id).unwrap();

        assert!(db.get_download(id).unwrap().is_none());
        assert!(db.get_chunk(id, 0).unwrap().is_none());
    }
}
