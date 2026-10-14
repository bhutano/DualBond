// Wrapper tipizzati su invoke/listen (§5 della spec, file tree). Unico
// punto di contatto col backend Tauri — nessun componente chiama
// `invoke`/`listen` direttamente, così il contratto IPC vive in un solo
// posto quando cambia.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type LinkKind = "usb" | "wifi" | "ethernet" | "other";
export type LinkState = "online" | "probing" | "degraded" | "offline" | "not_validated" | "disabled";

export type LinkStats = {
  key: string;
  name: string;
  kind: LinkKind;
  color: string;
  state: LinkState;
  enabled: boolean;
  metered: boolean;
  mbps: number;
  avgMbps: number;
  peakMbps: number;
  /** traffico reale della scheda, misurato sempre (anche a tunnel fermo) */
  liveMbps: number;
  liveAvgMbps: number;
  bytes: number;
  sessionBytes: number;
  rttMs: number | null;
  connections: number;
  errors: number;
  signalPercent?: number;
};

export type DownloadRunState = "running" | "paused" | "verifying" | "done" | "error" | "cancelled";

export type DownloadStats = {
  name: string;
  downloaded: number;
  total: number | null;
  percent: number;
  etaSec: number | null;
  segmentsDone: number;
  segmentsTotal: number;
  retries: number;
  state: DownloadRunState;
};

export type RecentEvent = { ts: number; level: string; text: string };

export type StatsSnapshot = {
  ts: number;
  links: LinkStats[];
  totalMbps: number;
  totalAvgMbps: number;
  gainFactor: number;
  scaleMax: number;
  download?: DownloadStats;
  recentEvents: RecentEvent[];
};

export type LinkIdentity = {
  key: string;
  kind: string;
  displayName: string;
  autoName: string | null;
  color: string;
  icon: string;
  enabled: boolean;
  metered: boolean;
  firstSeen: number;
  lastSeen: number;
  totalBytes: number;
};

export type ProbeResult = { family: string; outcome: string; boundCorrectly: boolean };
export type LinkReport = {
  ifIndex: number;
  name: string;
  kind: string;
  stableKey: string;
  ips: string[];
  internetAccess: "Internet" | "LocalOnly" | "Unknown";
  probes: ProbeResult[];
};
export type DoctorReport = { links: LinkReport[]; warnings: string[] };

export type SystemLinkStatus = {
  key: string;
  name: string;
  kind: string;
  ifIndex: number;
  localIp: string;
  enabled: boolean;
  metered: boolean;
  state: "online" | "probing" | "local_only" | "degraded" | "offline" | "disabled";
  mbps: number;
  avgMbps: number;
  bytesIn: number;
  bytesOut: number;
  tcpFlows: number;
  udpFlows: number;
  errors: number;
};

export type SystemBalanceStatus = {
  serviceAvailable: boolean;
  enabled: boolean;
  state: "unavailable" | "idle" | "starting" | "waiting" | "active" | "error" | "stopped";
  adapterName: string;
  ipv4Only: boolean;
  totalMbps: number;
  totalAvgMbps: number;
  tcpFlows: number;
  udpFlows: number;
  links: SystemLinkStatus[];
  lastError: string | null;
  updatedAt: number;
};

/** Sottoscrizione unica allo `stats` event (§5: "il frontend si iscrive
 * una volta", mai polling con invoke in loop). */
export function onStats(callback: (snapshot: StatsSnapshot) => void): Promise<UnlistenFn> {
  return listen<StatsSnapshot>("stats", (event) => callback(event.payload));
}

export function doctorReport(): Promise<DoctorReport> {
  return invoke("doctor_report");
}

export function getSystemBalanceStatus(): Promise<SystemBalanceStatus> {
  return invoke("get_system_balance_status");
}

export function setSystemBalanceEnabled(enabled: boolean): Promise<SystemBalanceStatus> {
  return invoke("set_system_balance_enabled", { enabled });
}

export function refreshSystemLinks(): Promise<SystemBalanceStatus> {
  return invoke("refresh_system_links");
}

export function setSystemLinkEnabled(key: string, enabled: boolean): Promise<SystemBalanceStatus> {
  return invoke("set_system_link_enabled", { key, enabled });
}

export function setSystemLinkMetered(key: string, metered: boolean): Promise<SystemBalanceStatus> {
  return invoke("set_system_link_metered", { key, metered });
}

export function startDownload(url: string): Promise<void> {
  return invoke("start_download", { url });
}

export function pauseDownload(): Promise<void> {
  return invoke("pause_download");
}

export function resumeDownload(): Promise<void> {
  return invoke("resume_download");
}

export function cancelDownload(): Promise<void> {
  return invoke("cancel_download");
}

export function listLinks(): Promise<LinkIdentity[]> {
  return invoke("list_links");
}

export function renameLink(key: string, name: string): Promise<void> {
  return invoke("rename_link", { key, name });
}

export function setLinkColor(key: string, color: string): Promise<void> {
  return invoke("set_link_color", { key, color });
}

export function setLinkEnabled(key: string, enabled: boolean): Promise<void> {
  return invoke("set_link_enabled", { key, enabled });
}

export function setLinkMetered(key: string, metered: boolean): Promise<void> {
  return invoke("set_link_metered", { key, metered });
}

export function forgetLink(key: string): Promise<void> {
  return invoke("forget_link", { key });
}

export function openDownloadFolder(): Promise<void> {
  return invoke("open_download_folder");
}

// §9 della spec — %APPDATA%\DualBond\config.toml, snake_case sul lato
// Rust (deriva da toml/serde senza rename_all, a differenza degli altri
// tipi IPC): qui i nomi restano identici, nessuna conversione camelCase.
export type Settings = {
  download_directory: string;
  max_connections: string;
  segment_size: string;
  checksum_algorithm: string;
  retry_count: number;
  retry_backoff_ms: number;
  scheduler_interval_ms: number;
  prevent_sleep: boolean;
  auto_resume_on_start: boolean;
  ask_before_metered: boolean;
  theme: string;
  chart_window_sec: number;
};

export function getSettings(): Promise<Settings> {
  return invoke("get_settings");
}

export function saveSettings(settings: Settings): Promise<void> {
  return invoke("save_settings", { settings });
}
