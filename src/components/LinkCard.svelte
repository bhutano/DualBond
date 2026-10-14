<script lang="ts">
  import { lang, t } from "../lib/i18n.svelte";
  // Pannello di dettaglio di un link (§6.4): throughput medio/massimo,
  // errori, retry, colore, Rinomina/Cambia colore. È qui che vive il
  // gergo tecnico, non nella riga compatta.

  import type { LinkStats } from "../lib/ipc";
  import { renameLink, setLinkColor, forgetLink } from "../lib/ipc";
  import { formatMbps, formatBytes, formatRtt } from "../lib/format";
  import { untrack } from "svelte";
  import Icon from "./Icon.svelte";

  let { link, onClose }: { link: LinkStats; onClose: () => void } = $props();

  const PALETTE = ["#00E8FF", "#B026FF", "#FF6B35", "#4CD964", "#FFD60A", "#FF375F", "#5E5CE6", "#64D2FF"];

  let renaming = $state(false);
  let nameDraft = $state(untrack(() => link.name));

  async function saveName() {
    if (nameDraft.trim()) await renameLink(link.key, nameDraft.trim());
    renaming = false;
  }

  async function pickColor(color: string) {
    await setLinkColor(link.key, color);
  }

  async function onForget() {
    await forgetLink(link.key);
    onClose();
  }
</script>

<div class="overlay" onclick={onClose} role="presentation">
  <div
    class="card"
    onclick={(e) => e.stopPropagation()}
    onkeydown={(e) => e.stopPropagation()}
    role="dialog"
    tabindex="-1"
    aria-label={t("Dettaglio {name}", { name: link.name })}
  >
    <div class="header">
      <span class="dot" style="--dot-color: {link.color}"></span>
      {#if renaming}
        <input
          bind:value={nameDraft}
          onkeydown={(e) => e.key === "Enter" && saveName()}
          onblur={saveName}
        />
      {:else}
        <button type="button" class="name-btn" onclick={() => (renaming = true)}>{link.name}</button>
      {/if}
      <button type="button" class="close" onclick={onClose} aria-label={t("Chiudi")}><Icon name="close" size={17} /></button>
    </div>

    <div class="colors">
      {#each PALETTE as c}
        <button
          type="button"
          class="swatch"
          class:active={c.toLowerCase() === link.color.toLowerCase()}
          style="--swatch-color: {c}"
          onclick={() => pickColor(c)}
          aria-label={t("Colore {c}", { c })}
        ></button>
      {/each}
    </div>

    <dl>
      <dt>{t("Velocità attuale")}</dt>
      <dd>{formatMbps(link.mbps)}</dd>
      <dt>{t("Media 10 s")}</dt>
      <dd>{formatMbps(link.avgMbps)}</dd>
      <dt>{t("Picco sessione")}</dt>
      <dd>{formatMbps(link.peakMbps)}</dd>
      {#if link.rttMs != null}
        <dt>RTT</dt>
        <dd>{formatRtt(link.rttMs)}</dd>
      {/if}
      <dt>{t("Connessioni attive")}</dt>
      <dd>{link.connections}</dd>
      <dt>{t("Errori")}</dt>
      <dd>{link.errors}</dd>
      <dt>{t("Traffico totale")}</dt>
      <dd>{formatBytes(link.bytes)}</dd>
      <dt>{t("Chiave stabile")}</dt>
      <dd class="mono">{link.key}</dd>
    </dl>

    <button type="button" class="forget" onclick={onForget}>{t("Dimentica questo link")}</button>
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

  .card {
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 1.25rem;
    width: min(360px, 90vw);
    display: flex;
    flex-direction: column;
    gap: 0.9rem;
  }

  .header {
    display: flex;
    align-items: center;
    gap: 0.6rem;
  }

  .dot {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--dot-color);
    flex-shrink: 0;
  }

  .name-btn {
    flex: 1;
    text-align: left;
    background: none;
    border: none;
    color: var(--text);
    font-size: 1.05rem;
    font-weight: 600;
    cursor: pointer;
    padding: 0;
  }

  .header input {
    flex: 1;
    background: var(--surface-2);
    border: 1px solid var(--accent);
    border-radius: 6px;
    padding: 0.3rem 0.5rem;
    color: var(--text);
    font-size: 1.05rem;
  }

  .close {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 1rem;
  }

  .colors {
    display: flex;
    gap: 0.4rem;
  }

  .swatch {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background: var(--swatch-color);
    border: 2px solid transparent;
    cursor: pointer;
  }

  .swatch.active {
    border-color: var(--text);
  }

  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 0.35rem 0.8rem;
    margin: 0;
    font-size: 0.85rem;
  }

  dt {
    color: var(--text-muted);
  }

  dd {
    margin: 0;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .mono {
    font-family: ui-monospace, monospace;
    font-size: 0.75rem;
    word-break: break-all;
  }

  .forget {
    background: none;
    border: 1px solid var(--border);
    color: var(--error);
    border-radius: 6px;
    padding: 0.4rem;
    cursor: pointer;
  }
</style>
