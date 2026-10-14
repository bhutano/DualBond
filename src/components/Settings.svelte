<script lang="ts">
  import { lang, t } from "../lib/i18n.svelte";
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import {
    doctorReport,
    forgetLink,
    getSettings,
    listLinks,
    renameLink,
    saveSettings,
    setLinkColor,
    setLinkEnabled,
    setLinkMetered,
    type DoctorReport,
    type LinkIdentity,
    type Settings as SettingsType,
  } from "../lib/ipc";
  import { monitor, systemBalance } from "../lib/stores.svelte";
  import { theme, type ThemePreference } from "../lib/theme.svelte";
  import { accent, ACCENT_PRESETS } from "../lib/accent.svelte";
  import Icon from "./Icon.svelte";

  let { onClose }: { onClose: () => void } = $props();

  type Tab = "connections" | "download" | "appearance" | "diagnostics";
  let tab = $state<Tab>("connections");
  let settings = $state<SettingsType | null>(null);
  let links = $state<LinkIdentity[]>([]);
  let loading = $state(true);
  let saving = $state(false);
  let error = $state<string | null>(null);
  let report = $state<DoctorReport | null>(null);
  let diagnosing = $state(false);

  const palette = ["#00E8FF", "#B026FF", "#FF6B35", "#4CD964", "#FFD60A", "#FF375F", "#5E5CE6", "#64D2FF"];
  const liveByKey = $derived(new Map((monitor.snapshot?.links ?? []).map((link) => [link.key, link])));

  onMount(async () => {
    try {
      [settings, links] = await Promise.all([getSettings(), listLinks()]);
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      loading = false;
    }
  });

  async function refreshLinks() {
    links = await listLinks();
  }

  async function refreshConnections() {
    await Promise.all([refreshLinks(), systemBalance.refresh()]);
  }

  async function persist() {
    if (!settings || saving) return;
    saving = true;
    error = null;
    try {
      await saveSettings(settings);
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      saving = false;
    }
  }

  async function chooseDirectory() {
    const selected = await open({ directory: true, multiple: false, title: t("Scegli la cartella dei download") });
    if (typeof selected === "string" && settings) {
      settings.download_directory = selected;
      await persist();
    }
  }

  async function updateLink(operation: () => Promise<void>) {
    error = null;
    try {
      await operation();
      await refreshLinks();
      try {
        await systemBalance.refresh();
      } catch {
        // Il registro locale resta utilizzabile anche se il servizio di
        // sistema è momentaneamente fermo.
      }
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  async function runDiagnostics() {
    diagnosing = true;
    error = null;
    try {
      report = await Promise.race([
        doctorReport(),
        new Promise<never>((_, reject) =>
          setTimeout(() => reject(new Error(t("La diagnostica non ha risposto entro 20 secondi."))), 20_000),
        ),
      ]);
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      diagnosing = false;
    }
  }

  function setTheme(preference: ThemePreference) {
    theme.set(preference);
  }

  function kindLabel(kind: string) {
    if (kind === "wifi") return "Wi-Fi";
    if (kind === "usb") return t("Tethering USB");
    if (kind === "ethernet") return "Ethernet";
    return t("Altra rete");
  }
</script>

<div class="overlay" onclick={onClose} role="presentation">
  <div
    class="panel"
    onclick={(event) => event.stopPropagation()}
    onkeydown={(event) => event.stopPropagation()}
    role="dialog"
    aria-modal="true"
    aria-labelledby="settings-title"
    tabindex="-1"
  >
    <header class="panel-header">
      <div>
        <p class="eyebrow">DualBond</p>
        <h1 id="settings-title">{t("Impostazioni")}</h1>
      </div>
      <button class="close-button" type="button" onclick={onClose} aria-label={t("Chiudi impostazioni")}>
        <Icon name="close" size={18} />
      </button>
    </header>

    <nav class="tabs" aria-label={t("Sezioni impostazioni")}>
      <button type="button" class:active={tab === "connections"} onclick={() => (tab = "connections")}>
        {t("Connessioni")}
      </button>
      <button type="button" class:active={tab === "download"} onclick={() => (tab = "download")}>{t("Download")}</button>
      <button type="button" class:active={tab === "appearance"} onclick={() => (tab = "appearance")}>{t("Aspetto")}</button>
      <button type="button" class:active={tab === "diagnostics"} onclick={() => (tab = "diagnostics")}>{t("Diagnostica")}</button>
    </nav>

    <div class="content">
      {#if error}
        <div class="error-message" role="alert">{error}</div>
      {/if}

      {#if loading}
        <p class="placeholder">{t("Caricamento delle impostazioni…")}</p>
      {:else if tab === "connections"}
        <div class="section-intro">
          <div>
            <h2>{t("Tutte le connessioni")}</h2>
            <p>{t("Le preferenze valgono sia per la modalità sistema sia per l’acceleratore file.")}</p>
          </div>
          <button class="secondary-button" type="button" onclick={() => void refreshConnections()} disabled={systemBalance.busy}>
            <Icon name="refresh" size={15} /> {t("Aggiorna")}
          </button>
        </div>

        {#if links.length === 0}
          <p class="placeholder">{t("Nessuna connessione è stata rilevata. Le nuove reti compariranno e saranno abilitate automaticamente.")}</p>
        {:else}
          <ul class="connection-list">
            {#each links as link (link.key)}
              {@const live = liveByKey.get(link.key)}
              <li class:offline={!live || live.state === "offline"}>
                <span class="connection-color" style={`--connection-color: ${link.color}`}></span>
                <div class="connection-main">
                  <input
                    class="connection-name"
                    value={link.displayName}
                    aria-label={t("Nome di {name}", { name: link.displayName })}
                    onblur={(event) => {
                      const name = event.currentTarget.value.trim();
                      if (name && name !== link.displayName) void updateLink(() => renameLink(link.key, name));
                    }}
                  />
                  <span>{kindLabel(link.kind)} · {live && live.state !== "offline" ? t("online") : t("offline")}</span>
                </div>
                <div class="color-picker" aria-label={t("Colore di {name}", { name: link.displayName })}>
                  {#each palette as color}
                    <button
                      type="button"
                      class:active={color.toLowerCase() === link.color.toLowerCase()}
                      style={`--swatch: ${color}`}
                      aria-label={t("Scegli {c}", { c: color })}
                      onclick={() => void updateLink(() => setLinkColor(link.key, color))}
                    ></button>
                  {/each}
                </div>
                <label class="metered-option">
                  <input
                    type="checkbox"
                    checked={link.metered}
                    onchange={() => void updateLink(() => setLinkMetered(link.key, !link.metered))}
                  />
                  {t("A consumo")}
                </label>
                <button
                  class="switch"
                  class:on={link.enabled}
                  type="button"
                  role="switch"
                  aria-checked={link.enabled}
                  aria-label={t("Usa {name}", { name: link.displayName })}
                  onclick={() => void updateLink(() => setLinkEnabled(link.key, !link.enabled))}
                ><span></span></button>
                <button
                  class="delete-button"
                  type="button"
                  aria-label={t("Dimentica {name}", { name: link.displayName })}
                  onclick={() => void updateLink(() => forgetLink(link.key))}
                ><Icon name="trash" size={16} /></button>
              </li>
            {/each}
          </ul>
        {/if}
      {:else if tab === "download" && settings}
        <div class="section-intro">
          <div>
            <h2>{t("Acceleratore file")}</h2>
            <p>{t("Queste opzioni si applicano soltanto ai download avviati dalla sezione dedicata.")}</p>
          </div>
        </div>
        <div class="form-grid">
          <div class="field wide">
            <label for="download-directory">{t("Cartella di destinazione")}</label>
            <div class="directory-row">
              <input
                id="download-directory"
                type="text"
                placeholder={t("Cartella Download di Windows")}
                bind:value={settings.download_directory}
                onblur={persist}
              />
              <button class="secondary-button" type="button" onclick={chooseDirectory}>{t("Scegli…")}</button>
            </div>
            <p>{t("Se il campo è vuoto, DualBond usa la cartella Download di Windows.")}</p>
          </div>
          <div class="field">
            <label for="max-connections">{t("Connessioni parallele per rete")}</label>
            <select id="max-connections" bind:value={settings.max_connections} onchange={persist}>
              <option value="auto">{t("Automatiche (consigliato)")}</option>
              {#each [2, 4, 8, 12, 16, 24, 32] as count}
                <option value={String(count)}>{count}</option>
              {/each}
            </select>
            <p>{t("Più connessioni non significano sempre maggiore velocità; la modalità automatica ne usa quattro.")}</p>
          </div>
        </div>
        <div class="info-card">
          {t("L’acceleratore file resta utile quando un singolo download può essere suddiviso in segmenti. La modalità sistema, invece, distribuisce i flussi completi senza spezzare una singola connessione cifrata.")}
        </div>
      {:else if tab === "appearance"}
        <div class="section-intro">
          <div><h2>{t("Aspetto")}</h2><p>{t("Tema e colori vengono applicati subito e restano memorizzati su questo PC.")}</p></div>
        </div>
        <div class="field">
          <span class="field-label">{t("Tema")}</span>
          <div class="segmented" role="group" aria-label={t("Tema")}>
            <button type="button" class:active={theme.preference === "system"} onclick={() => setTheme("system")}>{t("Sistema")}</button>
            <button type="button" class:active={theme.preference === "light"} onclick={() => setTheme("light")}>{t("Chiaro")}</button>
            <button type="button" class:active={theme.preference === "dark"} onclick={() => setTheme("dark")}>{t("Scuro")}</button>
          </div>
        </div>
        <div class="field">
          <span class="field-label">{t("Colore principale")}</span>
          <div class="accent-grid">
            {#each ACCENT_PRESETS as preset (preset.id)}
              <button
                type="button"
                class:active={accent.presetId === preset.id}
                style={`--accent-a: ${preset.accent}; --accent-b: ${preset.accent2}`}
                onclick={() => accent.set(preset.id)}
              >
                <span></span>{t(preset.name.replace(" / ", " e "))}
              </button>
            {/each}
          </div>
        </div>
      {:else if tab === "diagnostics"}
        <div class="section-intro">
          <div><h2>{t("Diagnostica reti")}</h2><p>{t("Verifica che ogni interfaccia possa uscire con il proprio indirizzo e percorso.")}</p></div>
          <button class="primary-button" type="button" onclick={runDiagnostics} disabled={diagnosing}>
            <Icon name="activity" size={15} /> {diagnosing ? t("Verifica in corso…") : t("Avvia verifica")}
          </button>
        </div>
        {#if report}
          {#each report.warnings as warning}
            <div class="warning-message">{t(warning)}</div>
          {/each}
          <ul class="diagnostic-list">
            {#each report.links as link (link.stableKey)}
              <li>
                <div><strong>{link.name}</strong><span>{kindLabel(link.kind)} · {link.ips.join(", ")}</span></div>
                <div class="probe-list">
                  {#each link.probes as probe}
                    <span class:ok={probe.boundCorrectly}>{probe.family}: {probe.boundCorrectly ? t("percorso verificato") : t(probe.outcome)}</span>
                  {/each}
                </div>
              </li>
            {/each}
          </ul>
        {:else}
          <p class="placeholder">{t("Avvia la verifica per controllare tutte le connessioni disponibili.")}</p>
        {/if}
      {/if}
    </div>

    <footer><span>{saving ? t("Salvataggio…") : t("Le modifiche vengono salvate automaticamente.")}</span></footer>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 50; display: grid; place-items: center; padding: 1.25rem; background: rgba(3, 7, 11, 0.66); backdrop-filter: blur(8px); }
  .panel { width: min(880px, 96vw); height: min(720px, 92vh); display: grid; grid-template-rows: auto auto minmax(0, 1fr) auto; overflow: hidden; border: 1px solid var(--border-strong); border-radius: 18px; background: var(--surface-1); box-shadow: 0 30px 90px rgba(0, 0, 0, 0.38); }
  .panel-header { display: flex; align-items: center; justify-content: space-between; padding: 1.15rem 1.35rem 0.95rem; }
  .eyebrow { margin: 0 0 0.18rem; color: var(--text-subtle); font-size: 0.65rem; font-weight: 700; letter-spacing: 0.09em; text-transform: uppercase; }
  h1, h2, p { margin: 0; }
  h1 { font-size: 1.3rem; letter-spacing: -0.025em; }
  h2 { font-size: 1rem; letter-spacing: -0.015em; }
  .close-button, .delete-button { display: grid; place-items: center; border: 0; border-radius: 8px; background: transparent; color: var(--text-muted); cursor: pointer; }
  .close-button { width: 36px; height: 36px; }
  .tabs { display: flex; gap: 0.2rem; padding: 0 1.1rem; border-bottom: 1px solid var(--border); overflow-x: auto; }
  .tabs button { position: relative; padding: 0.72rem 0.78rem; border: 0; background: transparent; color: var(--text-muted); font-size: 0.78rem; cursor: pointer; white-space: nowrap; }
  .tabs button::after { content: ""; position: absolute; right: 0.7rem; bottom: -1px; left: 0.7rem; height: 2px; border-radius: 2px; background: transparent; }
  .tabs button.active { color: var(--text); }
  .tabs button.active::after { background: var(--accent); }
  .content { overflow-y: auto; padding: 1.25rem 1.35rem 1.6rem; }
  .section-intro { display: flex; align-items: flex-start; justify-content: space-between; gap: 1rem; margin-bottom: 1rem; }
  .section-intro p, .field p { max-width: 66ch; margin-top: 0.3rem; color: var(--text-subtle); font-size: 0.74rem; line-height: 1.4; }
  .primary-button, .secondary-button { display: inline-flex; align-items: center; justify-content: center; gap: 0.38rem; min-height: 34px; padding: 0.4rem 0.7rem; border: 1px solid var(--border-strong); border-radius: 8px; background: var(--surface-2); color: var(--text); font-size: 0.74rem; cursor: pointer; white-space: nowrap; }
  .primary-button { border-color: transparent; background: var(--accent); color: #071216; font-weight: 700; }
  button:disabled { opacity: 0.48; cursor: default; }
  .connection-list, .diagnostic-list { display: grid; gap: 0.5rem; margin: 0; padding: 0; list-style: none; }
  .connection-list li { display: grid; grid-template-columns: auto minmax(130px, 1fr) auto auto auto auto; align-items: center; gap: 0.7rem; min-height: 64px; padding: 0.6rem 0.65rem; border: 1px solid var(--border); border-radius: 11px; background: var(--surface-2); }
  .connection-list li.offline { border-style: dashed; }
  .connection-color { width: 10px; height: 34px; border-radius: 5px; background: var(--connection-color); }
  .connection-main { min-width: 0; }
  .connection-main span { display: block; margin-top: 0.15rem; color: var(--text-subtle); font-size: 0.66rem; }
  .connection-name { width: 100%; padding: 0; border: 0; background: transparent; color: var(--text); font-size: 0.8rem; font-weight: 680; text-overflow: ellipsis; }
  .color-picker { display: flex; gap: 0.17rem; }
  .color-picker button { width: 15px; height: 15px; padding: 0; border: 2px solid transparent; border-radius: 50%; background: var(--swatch); cursor: pointer; }
  .color-picker button.active { border-color: var(--text); }
  .metered-option { display: flex; align-items: center; gap: 0.3rem; color: var(--text-muted); font-size: 0.7rem; white-space: nowrap; }
  .switch { position: relative; width: 40px; height: 23px; padding: 0; border: 0; border-radius: 12px; background: var(--surface-3); cursor: pointer; }
  .switch span { position: absolute; top: 3px; left: 3px; width: 17px; height: 17px; border-radius: 50%; background: var(--text-subtle); transition: transform 160ms ease; }
  .switch.on { background: color-mix(in srgb, var(--success) 72%, #183426); }
  .switch.on span { transform: translateX(17px); background: white; }
  .delete-button { width: 30px; height: 30px; }
  .delete-button:hover { color: var(--error); background: color-mix(in srgb, var(--error) 7%, transparent); }
  .form-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 1rem; }
  .field { min-width: 0; margin-bottom: 1.25rem; }
  .field.wide { grid-column: 1 / -1; }
  .field label, .field-label { display: block; margin-bottom: 0.4rem; color: var(--text-muted); font-size: 0.76rem; font-weight: 650; }
  .field input, .field select { min-height: 38px; padding: 0.45rem 0.6rem; border: 1px solid var(--border); border-radius: 8px; background: var(--surface-2); color: var(--text); font-size: 0.8rem; }
  .directory-row { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 0.45rem; }
  .directory-row input { width: 100%; }
  .info-card, .error-message, .warning-message { padding: 0.75rem 0.85rem; border: 1px solid var(--border); border-radius: 9px; color: var(--text-muted); background: var(--surface-2); font-size: 0.76rem; line-height: 1.42; }
  .error-message { margin-bottom: 0.85rem; border-color: color-mix(in srgb, var(--error) 30%, var(--border)); color: var(--error); }
  .warning-message { margin-bottom: 0.6rem; color: var(--warning); }
  .segmented { display: inline-flex; padding: 3px; border: 1px solid var(--border); border-radius: 9px; background: var(--surface-2); }
  .segmented button { min-width: 76px; padding: 0.42rem 0.65rem; border: 0; border-radius: 6px; background: transparent; color: var(--text-muted); font-size: 0.75rem; cursor: pointer; }
  .segmented button.active { background: var(--surface-3); color: var(--text); box-shadow: 0 2px 7px rgba(0,0,0,.15); }
  .accent-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 220px)); gap: 0.5rem; }
  .accent-grid button { display: flex; align-items: center; gap: 0.55rem; padding: 0.55rem; border: 1px solid var(--border); border-radius: 9px; background: var(--surface-2); color: var(--text-muted); font-size: 0.74rem; cursor: pointer; }
  .accent-grid button span { width: 30px; height: 24px; border-radius: 6px; background: linear-gradient(135deg, var(--accent-a), var(--accent-b)); }
  .accent-grid button.active { border-color: color-mix(in srgb, var(--accent) 50%, var(--border)); color: var(--text); }
  .diagnostic-list li { display: grid; grid-template-columns: minmax(180px, 1fr) 1.4fr; gap: 1rem; padding: 0.75rem; border: 1px solid var(--border); border-radius: 10px; background: var(--surface-2); }
  .diagnostic-list strong, .diagnostic-list span { display: block; }
  .diagnostic-list strong { font-size: 0.8rem; }
  .diagnostic-list div > span { margin-top: 0.2rem; color: var(--text-subtle); font-size: 0.67rem; }
  .probe-list span { color: var(--error); font-size: 0.7rem; }
  .probe-list span.ok { color: var(--success); }
  .placeholder { padding: 2rem 0.5rem; color: var(--text-muted); font-size: 0.8rem; }
  footer { padding: 0.7rem 1.35rem; border-top: 1px solid var(--border); color: var(--text-subtle); font-size: 0.68rem; }

  @media (max-width: 760px) {
    .overlay { padding: 0; }
    .panel { width: 100vw; height: 100vh; border: 0; border-radius: 0; }
    .connection-list li { grid-template-columns: auto minmax(0, 1fr) auto auto; }
    .color-picker { grid-column: 2 / -1; }
    .metered-option { grid-column: 2; }
    .form-grid, .diagnostic-list li { grid-template-columns: 1fr; }
  }
</style>
