// Byte, bitrate, durata in formato umano (§6.2/§6.4 della spec: cifre
// grandi al centro del gauge, righe compatte nell'elenco link). Il locale
// segue la lingua scelta nell'interfaccia ("194,3 Mbps" / "194.3 Mbps").

import { lang, t } from "./i18n.svelte";

const numberFormat = () => new Intl.NumberFormat(lang.locale, { maximumFractionDigits: 1 });
const intFormat = () => new Intl.NumberFormat(lang.locale, { maximumFractionDigits: 0 });

export function formatInt(value: number): string {
  return intFormat().format(value);
}

/** Cifra e unità del gauge: sopra 1.000 Mbps passa ai Gbps ("2,3 Gbps"). */
export function speedParts(mbps: number): { value: string; unit: string } {
  if (mbps >= 1000) {
    return { value: numberFormat().format(mbps / 1000), unit: "Gbps" };
  }
  return { value: Math.round(mbps).toLocaleString(lang.locale), unit: "Mbps" };
}

export function formatMbps(mbps: number): string {
  const { value, unit } = speedParts(mbps);
  return unit === "Gbps" ? `${value} ${unit}` : `${numberFormat().format(mbps)} Mbps`;
}

export function formatPercent(percent: number): string {
  return `${numberFormat().format(percent)} %`;
}

const BYTE_UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

export function formatBytes(bytes: number): string {
  if (bytes <= 0) return "0 B";
  let value = bytes;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < BYTE_UNITS.length - 1) {
    value /= 1024;
    unitIndex++;
  }
  const digits = unitIndex === 0 ? 0 : value < 10 ? 2 : 1;
  return `${value.toLocaleString(lang.locale, { maximumFractionDigits: digits, minimumFractionDigits: 0 })} ${BYTE_UNITS[unitIndex]}`;
}

/** "4,58 GB di 10,7 GB" — coppia usata/totale, stesso mockup §6.2. */
export function formatBytesOf(used: number, total: number | null): string {
  if (total == null) return formatBytes(used);
  return t("{a} di {b}", { a: formatBytes(used), b: formatBytes(total) });
}

export function formatRtt(rttMs: number | null): string | null {
  if (rttMs == null) return null;
  return `${intFormat().format(rttMs)} ms`;
}

/** "4 min 32 s" — mai secondi grezzi oltre il minuto, mai ore superflue
 * per stime brevi (§6.2: "restano 4 min 32 s"). */
export function formatDuration(totalSeconds: number | null): string | null {
  if (totalSeconds == null || !Number.isFinite(totalSeconds) || totalSeconds < 0) return null;
  const s = Math.round(totalSeconds);
  const hours = Math.floor(s / 3600);
  const minutes = Math.floor((s % 3600) / 60);
  const seconds = s % 60;

  if (hours > 0) return `${hours} h ${minutes} min`;
  if (minutes > 0) return `${minutes} min ${seconds} s`;
  return `${seconds} s`;
}

export function formatGainFactor(gainFactor: number): string {
  return t("{v}× guadagno", { v: numberFormat().format(gainFactor) });
}
