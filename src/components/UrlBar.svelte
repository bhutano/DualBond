<script lang="ts">
  import { lang, t } from "../lib/i18n.svelte";
  import { startDownload } from "../lib/ipc";
  import { friendlyMessage } from "../lib/messages";
  import Icon from "./Icon.svelte";

  let { onStarted = () => {} }: { onStarted?: () => void } = $props();
  let url = $state("");
  let submitting = $state(false);
  let error = $state<string | null>(null);

  async function submit() {
    const value = url.trim();
    if (!value || submitting) return;
    try {
      const parsed = new URL(value);
      if (!['http:', 'https:'].includes(parsed.protocol)) throw new Error();
    } catch {
      error = t("Inserisci un indirizzo HTTP o HTTPS completo.");
      return;
    }

    submitting = true;
    error = null;
    try {
      await startDownload(value);
      url = "";
      onStarted();
    } catch (e) {
      error = friendlyMessage(String(e)).text;
    } finally {
      submitting = false;
    }
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.key === "Enter") submit();
  }
</script>

<div class="composer">
  <div class="composer-copy">
    <span class="composer-icon"><Icon name="download" size={19} /></span>
    <div>
      <p class="eyebrow">{t("Nuovo download multi-link")}</p>
      <h2>{t("Incolla l’indirizzo diretto del file")}</h2>
      <p class="helper">{t("DualBond distribuirà i segmenti sulle connessioni abilitate.")}</p>
    </div>
  </div>

  <div class="input-area">
    <label for="url-input">{t("Indirizzo del file")}</label>
    <div class="row">
      <input
        id="url-input"
        type="url"
        inputmode="url"
        autocomplete="url"
        placeholder={t("https://esempio.it/file.zip")}
        bind:value={url}
        onkeydown={onKeydown}
        disabled={submitting}
        aria-describedby={error ? "url-error" : undefined}
      />
      <button class="button primary submit" type="button" onclick={submit} disabled={submitting || !url.trim()}>
        <Icon name="download" size={16} /> {submitting ? t("Avvio…") : t("Avvia download")}
      </button>
    </div>
    {#if error}<p id="url-error" class="error" role="alert">{error}</p>{/if}
  </div>
</div>

<style>
  .composer { display: grid; grid-template-columns: minmax(260px, 0.75fr) minmax(360px, 1.25fr); align-items: center; gap: 1.5rem; padding: 1.15rem 1.25rem; }
  .composer-copy { display: flex; align-items: center; gap: 0.75rem; }
  .composer-icon { width: 38px; height: 38px; flex: 0 0 auto; display: grid; place-items: center; border-radius: 10px; color: var(--accent); background: color-mix(in srgb, var(--accent) 10%, var(--surface-2)); }
  .eyebrow { margin: 0 0 0.16rem; color: var(--text-subtle); font-size: 0.64rem; font-weight: 700; letter-spacing: 0.08em; text-transform: uppercase; }
  h2 { margin: 0; font-size: 0.95rem; letter-spacing: -0.012em; }
  .helper { max-width: 48ch; margin: 0.25rem 0 0; color: var(--text-subtle); font-size: 0.72rem; line-height: 1.35; }
  .input-area label { display: block; margin-bottom: 0.36rem; color: var(--text-muted); font-size: 0.7rem; }
  .row { display: flex; gap: 0.55rem; }
  input { min-width: 0; flex: 1; height: 42px; padding: 0 0.78rem; border: 1px solid var(--border-strong); border-radius: 9px; outline: none; background: var(--surface-inset); color: var(--text); font: inherit; font-size: 0.84rem; transition: border-color 150ms ease, box-shadow 150ms ease; }
  input::placeholder { color: var(--text-subtle); }
  input:focus { border-color: var(--accent); box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 13%, transparent); }
  .submit { min-width: 152px; justify-content: center; }
  .error { margin: 0.42rem 0 0; color: var(--error); font-size: 0.75rem; }
  @media (max-width: 760px) { .composer { grid-template-columns: 1fr; gap: 1rem; } }
  @media (max-width: 520px) { .row { flex-direction: column; } .submit { width: 100%; } }
</style>
