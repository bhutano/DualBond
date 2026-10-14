//! Tipi del contratto IPC (§5 "Contratto IPC" della spec) — devono
//! combaciare esattamente con i tipi TypeScript lato frontend
//! (`src/lib/ipc.ts`). Il core emette un evento `stats` 4 volte al
//! secondo con uno snapshot completo; il frontend si iscrive una volta,
//! mai polling.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkStats {
    pub key: String,
    pub name: String,
    pub kind: String, // "usb" | "wifi" | "ethernet" | "other"
    pub color: String,
    pub state: String, // "online" | "probing" | "degraded" | "offline" | "not_validated" | "disabled"
    pub enabled: bool,
    pub metered: bool,
    pub mbps: f64,
    pub avg_mbps: f64,
    pub peak_mbps: f64,
    /// Traffico reale dell'interfaccia (ricevuto + inviato), misurato sempre:
    /// anche senza download dell'acceleratore, a tunnel fermo o con il link
    /// escluso dal bilanciamento. È quello che mostra la schermata principale.
    pub live_mbps: f64,
    pub live_avg_mbps: f64,
    pub bytes: u64,
    pub session_bytes: u64,
    pub rtt_ms: Option<f64>,
    pub connections: u32,
    pub errors: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal_percent: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadStats {
    pub name: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub percent: f64,
    pub eta_sec: Option<f64>,
    pub segments_done: u32,
    pub segments_total: u32,
    pub retries: u32,
    pub state: String, // "running" | "paused" | "verifying" | "done" | "error"
}

#[derive(Debug, Clone, Serialize)]
pub struct RecentEvent {
    pub ts: i64,
    pub level: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsSnapshot {
    pub ts: i64,
    pub links: Vec<LinkStats>,
    pub total_mbps: f64,
    pub total_avg_mbps: f64,
    pub gain_factor: f64,
    pub scale_max: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download: Option<DownloadStats>,
    pub recent_events: Vec<RecentEvent>,
}
