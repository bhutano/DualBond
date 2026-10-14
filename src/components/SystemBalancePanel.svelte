<script lang="ts">
  import { lang, t } from "../lib/i18n.svelte";
  import { formatMbps } from "../lib/format";
  import { systemBalance, monitor } from "../lib/stores.svelte";
  import { SCALE_STEPS } from "../lib/scale";
  import Icon from "./Icon.svelte";
  import MiniGauge from "./MiniGauge.svelte";

  const status = $derived(systemBalance.status);
  const active = $derived(status?.state === "active");
  const availableLinks = $derived(
    (status?.links ?? [])
      .filter((link) => link.enabled && link.state === "online")
      .sort((left, right) => left.name.localeCompare(right.name, lang.current)),
  );
  // link presenti ma disattivati dall'utente: restano in elenco, attenuati e
  // con l'interruttore spento, così si possono riaccendere da qui
  const disabledLinks = $derived((status?.links ?? []).filter((link) => !link.enabled && link.state !== "offline"));
  const feedLinks = $derived(
    [...availableLinks, ...disabledLinks].sort((left, right) => left.name.localeCompare(right.name, lang.current)),
  );
  // Velocità mostrate: traffico reale di ogni scheda, misurato dal monitor
  // sempre, dall'avvio, anche a tunnel fermo o con il link escluso. Il
  // servizio conta solo il traffico che passa da DualBond: resta il ripiego
  // se il monitor non ha (ancora) un campione per quel link.
  function liveOf(link: { key: string; mbps: number; avgMbps: number }) {
    const measured = monitor.snapshot?.links.find((entry) => entry.key === link.key);
    return measured && measured.state !== "offline" && typeof measured.liveMbps === "number"
      ? { mbps: measured.liveMbps, avg: measured.liveAvgMbps }
      : { mbps: link.mbps, avg: link.avgMbps };
  }
  const live = $derived(new Map(feedLinks.map((link) => [link.key, liveOf(link)])));
  const totalNow = $derived([...live.values()].reduce((sum, rate) => sum + rate.mbps, 0));
  const totalAvg = $derived([...live.values()].reduce((sum, rate) => sum + rate.avg, 0));
  const scaleMax = $derived.by(() => {
    const peak = Math.max(totalNow, ...[...live.values()].map((rate) => rate.mbps), 0);
    return SCALE_STEPS.find((step) => step >= peak * 1.15) ?? SCALE_STEPS[SCALE_STEPS.length - 1];
  });
  const palette = ["#69d6e8", "#d889f2", "#f2a764", "#73d69a", "#f0d36d", "#ec8190"];

  function linkColor(key: string) {
    const index = Math.max(0, feedLinks.findIndex((link) => link.key === key));
    return monitor.snapshot?.links.find((link) => link.key === key)?.color ?? palette[index % palette.length];
  }

  function stateLabel() {
    if (!status?.serviceAvailable) return t("Servizio non disponibile");
    if (status.state === "active") return t("Tutto il PC è bilanciato");
    if (status.state === "starting") return t("Attivazione in corso…");
    if (status.state === "waiting") return t("In attesa di una connessione utilizzabile");
    if (status.state === "error") return t("Intervento richiesto");
    return t("Pronto");
  }

  function errorTitle() {
    return status?.state === "error" ? t("Il tunnel non è partito.") : t("Il servizio non risponde.");
  }

  function kindLabel(kind: string) {
    if (kind === "usb") return "USB";
    if (kind === "wifi") return "Wi-Fi";
    if (kind === "ethernet") return "Ethernet";
    return t("Rete");
  }

  // ---- linee che confluiscono nel tachimetro totale ----------------------
  // Ogni link ha la sua linea: parte dal bordo destro della sua scheda ed
  // entra nel tachimetro grande in un punto tutto suo, con un tracciato
  // squadrato _|‾. Il tratteggio si muove verso il totale, più veloce quando
  // il link trasporta più traffico, fermo quando è a zero.
  type Flow = { key: string; d: string; color: string; duration: number; idle: boolean; end: { x: number; y: number } };

  let stageEl = $state<HTMLDivElement | null>(null);
  let hubEl = $state<HTMLDivElement | null>(null);
  let geometry = $state<{
    width: number;
    height: number;
    points: Map<string, { x: number; y: number }>;
    hub: { left: number; top: number; height: number };
  } | null>(null);

  function measure() {
    if (!stageEl || !hubEl) return;
    const stage = stageEl.getBoundingClientRect();
    const hub = (hubEl.querySelector(".gauge") ?? hubEl).getBoundingClientRect();
    const points = new Map<string, { x: number; y: number }>();
    stageEl.querySelectorAll<HTMLElement>("[data-feed]").forEach((feed) => {
      const rect = feed.getBoundingClientRect();
      points.set(feed.dataset.feed!, { x: rect.right - stage.left, y: rect.top + rect.height / 2 - stage.top });
    });
    geometry = {
      width: stage.width,
      height: stage.height,
      points,
      hub: { left: hub.left - stage.left, top: hub.top - stage.top, height: hub.height },
    };
  }

  $effect(() => {
    if (!stageEl) return;
    const observer = new ResizeObserver(() => measure());
    observer.observe(stageEl);
    return () => observer.disconnect();
  });

  // ri-misura quando cambia l'insieme dei link visibili
  $effect(() => {
    feedLinks.map((link) => `${link.key}:${link.enabled}`).join("|");
    queueMicrotask(measure);
  });

  function squarePath(x1: number, y1: number, x2: number, y2: number, mx: number) {
    const dy = y2 - y1;
    if (Math.abs(dy) < 1) return `M ${x1} ${y1} H ${x2}`;
    const dir = Math.sign(dy);
    const r = Math.max(0, Math.min(8, Math.abs(dy) / 2, mx - x1, x2 - mx));
    return `M ${x1} ${y1} H ${mx - r} Q ${mx} ${y1} ${mx} ${y1 + dir * r} V ${y2 - dir * r} Q ${mx} ${y2} ${mx + r} ${y2} H ${x2}`;
  }

  // Punti d'ingresso nel tachimetro totale: stesso ordine delle schede,
  // raccolti attorno al centro del quadrante ma sempre ben separati.
  function entryPoints(starts: { x: number; y: number }[], hub: { left: number; top: number; height: number }) {
    const n = starts.length;
    if (n === 0) return [];
    const gap = Math.min(34, (hub.height * 0.5) / Math.max(1, n - 1));
    const mid = Math.min(Math.max(starts.reduce((sum, s) => sum + s.y, 0) / n, hub.top + hub.height * 0.3), hub.top + hub.height * 0.62);
    return starts.map((_, i) => ({ x: hub.left, y: mid + (i - (n - 1) / 2) * gap }));
  }

  // Tutte le linee girano alla stessa x. Solo se due tratti verticali si
  // sovrapporrebbero, ognuna gira a una x sua, annidate così da non incrociarsi.
  function turnPoints(starts: { x: number; y: number }[], ends: { x: number; y: number }[]) {
    const left = Math.max(...starts.map((s) => s.x));
    const right = Math.min(...ends.map((e) => e.x));
    const shared = left + (right - left) * 0.5;
    const spans = starts.map((s, i) => [Math.min(s.y, ends[i].y), Math.max(s.y, ends[i].y)]);
    const clash = spans.some(([a1, b1], i) => spans.some(([a2, b2], j) => i < j && a1 < b2 + 6 && a2 < b1 + 6 && Math.abs(starts[i].y - ends[i].y) > 1 && Math.abs(starts[j].y - ends[j].y) > 1));
    if (!clash) return starts.map(() => shared);
    // chi deve scendere di più gira più vicino al totale (e viceversa per chi sale)
    const reach = starts.map((s, i) => ends[i].y - s.y);
    const order = reach.map((r, i) => ({ r, i })).sort((a, b) => Math.abs(a.r) - Math.abs(b.r));
    const step = (right - left) / (order.length + 1);
    const xs = new Array<number>(starts.length);
    order.forEach(({ i }, rank) => (xs[i] = left + step * (rank + 1)));
    return xs;
  }

  const flows = $derived.by((): Flow[] => {
    if (!geometry) return [];
    const { points, hub } = geometry;
    // ogni connessione misurata confluisce nel totale, anche se esclusa dal
    // bilanciamento: il suo traffico reale fa comunque parte della velocità
    const placed = feedLinks.flatMap((link) => {
      const start = points.get(link.key);
      return start ? [{ link, start }] : [];
    });
    const ends = entryPoints(placed.map((p) => p.start), hub);
    const turns = turnPoints(placed.map((p) => p.start), ends);
    return placed.map(({ link, start }, i) => {
      const end = ends[i];
      // tracciato squadrato: orizzontale → verticale → orizzontale
      const d = squarePath(start.x, start.y, end.x, end.y, turns[i]);
      const mbps = Math.max(0, live.get(link.key)?.mbps ?? 0);
      const idle = mbps < 0.5;
      // 1 Mbps ≈ 1,6 s per ciclo, 1 Gbps ≈ 0,35 s: scala logaritmica, mai sotto 0,3 s
      const duration = Math.max(0.3, 1.6 - Math.log10(Math.max(1, mbps)) * 0.42);
      return { key: link.key, d, color: linkColor(link.key), duration, idle, end };
    });
  });
</script>

<section class="system-panel" class:active class:error={status?.state === "error"} aria-labelledby="system-title">
  <div class="system-primary">
    <div class="system-copy">
      <span class="mode-kicker"><span class="mode-dot"></span> {t("Modalità sistema")}</span>
      <h1 id="system-title">{t("Un’unica Internet, dietro tutte le tue connessioni.")}</h1>
      <p>{t("DualBond distribuisce automaticamente i flussi di Windows tra Ethernet, Wi-Fi e tethering USB.")}</p>
    </div>

    <div class="master-control">
      <span class="control-state">{stateLabel()}</span>
      <button
        class="master-switch"
        class:on={status?.enabled}
        type="button"
        role="switch"
        aria-checked={status?.enabled ?? false}
        aria-label={t("Bilanciamento di sistema")}
        disabled={!status?.serviceAvailable || systemBalance.busy}
        onclick={() => void systemBalance.toggle(!(status?.enabled ?? false))}
      >
        <span></span>
      </button>
    </div>
  </div>

  {#if status?.lastError}
    <div class="system-error" role="alert">
      <Icon name="activity" size={17} />
      <div><strong>{errorTitle()}</strong><span>{status.lastError}</span></div>
    </div>
  {/if}

  <div class="stage" bind:this={stageEl} aria-label={t("Tachimetri modalità sistema")}>
    <div class="feeds">
      <div class="feeds-heading">
        <span>{t("Percorsi disponibili")}</span>
        <button type="button" onclick={() => void systemBalance.refresh()} disabled={systemBalance.busy}>
          <span class:spinning={systemBalance.busy}><Icon name="refresh" size={14} /></span>
          {t("Rileva ora")}
        </button>
      </div>
      {#if !status}
        <span class="link-placeholder">{t("Lettura del servizio…")}</span>
      {:else if feedLinks.length === 0}
        <span class="link-placeholder">{t("Nessuna connessione Internet stabile disponibile.")}</span>
      {:else}
        {#if availableLinks.length === 0}
          <p class="off-hint">{disabledLinks.length === 1
            ? t("{name} è esclusa dal bilanciamento: accendila qui sotto per usarla con DualBond.", { name: disabledLinks[0].name })
            : t("Le tue connessioni sono escluse dal bilanciamento: accendile qui sotto per usarle con DualBond.")}</p>
        {/if}
        <div class="feed-list" style={`--feeds: ${feedLinks.length}`}>
          {#each feedLinks as link (link.key)}
            <div class="feed" class:off={!link.enabled} data-feed={link.key}>
              <MiniGauge
                row
                label={link.name}
                detail={link.enabled ? `${kindLabel(link.kind)} · ${link.localIp}` : `${kindLabel(link.kind)} · ${t("Esclusa")}`}
                value={live.get(link.key)?.mbps ?? 0}
                average={live.get(link.key)?.avg ?? 0}
                {scaleMax}
                color={linkColor(link.key)}
                online={link.state !== "offline"}
              >
                <button
                  class="link-switch"
                  class:on={link.enabled}
                  type="button"
                  role="switch"
                  aria-label={t("Usa {name}", { name: link.name })}
                  aria-checked={link.enabled}
                  onclick={() => void systemBalance.setLinkEnabled(link.key, !link.enabled)}
                ><span></span></button>
              </MiniGauge>
            </div>
          {/each}
        </div>
      {/if}
    </div>

    {#if geometry}
      <svg class="flows" width={geometry.width} height={geometry.height} aria-hidden="true">
        {#each flows as flow (flow.key)}
          <path class="flow-base" d={flow.d} style={`--flow-color: ${flow.color}`} />
          <path
            class="flow"
            class:idle={flow.idle}
            d={flow.d}
            style={`--flow-color: ${flow.color}; --flow-duration: ${flow.duration}s`}
          />
          <circle class="flow-sink" class:idle={flow.idle} cx={flow.end.x} cy={flow.end.y} r="4" style={`--flow-color: ${flow.color}`} />
        {/each}
      </svg>
    {/if}

    <div class="hub" bind:this={hubEl}>
      <MiniGauge
        label={t("Totale combinato")}
        value={totalNow}
        average={totalAvg}
        {scaleMax}
        color="var(--accent)"
        online={feedLinks.length > 0}
        featured
      />
    </div>
  </div>

  <div class="system-stats">
    <div class="system-metric featured">
      <span>{t("Traffico ora")}</span>
      <strong>{formatMbps(totalNow)}</strong>
    </div>
    <div class="system-metric">
      <span>{t("Media")}</span>
      <strong>{formatMbps(totalAvg)}</strong>
    </div>
    <div class="system-metric">
      <span>{t("Link disponibili")}</span>
      <strong>{availableLinks.length}</strong>
    </div>
    <div class="system-metric">
      <span>{t("Flussi vivi")}</span>
      <strong>{(status?.tcpFlows ?? 0) + (status?.udpFlows ?? 0)}</strong>
    </div>
  </div>

  <p class="system-note">
    {t("Bilanciamento automatico per flusso. I singoli download cifrati non vengono spezzati; per quelli usa l’acceleratore file qui sotto.")}
  </p>
</section>

<style>
  .system-panel {
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border: 1px solid color-mix(in srgb, var(--accent) 22%, var(--border));
    border-radius: 20px;
    background:
      linear-gradient(118deg, color-mix(in srgb, var(--accent) 8%, transparent), transparent 42%),
      var(--surface-1);
    box-shadow: var(--shadow-card);
    transition: border-color 180ms ease, box-shadow 180ms ease;
  }

  .system-panel.active {
    border-color: color-mix(in srgb, var(--success) 42%, var(--border));
    box-shadow: 0 18px 60px color-mix(in srgb, var(--success) 7%, transparent);
  }

  .system-panel.error { border-color: color-mix(in srgb, var(--error) 50%, var(--border)); }
  .system-primary { flex: none; display: flex; align-items: center; justify-content: space-between; gap: 1.5rem; padding: 1rem 1.35rem 0.7rem; }
  .system-copy { min-width: 0; max-width: 720px; }
  .mode-kicker { display: inline-flex; align-items: center; gap: 0.45rem; color: var(--text-subtle); font-size: 0.66rem; font-weight: 700; letter-spacing: 0.08em; text-transform: uppercase; }
  .mode-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--text-subtle); }
  .active .mode-dot { background: var(--success); box-shadow: 0 0 0 5px color-mix(in srgb, var(--success) 12%, transparent); }
  h1 { margin: 0.3rem 0 0.25rem; font-size: clamp(1.2rem, 2.2vw, 1.65rem); line-height: 1.12; letter-spacing: -0.03em; }
  p { margin: 0; color: var(--text-muted); font-size: 0.82rem; line-height: 1.4; }

  .master-control { flex: 0 0 auto; display: grid; justify-items: end; gap: 0.45rem; }
  .control-state { color: var(--text-muted); font-size: 0.76rem; font-weight: 650; }
  .master-switch, .link-switch { position: relative; border: 0; background: var(--surface-3); cursor: pointer; transition: background 160ms ease; }
  .master-switch { width: 64px; height: 36px; border-radius: 18px; box-shadow: inset 0 0 0 1px var(--border-strong); }
  .master-switch span { position: absolute; top: 5px; left: 5px; width: 26px; height: 26px; border-radius: 50%; background: var(--text-muted); box-shadow: 0 3px 8px rgba(0,0,0,.25); transition: transform 180ms cubic-bezier(.2,.8,.2,1), background 160ms ease; }
  .master-switch.on { background: color-mix(in srgb, var(--success) 75%, #183426); }
  .master-switch.on span { transform: translateX(28px); background: white; }
  .master-switch:disabled { opacity: 0.45; cursor: default; }

  .system-error { flex: none; display: flex; align-items: flex-start; gap: 0.65rem; margin: 0 1.35rem 0.6rem; padding: 0.6rem 0.85rem; border-radius: 10px; color: var(--error); background: color-mix(in srgb, var(--error) 8%, transparent); }
  .system-error strong, .system-error span { display: block; }
  .system-error strong { font-size: 0.82rem; }
  .system-error span { margin-top: 0.18rem; color: var(--text-muted); font-size: 0.74rem; }

  /* ---- palco: link a sinistra, linee al centro, totale a destra ---- */
  .stage {
    position: relative;
    flex: 1 1 auto;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(230px, 340px) minmax(56px, 1fr) auto;
    align-items: stretch;
    gap: 0;
    padding: 0.2rem 1.35rem 0.9rem;
  }
  .feeds { position: relative; z-index: 1; min-height: 0; display: flex; flex-direction: column; gap: 0.45rem; grid-column: 1; }
  .feeds-heading { flex: none; display: flex; align-items: center; justify-content: space-between; color: var(--text-subtle); font-size: 0.66rem; font-weight: 700; letter-spacing: 0.06em; text-transform: uppercase; }
  .feeds-heading button { display: inline-flex; align-items: center; gap: 0.3rem; border: 0; background: transparent; color: var(--text-muted); font-size: 0.7rem; text-transform: none; letter-spacing: 0; cursor: pointer; }
  .feed-list { flex: 1 1 auto; min-height: 0; display: flex; flex-direction: column; justify-content: center; gap: 0.45rem; }
  .feed { flex: 0 1 86px; min-height: 48px; }
  .link-placeholder { padding: 1rem 0.2rem; color: var(--text-muted); font-size: 0.8rem; }
  .off-hint { flex: none; margin: 0.2rem 0 0; max-width: 36ch; color: var(--warning); font-size: 0.78rem; }
  /* esclusa dal bilanciamento: ancora misurata, bordo tratteggiato e un
     filo attenuata; l'interruttore resta ben visibile */
  .feed.off :global(.gauge) { border-style: dashed; }
  .feed.off :global(.gauge > :not(.link-switch)) { opacity: 0.72; }

  .hub { position: relative; z-index: 1; grid-column: 3; min-height: 0; height: 100%; aspect-ratio: 1 / 1.08; max-width: 46vw; justify-self: end; }
  .hub :global(.gauge) { padding: 0.4rem 0.5rem 0.6rem; }
  .hub :global(svg) { flex: 1 1 auto; min-height: 0; width: 100%; height: 100%; margin: 0 !important; }

  .flows { position: absolute; inset: 0; z-index: 2; pointer-events: none; overflow: visible; }
  .flow-base { fill: none; stroke: var(--flow-color); stroke-width: 2; stroke-linejoin: round; opacity: 0.14; }
  .flow {
    fill: none;
    stroke: var(--flow-color);
    stroke-width: 2.4;
    stroke-linecap: round;
    stroke-linejoin: round;
    stroke-dasharray: 6 10;
    filter: drop-shadow(0 0 4px color-mix(in srgb, var(--flow-color) 55%, transparent));
    animation: flow var(--flow-duration, 1s) linear infinite;
  }
  .flow.idle { animation-play-state: paused; opacity: 0.35; filter: none; }
  .flow-sink { fill: var(--flow-color); stroke: var(--surface-1); stroke-width: 2; filter: drop-shadow(0 0 6px var(--flow-color)); animation: pulse 1.6s ease-in-out infinite; transform-box: fill-box; transform-origin: center; }
  .flow-sink.idle { animation: none; opacity: 0.45; filter: none; }
  @keyframes pulse { 50% { transform: scale(1.35); } }
  @keyframes flow { to { stroke-dashoffset: -32; } }
  @media (prefers-reduced-motion: reduce) { .flow, .flow-sink { animation: none; } }

  /* ---- metriche e nota ---- */
  .system-stats { flex: none; display: grid; grid-template-columns: 1.35fr repeat(3, 1fr); border-block: 1px solid var(--border); background: color-mix(in srgb, var(--surface-inset) 80%, transparent); }
  .system-metric { min-width: 0; padding: 0.55rem 1.15rem; border-right: 1px solid var(--border); }
  .system-metric:last-child { border-right: 0; }
  .system-metric span { display: block; margin-bottom: 0.15rem; color: var(--text-subtle); font-size: 0.67rem; }
  .system-metric strong { font-family: var(--font-numeric); font-size: 0.95rem; font-variant-numeric: tabular-nums lining-nums; }
  .system-metric.featured strong { color: var(--accent); font-size: 1.05rem; }

  .link-switch { flex: none; width: 36px; height: 20px; border-radius: 10px; }
  .link-switch span { position: absolute; top: 3px; left: 3px; width: 14px; height: 14px; border-radius: 50%; background: var(--text-subtle); transition: transform 160ms ease, background 160ms ease; }
  .link-switch.on { background: color-mix(in srgb, var(--success) 70%, #183426); }
  .link-switch.on span { transform: translateX(16px); background: white; }
  .system-note { flex: none; max-width: none; padding: 0.5rem 1.35rem 0.6rem; color: var(--text-subtle); font-size: 0.68rem; }
  .spinning { display: inline-flex; animation: spin 800ms linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }

  /* finestre basse: si tolgono le righe accessorie, mai i tachimetri */
  @media (max-height: 700px) {
    .system-copy p, .system-note { display: none; }
    .system-primary { padding-block: 0.7rem 0.4rem; }
  }

  /* finestre strette: totale sopra, link sotto, niente linee */
  @media (max-width: 640px) {
    .system-primary { align-items: flex-start; padding: 1rem; }
    .stage { grid-template-columns: 1fr; grid-template-rows: minmax(180px, 1fr) auto; padding-inline: 1rem; gap: 0.6rem; }
    .hub { grid-column: 1; grid-row: 1; justify-self: center; max-width: none; }
    .feeds { grid-row: 2; }
    .flows { display: none; }
    .system-stats { grid-template-columns: repeat(2, 1fr); }
    .system-metric:nth-child(2) { border-right: 0; }
    .system-metric:nth-child(-n+2) { border-bottom: 1px solid var(--border); }
  }
</style>
