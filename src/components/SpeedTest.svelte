<script lang="ts">
  import { lang, t } from "../lib/i18n.svelte";
  // Pulsante [🔍 Test] della barra superiore (§6.2). **Semplificazione
  // nota**: un vero speed test dedicato (`metrics/benchmark.rs` del file
  // tree della spec) non è stato implementato — questo riusa la
  // validazione empirica del binding (`doctor`, già reale e già misura
  // un trasferimento di prova per link, §4.3) invece di un benchmark di
  // throughput puro separato. Rimandato a M7.

  import { doctorReport, type DoctorReport } from "../lib/ipc";

  let { onClose }: { onClose: () => void } = $props();

  let report = $state<DoctorReport | null>(null);
  let running = $state(false);
  let error = $state<string | null>(null);

  async function run() {
    running = true;
    report = null;
    error = null;
    try {
      report = await Promise.race([
        doctorReport(),
        new Promise<never>((_, reject) =>
          setTimeout(() => reject(new Error(t("La diagnostica non ha risposto entro 20 secondi."))), 20_000),
        ),
      ]);
    } catch (e) {
      error = String(e);
    } finally {
      running = false;
    }
  }

  run();
</script>

<div class="overlay" onclick={onClose} role="presentation">
  <div
    class="panel"
    onclick={(e) => e.stopPropagation()}
    onkeydown={(e) => e.stopPropagation()}
    role="dialog"
    tabindex="-1"
    aria-label={t("Test connessioni")}
  >
    <div class="header">
      <span>{t("Test connessioni")}</span>
      <button type="button" class="close" onclick={onClose} aria-label={t("Chiudi")}>✕</button>
    </div>
    {#if running}
      <p class="muted">{t("Verifica in corso…")}</p>
    {:else if error}
      <p class="error">{error}</p>
      <button type="button" class="rerun" onclick={run}>{t("Riprova")}</button>
    {:else if report}
      {#each report.warnings as warning}
        <p class="warning">{t(warning)}</p>
      {/each}
      {#if report.links.length === 0}
        <p class="muted">{t("Nessun link fisico rilevato.")}</p>
      {:else}
        <ul>
          {#each report.links as l}
            <li>
              <strong>{l.name}</strong>
              {#each l.probes as p}
                <div class="probe" class:ok={p.boundCorrectly}>{p.family}: {t(p.outcome)}</div>
              {/each}
            </li>
          {/each}
        </ul>
      {/if}
      <button type="button" class="rerun" onclick={run}>{t("Ripeti")}</button>
    {/if}
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.4);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 20;
  }

  .panel {
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 1.25rem;
    width: min(420px, 90vw);
  }

  .header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-weight: 600;
    margin-bottom: 0.8rem;
  }

  .close {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
  }

  .muted {
    color: var(--text-muted);
  }

  .error {
    color: var(--error);
  }

  .warning {
    color: var(--warning);
  }

  ul {
    list-style: none;
    padding: 0;
    margin: 0;
  }

  li {
    margin-bottom: 0.6rem;
  }

  .probe {
    font-size: 0.85rem;
    color: var(--error);
  }

  .probe.ok {
    color: var(--success);
  }

  .rerun {
    margin-top: 0.6rem;
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--text);
    border-radius: 6px;
    padding: 0.4rem 0.9rem;
    cursor: pointer;
  }
</style>
