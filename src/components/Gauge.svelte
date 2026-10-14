<script lang="ts">
  import { lang, t } from "../lib/i18n.svelte";
  // Gauge radiale (§6.3 della spec). SVG scritto a mano, nessuna libreria.
  // Arco 270° (135°→405°), tre strati: traccia di fondo, archi dei link
  // impilati (cumulativi sulla scala log condivisa, cosi' la punta
  // dell'arco impilato coincide sempre col marcatore del totale), e il
  // marcatore stesso.

  import { untrack } from "svelte";
  import type { LinkStats } from "../lib/ipc";
  import { valueToFraction, fractionToAngleDeg, tickValues, polarToCartesian, ARC_START_DEG } from "../lib/scale";
  import { formatMbps, formatGainFactor, speedParts } from "../lib/format";

  let { links, totalMbps, totalAvgMbps, scaleMax, gainFactor }: {
    links: LinkStats[];
    totalMbps: number;
    totalAvgMbps: number;
    scaleMax: number;
    gainFactor: number;
  } = $props();

  const SIZE = 320;
  const CENTER = SIZE / 2;
  const RADIUS = 130;
  const TRACK_WIDTH = 20;

  function arcPath(radius: number, startDeg: number, endDeg: number): string {
    if (endDeg <= startDeg) return "";
    const start = polarToCartesian(radius, startDeg);
    const end = polarToCartesian(radius, endDeg);
    const largeArc = endDeg - startDeg > 180 ? 1 : 0;
    return `M ${CENTER + start.x} ${CENTER + start.y} A ${radius} ${radius} 0 ${largeArc} 1 ${CENTER + end.x} ${CENTER + end.y}`;
  }

  const fullTrackPath = $derived(arcPath(RADIUS, 135, 405));

  // Animazione: i campioni arrivano a 4Hz, interpoliamo con
  // requestAnimationFrame + easing (~180ms) per un movimento fluido.
  // prefers-reduced-motion: aggiornamento a scatti, senza interpolazione.
  const prefersReducedMotion =
    typeof window !== "undefined" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  let displayedTotal = $state(0);
  let animFrame = 0;

  $effect(() => {
    const target = totalMbps;
    if (prefersReducedMotion) {
      displayedTotal = target;
      return;
    }
    // `untrack`: leggere `displayedTotal` qui senza tracciarlo è
    // essenziale — `step()` sotto lo scrive a ogni frame; se l'effect
    // dipendesse anche da `displayedTotal` (oltre che da `totalMbps`) si
    // ri-innescherebbe da solo a ogni frame, azzerando `startTime` prima
    // che l'interpolazione possa mai progredire (il numero resta fermo
    // vicino al valore iniziale — bug osservato: gauge fermo a 0 mentre
    // il grafico sotto, che non ha questo loop, si muove correttamente).
    const start = untrack(() => displayedTotal);
    const startTime = performance.now();
    const DURATION = 180;
    cancelAnimationFrame(animFrame);
    function step(now: number) {
      const t = Math.min(1, (now - startTime) / DURATION);
      const eased = 1 - Math.pow(1 - t, 3); // ease-out cubic
      displayedTotal = start + (target - start) * eased;
      if (t < 1) animFrame = requestAnimationFrame(step);
    }
    animFrame = requestAnimationFrame(step);
    return () => cancelAnimationFrame(animFrame);
  });

  // Archi impilati: ogni link parte da dove finisce la somma cumulativa
  // precedente (§6.3) — la punta dell'ultimo coincide col totale.
  const stackedArcs = $derived.by(() => {
    let cumulative = 0;
    const arcs: { key: string; color: string; d: string }[] = [];
    for (const l of links) {
      if (l.state === "disabled" || l.state === "offline") continue;
      const startFrac = valueToFraction(cumulative, scaleMax);
      cumulative += Math.max(0, l.mbps);
      const endFrac = valueToFraction(cumulative, scaleMax);
      const startDeg = fractionToAngleDeg(startFrac);
      const endDeg = fractionToAngleDeg(endFrac);
      arcs.push({ key: l.key, color: l.color, d: arcPath(RADIUS, startDeg, endDeg) });
    }
    return arcs;
  });

  const needleAngle = $derived(fractionToAngleDeg(valueToFraction(displayedTotal, scaleMax)));
  const needleTip = $derived(polarToCartesian(RADIUS, needleAngle));

  const ticks = $derived(
    tickValues(scaleMax).map((v) => {
      const angle = fractionToAngleDeg(valueToFraction(v, scaleMax));
      const inner = polarToCartesian(RADIUS - TRACK_WIDTH / 2 - 4, angle);
      const outer = polarToCartesian(RADIUS + TRACK_WIDTH / 2 + 4, angle);
      const label = polarToCartesian(RADIUS + TRACK_WIDTH / 2 + 18, angle);
      return { v, inner, outer, label };
    }),
  );

  function avgLabel(mbps: number) {
    const p = speedParts(mbps);
    return p.unit === "Gbps" ? `${p.value} Gbps` : p.value;
  }

  const activeLinks = $derived(links.filter((l) => l.state !== "disabled" && l.state !== "offline"));
  const hasActiveLinks = $derived(activeLinks.length > 0);
  const singleLinkName = $derived(activeLinks.length === 1 ? activeLinks[0].name : null);
</script>

<svg
  style="overflow: visible"
  width={SIZE}
  height={SIZE}
  viewBox="0 0 {SIZE} {SIZE}"
  role="img"
  aria-label={hasActiveLinks
    ? t("Velocità totale {total}, media {avg}, {gain}", { total: formatMbps(totalMbps), avg: formatMbps(totalAvgMbps), gain: formatGainFactor(gainFactor) })
    : t("Nessuna connessione attiva")}
  class:paused={!hasActiveLinks}
>
  <path
    d={fullTrackPath}
    fill="none"
    stroke="var(--gauge-track)"
    stroke-width={TRACK_WIDTH}
    stroke-linecap="round"
  />

  {#each stackedArcs as arc (arc.key)}
    <path d={arc.d} fill="none" stroke={arc.color} stroke-width={TRACK_WIDTH} stroke-linecap="butt" />
  {/each}

  {#each ticks as tick (tick.v)}
    <line
      x1={CENTER + tick.inner.x}
      y1={CENTER + tick.inner.y}
      x2={CENTER + tick.outer.x}
      y2={CENTER + tick.outer.y}
      stroke="var(--gauge-tick)"
      stroke-width="1"
    />
    <text
      x={CENTER + tick.label.x}
      y={CENTER + tick.label.y}
      fill="var(--text-muted)"
      font-size="11"
      text-anchor="middle"
      dominant-baseline="middle"
    >
      {tick.v}
    </text>
  {/each}

  {#if hasActiveLinks}
    <circle
      cx={CENTER + needleTip.x}
      cy={CENTER + needleTip.y}
      r="6"
      fill="var(--text)"
      class="needle-glow"
    />
  {/if}

  <text x={CENTER} y={CENTER - 8} text-anchor="middle" class="center-value">
    {hasActiveLinks ? speedParts(displayedTotal).value : "—"}
  </text>
  <text x={CENTER} y={CENTER + 16} text-anchor="middle" class="center-unit">
    {hasActiveLinks ? speedParts(displayedTotal).unit : t("Nessuna connessione")}
  </text>
  {#if hasActiveLinks}
    <text x={CENTER} y={CENTER + 38} text-anchor="middle" class="center-gain">
      {t("{name} · media {avg}", { name: singleLinkName ?? formatGainFactor(gainFactor), avg: avgLabel(totalAvgMbps) })}
    </text>
  {/if}
</svg>

<style>
  svg {
    display: block;
  }

  svg.paused {
    opacity: 0.55;
  }

  .center-value {
    font-family: var(--font-numeric);
    font-size: 42px;
    font-weight: 560;
    font-variant-numeric: tabular-nums;
    fill: var(--text);
  }

  .center-unit {
    font-size: 13px;
    fill: var(--text-muted);
  }

  .center-gain {
    font-size: 11px;
    fill: var(--text-muted);
  }

  .needle-glow {
    filter: drop-shadow(0 0 6px var(--text));
  }

  @media (prefers-reduced-motion: reduce) {
    .needle-glow {
      filter: none;
    }
  }
</style>
