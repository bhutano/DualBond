<script lang="ts">
  import { lang, t } from "../lib/i18n.svelte";
  import type { LinkStats } from "../lib/ipc";
  import { formatMbps } from "../lib/format";
  import { setLinkEnabled } from "../lib/ipc";
  import Icon from "./Icon.svelte";

  let { link, onSelect }: { link: LinkStats; onSelect: (key: string) => void } = $props();
  let changing = $state(false);
  let requestedEnabled = $state<boolean | null>(null);
  const effectiveEnabled = $derived(requestedEnabled ?? link.enabled);
  const visibleState = $derived(!effectiveEnabled && link.state !== "offline" ? "disabled" : link.state);

  $effect(() => {
    if (requestedEnabled !== null && link.enabled === requestedEnabled) requestedEnabled = null;
  });

  const kindLabel = (kind: string) =>
    ({ usb: "USB", wifi: "Wi-Fi", ethernet: "Ethernet", other: t("Altro") })[kind] ?? kind;
  const stateLabel = (state: string) =>
    ({
      online: t("Online"),
      probing: t("Verifica"),
      degraded: t("Instabile"),
      offline: t("Scollegata"),
      not_validated: t("Non utilizzabile"),
      disabled: t("Non usata"),
    })[state] ?? state;

  async function toggle() {
    if (changing) return;
    const nextEnabled = !effectiveEnabled;
    requestedEnabled = nextEnabled;
    changing = true;
    try {
      await setLinkEnabled(link.key, nextEnabled);
    } catch {
      requestedEnabled = null;
    } finally {
      changing = false;
    }
  }
</script>

<div class="chip state-{visibleState}" class:disabled={!effectiveEnabled}>
  <button type="button" class="chip-main" onclick={() => onSelect(link.key)} aria-label={t("Dettagli di {name}", { name: link.name })}>
    <span class="identity">
      <span class="dot" style="--dot-color: {link.color}" aria-hidden="true"></span>
      <span class="names">
        <span class="name-row">
          <span class="name">{link.name}</span>
          <span class="kind-badge">{kindLabel(link.kind)}</span>
          {#if link.metered}<span class="metered-badge">{t("A consumo")}</span>{/if}
        </span>
        <span class="state-label">{stateLabel(visibleState)}{link.signalPercent != null ? ` · ${t("segnale {n} %", { n: link.signalPercent })}` : ""}</span>
      </span>
    </span>

    <span class="rates">
      {#if visibleState === "offline"}
        <span class="offline-rate">—</span>
      {:else if !effectiveEnabled}
        <span class="rate-current">{formatMbps(0)}</span>
        <span class="rate-average">{t("Esclusa dal download")}</span>
      {:else}
        <span class="rate-current">{formatMbps(link.mbps)}</span>
        <span class="rate-average">{t("media {v}", { v: formatMbps(link.avgMbps) })}</span>
      {/if}
    </span>
  </button>

  <button
    type="button"
    class="switch"
    class:on={effectiveEnabled}
    onclick={toggle}
    disabled={changing}
    role="switch"
    aria-checked={effectiveEnabled}
    aria-label={effectiveEnabled ? t("Disattiva {name}", { name: link.name }) : t("Usa {name}", { name: link.name })}
    title={effectiveEnabled ? t("Escludi dai download") : t("Usa nei download")}
  >
    <span class="switch-thumb"><Icon name={effectiveEnabled ? "check" : "close"} size={10} /></span>
  </button>
</div>

<style>
  .chip {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    width: 100%;
    padding: 0.62rem 0.65rem;
    border: 1px solid var(--border);
    border-radius: 11px;
    background: var(--surface-inset);
    transition: border-color 150ms ease, background 150ms ease, transform 150ms ease;
  }
  .chip:hover { border-color: var(--border-strong); background: var(--surface-2); transform: translateY(-1px); }
  .chip.disabled { opacity: 0.78; }
  .chip-main { flex: 1; min-width: 0; display: flex; align-items: center; justify-content: space-between; gap: 0.75rem; padding: 0; border: 0; background: transparent; color: var(--text); text-align: left; cursor: pointer; }
  .identity { min-width: 0; display: flex; align-items: center; gap: 0.62rem; }
  .dot { width: 9px; height: 9px; border-radius: 50%; flex: 0 0 auto; background: var(--dot-color); box-shadow: 0 0 0 4px color-mix(in srgb, var(--dot-color) 12%, transparent); }
  .state-probing .dot { background: var(--warning); animation: pulse 1.2s ease-in-out infinite; }
  .state-degraded .dot { background: var(--warning); }
  .state-offline .dot { background: var(--text-subtle); box-shadow: none; }
  .state-not_validated .dot { background: var(--error); }
  @keyframes pulse { 50% { opacity: 0.35; } }
  .names { min-width: 0; display: flex; flex-direction: column; gap: 0.18rem; }
  .name-row { min-width: 0; display: flex; align-items: center; gap: 0.38rem; }
  .name { max-width: 170px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 0.88rem; font-weight: 650; }
  .kind-badge { flex: 0 0 auto; color: var(--text-subtle); font-size: 0.6rem; font-weight: 700; letter-spacing: 0.07em; text-transform: uppercase; }
  .metered-badge { color: var(--warning); font-size: 0.64rem; }
  .state-label { color: var(--text-subtle); font-size: 0.7rem; font-variant-numeric: tabular-nums; }
  .rates { flex: 0 0 auto; display: flex; flex-direction: column; align-items: flex-end; gap: 0.12rem; font-family: var(--font-numeric); font-variant-numeric: tabular-nums lining-nums; }
  .rate-current { font-size: 0.85rem; font-weight: 620; }
  .rate-average { color: var(--text-subtle); font-size: 0.66rem; }
  .offline-rate { color: var(--text-subtle); }
  .switch { width: 34px; height: 20px; flex: 0 0 auto; position: relative; padding: 2px; border: 1px solid var(--border-strong); border-radius: 999px; background: var(--surface-3); color: var(--text-subtle); cursor: pointer; transition: background 160ms ease, border-color 160ms ease; }
  .switch-thumb { width: 14px; height: 14px; display: grid; place-items: center; border-radius: 50%; background: var(--text-subtle); color: var(--surface-1); transition: transform 160ms ease, background 160ms ease; }
  .switch.on { border-color: color-mix(in srgb, var(--success) 45%, transparent); background: color-mix(in srgb, var(--success) 24%, var(--surface-2)); }
  .switch.on .switch-thumb { transform: translateX(14px); background: var(--success); color: #07140d; }
  .switch:disabled { cursor: wait; opacity: 0.6; }
  @media (max-width: 480px) { .rates { display: none; } .name { max-width: 150px; } }
</style>
