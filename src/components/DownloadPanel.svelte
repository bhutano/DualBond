<script lang="ts">
  import { lang, t } from "../lib/i18n.svelte";
  import type { DownloadStats } from "../lib/ipc";
  import { pauseDownload, resumeDownload, cancelDownload, openDownloadFolder } from "../lib/ipc";
  import { formatBytesOf, formatPercent, formatDuration, formatMbps } from "../lib/format";
  import Icon from "./Icon.svelte";

  let {
    download,
    totalMbps,
    totalAvgMbps,
    onNewDownload,
  }: {
    download: DownloadStats;
    totalMbps: number;
    totalAvgMbps: number;
    onNewDownload: () => void;
  } = $props();

  let acting = $state(false);
  let actionError = $state<string | null>(null);

  const etaSec = $derived(
    download.total != null && totalAvgMbps > 0.1
      ? ((download.total - download.downloaded) * 8) / (totalAvgMbps * 1_000_000)
      : null,
  );

  const stateLabel = (state: string) =>
    ({
      running: t("Download in corso"),
      paused: t("In pausa"),
      verifying: t("Verifica del file…"),
      done: t("Download completato"),
      error: t("Download non riuscito"),
      cancelled: t("Download annullato"),
    })[state] ?? state;
  const terminal = $derived(["done", "error", "cancelled"].includes(download.state));

  async function runAction(action: () => Promise<void>) {
    if (acting) return;
    acting = true;
    actionError = null;
    try {
      await action();
    } catch (error) {
      actionError = String(error);
    } finally {
      acting = false;
    }
  }
</script>

<div class="panel state-{download.state}">
  <div class="panel-top">
    <div class="file-identity">
      <span class="file-icon"><Icon name="download" size={18} /></span>
      <div>
        <p class="eyebrow">{stateLabel(download.state)}</p>
        <h2>{download.name}</h2>
      </div>
    </div>
    <strong class="percent">{formatPercent(download.percent)}</strong>
  </div>

  <div class="bar-track" aria-label={t("Progresso {v}", { v: formatPercent(download.percent) })}>
    <div class="bar-fill" class:paused={download.state === "paused"} style={`width: ${Math.min(100, download.percent)}%`}></div>
  </div>

  <div class="detail-row">
    <span>{formatBytesOf(download.downloaded, download.total)}</span>
    {#if download.state === "running"}
      <span><strong>{formatMbps(totalMbps)}</strong> {t("ora")}</span>
      <span><strong>{formatMbps(totalAvgMbps)}</strong> {t("media 10 s")}</span>
    {/if}
    {#if etaSec != null && download.state === "running"}
      <span>{t("Circa")} <strong>{formatDuration(etaSec)}</strong> {t("rimanenti")}</span>
    {/if}
  </div>

  {#if actionError}<p class="action-error" role="alert">{actionError}</p>{/if}

  <div class="actions">
    {#if download.state === "running"}
      <button class="button secondary" type="button" onclick={() => runAction(pauseDownload)} disabled={acting}>
        <Icon name="pause" size={15} /> {t("Pausa")}
      </button>
    {:else if download.state === "paused"}
      <button class="button primary" type="button" onclick={() => runAction(resumeDownload)} disabled={acting}>
        <Icon name="play" size={15} /> {t("Riprendi")}
      </button>
    {/if}

    {#if download.state === "running" || download.state === "paused"}
      <button class="button danger" type="button" onclick={() => runAction(cancelDownload)} disabled={acting}>
        <Icon name="close" size={15} /> {t("Annulla")}
      </button>
    {/if}

    <span class="spacer"></span>

    {#if download.state === "done"}
      <button class="button secondary" type="button" onclick={() => runAction(openDownloadFolder)}>
        <Icon name="folder" size={15} /> {t("Apri cartella")}
      </button>
    {/if}
    {#if terminal}
      <button class="button primary" type="button" onclick={onNewDownload}>
        <Icon name="plus" size={15} /> {t("Scarica un altro file")}
      </button>
    {/if}
  </div>
</div>

<style>
  .panel { padding: 1.1rem 1.25rem 1.2rem; display: grid; gap: 0.78rem; }
  .panel-top,
  .file-identity,
  .detail-row,
  .actions { display: flex; align-items: center; }
  .panel-top { justify-content: space-between; gap: 1rem; }
  .file-identity { min-width: 0; gap: 0.72rem; }
  .file-icon { width: 38px; height: 38px; flex: 0 0 auto; display: grid; place-items: center; border-radius: 10px; color: var(--accent); background: color-mix(in srgb, var(--accent) 10%, var(--surface-2)); }
  .state-done .file-icon { color: var(--success); background: color-mix(in srgb, var(--success) 10%, var(--surface-2)); }
  .state-error .file-icon,
  .state-cancelled .file-icon { color: var(--error); background: color-mix(in srgb, var(--error) 9%, var(--surface-2)); }
  .eyebrow { margin: 0 0 0.18rem; color: var(--text-subtle); font-size: 0.65rem; font-weight: 700; letter-spacing: 0.08em; text-transform: uppercase; }
  h2 { max-width: 60ch; margin: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 0.94rem; }
  .percent { font-family: var(--font-numeric); font-size: 1.05rem; font-variant-numeric: tabular-nums; }
  .bar-track { height: 7px; overflow: hidden; border-radius: 99px; background: var(--surface-3); }
  .bar-fill { height: 100%; border-radius: inherit; background: linear-gradient(90deg, var(--accent), var(--accent-2)); transition: width 300ms ease-out; box-shadow: 0 0 12px color-mix(in srgb, var(--accent) 28%, transparent); }
  .bar-fill.paused { filter: saturate(0.3); }
  .detail-row { flex-wrap: wrap; gap: 0.45rem 1.1rem; color: var(--text-muted); font-size: 0.76rem; font-variant-numeric: tabular-nums; }
  .detail-row strong { color: var(--text); font-weight: 620; }
  .actions { gap: 0.55rem; }
  .spacer { flex: 1; }
  .action-error { margin: 0; color: var(--error); font-size: 0.78rem; }
  @media (max-width: 620px) { .detail-row { display: grid; } .actions { flex-wrap: wrap; } .spacer { display: none; } .actions .primary:last-child { margin-left: auto; } }
</style>
