<script lang="ts">
  import { lang, t } from "./lib/i18n.svelte";
  import { onMount } from "svelte";
  import "./app.css";
  import { monitor, systemBalance } from "./lib/stores.svelte";
  import { doctorReport } from "./lib/ipc";
  import { formatMbps } from "./lib/format";
  import "./lib/theme.svelte";
  import "./lib/accent.svelte";
  import Gauge from "./components/Gauge.svelte";
  import LinkChip from "./components/LinkChip.svelte";
  import LinkCard from "./components/LinkCard.svelte";
  import BandwidthChart from "./components/BandwidthChart.svelte";
  import DownloadPanel from "./components/DownloadPanel.svelte";
  import UrlBar from "./components/UrlBar.svelte";
  import Settings from "./components/Settings.svelte";
  import SpeedTest from "./components/SpeedTest.svelte";
  import Icon from "./components/Icon.svelte";
  import SystemBalancePanel from "./components/SystemBalancePanel.svelte";

  onMount(() => {
    monitor.start();
    systemBalance.start();
    return () => systemBalance.stop();
  });

  let selectedLinkKey = $state<string | null>(null);
  let settingsOpen = $state(false);
  let speedTestOpen = $state(false);
  let showComposer = $state(false);
  let acceleratorOpen = $state(false);
  let scanning = $state(false);
  let scanMessage = $state<string | null>(null);
  let scanMessageTimer: ReturnType<typeof setTimeout> | null = null;

  const snapshot = $derived(monitor.snapshot);
  const links = $derived(snapshot?.links ?? []);
  const liveLinks = $derived(links.filter((link) => link.state !== "offline"));
  const onlineLinks = $derived(liveLinks.filter((link) => link.enabled && link.state !== "disabled"));
  const offlineLinks = $derived(links.filter((link) => link.state === "offline"));
  const selectedLink = $derived(links.find((link) => link.key === selectedLinkKey) ?? null);
  const download = $derived(snapshot?.download ?? null);
  const terminalDownload = $derived(
    download != null && ["done", "error", "cancelled"].includes(download.state),
  );

  $effect(() => {
    if (!download) showComposer = false;
    if (download && ["running", "paused", "verifying"].includes(download.state)) showComposer = false;
  });

  async function scanNow() {
    if (scanning) return;
    scanning = true;
    scanMessage = null;
    try {
      const report = await doctorReport();
      const count = report.links.length;
      scanMessage = count === 1 ? t("1 connessione verificata") : t("{n} connessioni verificate", { n: count });
    } catch {
      scanMessage = t("Scansione non riuscita");
    } finally {
      scanning = false;
      if (scanMessageTimer) clearTimeout(scanMessageTimer);
      scanMessageTimer = setTimeout(() => (scanMessage = null), 4500);
    }
  }
</script>

<main class="shell">
  <header class="topbar">
    <div class="brand-lockup">
      <span class="brand-mark"><Icon name="activity" size={20} /></span>
      <div>
        <span class="brand">DualBond</span>
        <span class="brand-subtitle">{t("Internet multi-link per Windows")}</span>
      </div>
    </div>

    <div class="top-actions">
      <span class="monitor-state" class:ready={monitor.connected}>
        <span class="status-dot"></span>
        {monitor.connected ? t("Monitor attivo") : t("Connessione al motore…")}
      </span>
      <div class="lang-switch" role="group" aria-label={t("Lingua")}>
        <Icon name="globe" size={14} />
        {#each ["it", "en"] as code (code)}
          <button
            type="button"
            class:active={lang.current === code}
            aria-pressed={lang.current === code}
            title={code === "it" ? t("Italiano") : t("English")}
            onclick={() => lang.set(code as "it" | "en")}
          >{code.toUpperCase()}</button>
        {/each}
      </div>
      <button class="button quiet" type="button" onclick={() => (speedTestOpen = true)}>
        <Icon name="activity" size={16} /> {t("Diagnostica")}
      </button>
      <button class="icon-button" type="button" onclick={() => (settingsOpen = true)} aria-label={t("Impostazioni")}>
        <Icon name="settings" size={18} />
      </button>
    </div>
  </header>

  <div class="workspace">
    <SystemBalancePanel />

    <button class="accelerator" type="button" onclick={() => (acceleratorOpen = true)} aria-haspopup="dialog">
      <span class="accelerator-icon"><Icon name="download" size={17} /></span>
      <span><strong>{t("Acceleratore per singoli file")}</strong><small>{t("Somma i link quando il server consente di dividere il download.")}</small></span>
      <span class="accelerator-action">{t("Apri")}</span>
    </button>
  </div>
</main>

{#if acceleratorOpen}
  <div class="sheet-overlay" onclick={() => (acceleratorOpen = false)} role="presentation">
    <div
      class="sheet"
      role="dialog"
      aria-modal="true"
      aria-labelledby="accelerator-title"
      tabindex="-1"
      onclick={(event) => event.stopPropagation()}
      onkeydown={(event) => event.stopPropagation()}
    >
      <header class="sheet-head">
        <span class="accelerator-icon"><Icon name="download" size={17} /></span>
        <div><strong id="accelerator-title">{t("Acceleratore per singoli file")}</strong><small>{t("Somma i link quando il server consente di dividere il download.")}</small></div>
        <button class="icon-button" type="button" onclick={() => (acceleratorOpen = false)} aria-label={t("Chiudi")}><Icon name="close" size={18} /></button>
      </header>
      <div class="accelerator-body">
    <section class="hero-grid" aria-label={t("Riepilogo velocità")}>
      <article class="card gauge-card">
        <div class="section-heading">
          <div>
            <p class="eyebrow">{t("Traffico combinato")}</p>
            <h1>{t("Velocità in tempo reale")}</h1>
          </div>
          <span class="live-pill"><span></span> {t("Live")}</span>
        </div>
        <Gauge
          links={liveLinks}
          totalMbps={snapshot?.totalMbps ?? 0}
          totalAvgMbps={snapshot?.totalAvgMbps ?? 0}
          scaleMax={snapshot?.scaleMax ?? 10}
          gainFactor={snapshot?.gainFactor ?? 1}
        />
        <div class="metric-strip">
          <div class="metric">
            <span class="metric-label">{t("Media 10 s")}</span>
            <strong>{formatMbps(snapshot?.totalAvgMbps ?? 0)}</strong>
          </div>
          <div class="metric">
            <span class="metric-label">{t("Connessioni vive")}</span>
            <strong>{onlineLinks.length}</strong>
          </div>
          <div class="metric">
            <span class="metric-label">{t("Aggregazione")}</span>
            <strong>{onlineLinks.length > 1 ? `${(snapshot?.gainFactor ?? 1).toLocaleString(lang.locale, { maximumFractionDigits: 1 })}×` : t("Singola")}</strong>
          </div>
        </div>
      </article>

      <article class="card connections-card">
        <div class="section-heading connection-heading">
          <div>
            <p class="eyebrow">{t("Percorsi Internet")}</p>
            <h2>{t("Connessioni vive")}</h2>
          </div>
          <button class="button quiet scan-button" type="button" onclick={scanNow} disabled={scanning}>
            <span class:spinning={scanning}><Icon name="refresh" size={16} /></span>
            {scanning ? t("Verifica…") : t("Rileva ora")}
          </button>
        </div>

        {#if scanMessage}
          <div class="scan-result" role="status"><Icon name="check" size={15} /> {scanMessage}</div>
        {/if}

        <div class="connection-list">
          {#if !monitor.connected}
            <div class="empty-state">
              <span class="empty-icon"><Icon name="wifi" size={24} /></span>
              <strong>{t("Avvio del monitor")}</strong>
              <p>{t("Sto leggendo le interfacce di rete di Windows.")}</p>
            </div>
          {:else if liveLinks.length === 0}
            <div class="empty-state">
              <span class="empty-icon"><Icon name="wifi" size={24} /></span>
              <strong>{t("Nessuna connessione viva")}</strong>
              <p>{t("Collega Wi-Fi, Ethernet o tethering USB: apparirà automaticamente.")}</p>
              <button class="button secondary" type="button" onclick={scanNow}>{t("Rileva di nuovo")}</button>
            </div>
          {:else}
            {#each liveLinks as link (link.key)}
              <LinkChip {link} onSelect={(key) => (selectedLinkKey = key)} />
            {/each}
          {/if}
        </div>

        {#if offlineLinks.length > 0}
          <details class="remembered">
            <summary>{offlineLinks.length} {offlineLinks.length === 1 ? t("connessione memorizzata") : t("connessioni memorizzate")}</summary>
            <div class="remembered-list">
              {#each offlineLinks as link (link.key)}
                <LinkChip {link} onSelect={(key) => (selectedLinkKey = key)} />
              {/each}
            </div>
          </details>
        {/if}
      </article>
    </section>

    <section class="card chart-card">
      <div class="section-heading compact">
        <div>
          <p class="eyebrow">{t("Andamento")}</p>
          <h2>{t("Velocità combinata")}</h2>
        </div>
        <span class="chart-caption">{t("Valori smussati, aggiornamento 4 Hz")}</span>
      </div>
      <BandwidthChart links={liveLinks} totalMbps={snapshot?.totalMbps ?? 0} ts={snapshot?.ts ?? 0} />
    </section>

    <section class="card task-card" aria-label={t("Download")}>
      {#if download && !showComposer}
        <DownloadPanel
          {download}
          totalMbps={snapshot?.totalMbps ?? 0}
          totalAvgMbps={snapshot?.totalAvgMbps ?? 0}
          onNewDownload={() => (showComposer = true)}
        />
      {:else}
        <UrlBar onStarted={() => (showComposer = false)} />
        {#if terminalDownload}
          <button class="back-to-result" type="button" onclick={() => (showComposer = false)}>{t("Torna all’ultimo risultato")}</button>
        {/if}
      {/if}
    </section>
      </div>
    </div>
  </div>
{/if}

<svelte:window onkeydown={(event) => { if (event.key === "Escape" && acceleratorOpen && !settingsOpen && !speedTestOpen && !selectedLink) acceleratorOpen = false; }} />

{#if selectedLink}
  {#key selectedLink.key}
    <LinkCard link={selectedLink} onClose={() => (selectedLinkKey = null)} />
  {/key}
{/if}

{#if settingsOpen}
  <Settings onClose={() => (settingsOpen = false)} />
{/if}

{#if speedTestOpen}
  <SpeedTest onClose={() => (speedTestOpen = false)} />
{/if}

<style>
  .shell {
    width: min(1180px, 100%);
    height: 100vh;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .topbar {
    flex: none;
    min-height: 58px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding: 0 1.5rem;
    border-bottom: 1px solid var(--border);
    background: color-mix(in srgb, var(--bg) 86%, transparent);
    backdrop-filter: blur(18px);
    position: relative;
    z-index: 20;
  }

  .brand-lockup,
  .top-actions,
  .section-heading,
  .monitor-state,
  .scan-result {
    display: flex;
    align-items: center;
  }

  .brand-lockup { gap: 0.75rem; }
  .brand-mark {
    width: 36px;
    height: 36px;
    display: grid;
    place-items: center;
    color: var(--accent);
    border: 1px solid color-mix(in srgb, var(--accent) 35%, transparent);
    border-radius: 10px;
    background: color-mix(in srgb, var(--accent) 9%, var(--surface-1));
    box-shadow: inset 0 0 18px color-mix(in srgb, var(--accent) 8%, transparent);
  }
  .brand { display: block; font-size: 1.02rem; font-weight: 720; letter-spacing: -0.02em; }
  .brand-subtitle { display: block; margin-top: 0.06rem; color: var(--text-subtle); font-size: 0.72rem; }
  .top-actions { gap: 0.55rem; }
  .monitor-state { gap: 0.42rem; margin-right: 0.25rem; color: var(--text-muted); font-size: 0.78rem; }
  .status-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--warning); box-shadow: 0 0 0 4px color-mix(in srgb, var(--warning) 12%, transparent); }
  .monitor-state.ready .status-dot { background: var(--success); box-shadow: 0 0 0 4px color-mix(in srgb, var(--success) 12%, transparent); }

  .lang-switch { display: inline-flex; align-items: center; gap: 0.15rem; padding: 0.18rem 0.3rem 0.18rem 0.5rem; color: var(--text-subtle); border: 1px solid var(--border); border-radius: 999px; background: var(--surface-1); }
  .lang-switch button { border: 0; padding: 0.22rem 0.5rem; border-radius: 999px; background: transparent; color: var(--text-muted); font-size: 0.72rem; font-weight: 700; letter-spacing: 0.04em; cursor: pointer; }
  .lang-switch button.active { color: var(--bg); background: var(--accent); }

  .workspace { flex: 1 1 auto; min-height: 0; padding: 0.9rem 1.5rem 1rem; display: flex; flex-direction: column; gap: 0.75rem; overflow: auto; }
  .workspace > :global(.system-panel) { flex: 1 1 auto; }
  .accelerator {
    flex: none;
    width: 100%;
    color: inherit;
    font: inherit;
    text-align: left;
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: 0.8rem;
    min-height: 58px;
    padding: 0.6rem 1rem;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface-1);
    box-shadow: var(--shadow-card);
    cursor: pointer;
  }
  .accelerator:hover { border-color: color-mix(in srgb, var(--accent) 30%, var(--border)); }
  .accelerator-icon { width: 38px; height: 38px; display: grid; place-items: center; border-radius: 10px; color: var(--accent); background: color-mix(in srgb, var(--accent) 8%, var(--surface-2)); }
  .accelerator strong, .accelerator small, .sheet-head strong, .sheet-head small { display: block; }
  .accelerator strong, .sheet-head strong { font-size: 0.88rem; }
  .accelerator small, .sheet-head small { margin-top: 0.22rem; color: var(--text-subtle); font-size: 0.72rem; font-weight: 400; }
  .accelerator-action { color: var(--text-muted); font-size: 0.74rem; }

  .sheet-overlay { position: fixed; inset: 0; z-index: 40; display: grid; place-items: center; padding: 1rem; background: rgba(3, 7, 11, 0.66); backdrop-filter: blur(8px); }
  .sheet { width: min(1120px, 100%); max-height: calc(100vh - 2rem); display: flex; flex-direction: column; overflow: hidden; border: 1px solid var(--border-strong); border-radius: 18px; background: var(--bg); box-shadow: 0 30px 90px rgba(0, 0, 0, 0.38); }
  .sheet-head { flex: none; display: grid; grid-template-columns: auto minmax(0, 1fr) auto; align-items: center; gap: 0.8rem; padding: 0.8rem 1rem; border-bottom: 1px solid var(--border); background: var(--surface-1); }
  .accelerator-body { min-height: 0; overflow: auto; display: grid; grid-auto-rows: max-content; align-content: start; gap: 1rem; padding: 1rem; }
  .hero-grid { display: grid; grid-template-columns: minmax(0, 1.12fr) minmax(330px, 0.88fr); gap: 1rem; }
  .card { background: var(--surface-1); border: 1px solid var(--border); border-radius: var(--radius-lg); box-shadow: var(--shadow-card); }
  .gauge-card { padding: 1.25rem 1.35rem 0; overflow: hidden; }
  .connections-card { padding: 1.25rem 1.1rem 0.85rem; min-height: 440px; }
  .section-heading { justify-content: space-between; gap: 1rem; }
  .section-heading.compact { padding: 1.15rem 1.25rem 0; }
  .section-heading h1,
  .section-heading h2 { margin: 0.18rem 0 0; line-height: 1.18; letter-spacing: -0.025em; }
  .section-heading h1 { font-size: clamp(1.25rem, 2vw, 1.55rem); }
  .section-heading h2 { font-size: 1.12rem; }
  .eyebrow { margin: 0; color: var(--text-subtle); font-size: 0.68rem; font-weight: 700; letter-spacing: 0.09em; text-transform: uppercase; }
  .live-pill { display: inline-flex; align-items: center; gap: 0.42rem; color: var(--success); font-size: 0.75rem; font-weight: 650; }
  .live-pill span { width: 7px; height: 7px; border-radius: 50%; background: currentColor; box-shadow: 0 0 0 4px color-mix(in srgb, var(--success) 13%, transparent); }

  .gauge-card :global(svg) { width: min(320px, 75%); height: auto; margin: -0.2rem auto -0.25rem; }
  .metric-strip { display: grid; grid-template-columns: repeat(3, 1fr); margin: 0 -1.35rem; border-top: 1px solid var(--border); background: var(--surface-inset); }
  .metric { min-width: 0; padding: 0.88rem 1rem; border-right: 1px solid var(--border); }
  .metric:last-child { border-right: 0; }
  .metric-label { display: block; margin-bottom: 0.24rem; color: var(--text-subtle); font-size: 0.7rem; }
  .metric strong { font-family: var(--font-numeric); font-size: 0.95rem; font-variant-numeric: tabular-nums lining-nums; }

  .connection-heading { padding: 0 0.25rem 0.8rem; }
  .connection-list { display: grid; gap: 0.42rem; }
  .scan-result { gap: 0.4rem; margin: 0 0.25rem 0.65rem; padding: 0.5rem 0.65rem; border-radius: 8px; color: var(--success); background: color-mix(in srgb, var(--success) 8%, transparent); font-size: 0.78rem; }
  .spinning { display: inline-flex; animation: spin 0.8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .empty-state { min-height: 255px; display: flex; flex-direction: column; align-items: center; justify-content: center; text-align: center; padding: 1.5rem; color: var(--text-muted); }
  .empty-state strong { margin-top: 0.8rem; color: var(--text); font-size: 0.98rem; }
  .empty-state p { max-width: 34ch; margin: 0.4rem 0 1rem; font-size: 0.85rem; line-height: 1.42; }
  .empty-icon { width: 48px; height: 48px; display: grid; place-items: center; border-radius: 50%; color: var(--text-subtle); background: var(--surface-2); }
  .remembered { margin-top: 0.75rem; padding-top: 0.7rem; border-top: 1px solid var(--border); }
  .remembered summary { padding: 0.35rem 0.45rem; color: var(--text-subtle); cursor: pointer; font-size: 0.76rem; }
  .remembered-list { display: grid; gap: 0.35rem; margin-top: 0.35rem; opacity: 0.82; }

  .chart-card { overflow: hidden; }
  .chart-caption { color: var(--text-subtle); font-size: 0.74rem; }
  .chart-card :global(.chart-wrap) { padding: 0.3rem 1rem 0.85rem; }
  .task-card { overflow: hidden; border-color: color-mix(in srgb, var(--accent) 18%, var(--border)); }
  .back-to-result { display: block; margin: -0.35rem auto 0.85rem; border: 0; background: transparent; color: var(--text-muted); font-size: 0.76rem; cursor: pointer; }

  @media (max-width: 880px) {
    .hero-grid { grid-template-columns: 1fr; }
    .connections-card { min-height: auto; }
    .topbar { position: static; }
  }

  @media (max-width: 640px) {
    .topbar { padding: 0 1rem; }
    .brand-subtitle,
    .monitor-state,
    .top-actions .quiet { display: none; }
    .workspace { padding: 1rem; }
    .gauge-card { padding-inline: 1rem; }
    .metric-strip { margin-inline: -1rem; }
    .metric { padding-inline: 0.7rem; }
    .metric-label { min-height: 2em; }
    .chart-caption { display: none; }
    .connection-heading { align-items: flex-start; }
  }
</style>
