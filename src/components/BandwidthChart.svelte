<script lang="ts">
  import { lang, t } from "../lib/i18n.svelte";
  // Grafico temporale (§6.5): uPlot, area impilata con un layer per link
  // nei rispettivi colori più la linea del totale in sovrimpressione.
  // Finestra visibile 60s (default) / 300s, buffer 300s.

  import uPlot from "uplot";
  import "uplot/dist/uPlot.min.css";
  import type { LinkStats } from "../lib/ipc";
  import { onDestroy } from "svelte";

  let { links, totalMbps, ts }: { links: LinkStats[]; totalMbps: number; ts: number } = $props();

  const BUFFER_SECONDS = 300;
  const MAX_SAMPLES = BUFFER_SECONDS * 4; // 4Hz

  type Sample = { ts: number; total: number; byKey: Map<string, number> };
  let history: Sample[] = [];
  let knownKeys: string[] = [];
  let knownColors = new Map<string, string>();

  let windowSeconds = $state(60);
  let containerEl: HTMLDivElement;
  let plot: uPlot | null = null;
  let lastTs = 0;

  $effect(() => {
    // Dipendenza esplicita su ts: ogni nuovo snapshot spinge un campione.
    if (ts === lastTs) return;
    lastTs = ts;

    const byKey = new Map<string, number>();
    for (const l of links) {
      if (l.state === "disabled" || l.state === "offline") continue;
      byKey.set(l.key, l.mbps);
      if (!knownKeys.includes(l.key)) knownKeys.push(l.key);
      knownColors.set(l.key, l.color);
    }
    history.push({ ts: ts / 1000, total: totalMbps, byKey });
    if (history.length > MAX_SAMPLES) history.shift();

    render();
  });

  function buildSeries() {
    const xs = history.map((s) => s.ts);
    const stacked: number[][] = [];
    let cumulative = new Array(history.length).fill(0);
    for (const key of knownKeys) {
      const layer = history.map((s, i) => {
        cumulative[i] += s.byKey.get(key) ?? 0;
        return cumulative[i];
      });
      stacked.push(layer);
    }
    const totalLine = history.map((s) => s.total);
    return { xs, stacked, totalLine };
  }

  function render() {
    if (!containerEl) return;
    const { xs, stacked, totalLine } = buildSeries();
    const data: uPlot.AlignedData = [xs, ...stacked, totalLine] as uPlot.AlignedData;

    if (!plot) {
      const series: uPlot.Series[] = [
        {},
        ...knownKeys.map((key) => ({
          label: key,
          stroke: knownColors.get(key) ?? "#888",
          fill: (knownColors.get(key) ?? "#888") + "55",
          width: 1,
          points: { show: false },
        })),
        {
          label: t("Totale"),
          stroke: "var(--text)",
          width: 1.5,
          points: { show: false },
        },
      ];
      plot = new uPlot(
        {
          width: containerEl.clientWidth || 640,
          height: 140,
          series,
          scales: { x: { time: false } },
          axes: [
            { stroke: "var(--text-muted)", grid: { stroke: "var(--border)" } },
            { stroke: "var(--text-muted)", grid: { stroke: "var(--border)" } },
          ],
          legend: { show: false },
        },
        data,
        containerEl,
      );
    } else {
      // Il numero di serie può crescere se compare un nuovo link: uPlot
      // non supporta aggiungere serie a caldo, quindi in quel caso si
      // ricrea il plot da zero (raro: succede solo al primo rilevamento
      // di un link nuovo, non a ogni campione).
      if (plot.series.length !== data.length) {
        plot.destroy();
        plot = null;
        render();
        return;
      }
      const from = xs.length > 0 ? xs[xs.length - 1] - windowSeconds : 0;
      plot.setData(data);
      plot.setScale("x", { min: from, max: xs[xs.length - 1] ?? windowSeconds });
    }
  }

  function setWindow(seconds: number) {
    windowSeconds = seconds;
    render();
  }

  onDestroy(() => plot?.destroy());
</script>

<div class="chart-wrap">
  <div class="selector">
    <button type="button" class:active={windowSeconds === 60} onclick={() => setWindow(60)}>1 min</button>
    <button type="button" class:active={windowSeconds === 300} onclick={() => setWindow(300)}>5 min</button>
  </div>
  <div bind:this={containerEl} class="chart"></div>
</div>

<style>
  .chart-wrap {
    width: 100%;
  }

  .chart {
    width: 100%;
  }

  .selector {
    display: flex;
    gap: 0.4rem;
    justify-content: flex-end;
    margin-bottom: 0.3rem;
  }

  .selector button {
    background: transparent;
    border: 1px solid var(--border);
    color: var(--text-muted);
    border-radius: 6px;
    padding: 0.15rem 0.5rem;
    font-size: 0.8rem;
    cursor: pointer;
  }

  .selector button.active {
    color: var(--text);
    border-color: var(--accent);
  }
</style>
