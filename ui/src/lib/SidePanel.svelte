<script lang="ts">
  // The side panel: a summary of the current column, a chart, the files
  // next to the open file, and recent activity. Everything is computed by
  // the engine in bounded passes; text is shown as text (no HTML).
  import AllNone from "./AllNone.svelte";
  import { untrack } from "svelte";
  import { backend, type Opened, type ReadOptions } from "./backend";
  import type { Tab } from "./tab.svelte";
  import { TYPE_NAMES, fmt, type ColType, type ChartData, type ChartKind, type Listing, type Summary } from "./views";

  export type PanelTab = "summary" | "chart" | "files" | "activity";

  interface Props {
    tab: Tab | undefined;
    which: PanelTab;
    titles: string[];
    activity: { time: string; text: string }[];
    read: ReadOptions;
    onOpened: (o: Opened) => void;
    /** `cut`: `value` is a shortened value ending in "…" (filter on its start). */
    onFilterValue: (col: number, value: string, cut: boolean) => void;
    onClose: () => void;
  }
  let { tab, which = $bindable(), titles, activity, read, onOpened, onFilterValue, onClose }: Props = $props();

  // ---- summary ----
  let summary = $state<Summary | null>(null);
  let problem = $state("");
  const col = $derived(tab ? tab.sel.focus.col : 0);
  $effect(() => {
    if (which !== "summary" || !tab || !tab.info.indexed) return;
    const t = tab;
    const c = col;
    void t.version;
    const header = t.header;
    const timer = setTimeout(() => {
      backend()
        .columnSummary(t.id, c, header, $state.snapshot(read) as ReadOptions)
        .then((s) => {
          summary = s;
          problem = "";
        })
        .catch((e: unknown) => (problem = (e as { message?: string }).message ?? String(e)));
    }, 250);
    return () => clearTimeout(timer);
  });

  // Every column's type (from a sample, so quick at any size).
  let types = $state<ColType[]>([]);
  $effect(() => {
    if (which !== "summary" || !tab || !tab.info.indexed) return;
    const t = tab;
    void t.version;
    const header = t.header;
    const timer = setTimeout(() => {
      backend()
        .columnTypes(t.id, header, $state.snapshot(read) as ReadOptions)
        .then((x) => (types = x))
        .catch(() => (types = []));
    }, 300);
    return () => clearTimeout(timer);
  });

  // ---- chart ----
  let kind = $state<ChartKind>("bar");
  let label = $state<number | null>(null);
  let values = $state<number[]>([]);
  let chart = $state<ChartData | null>(null);
  let canvas = $state<HTMLCanvasElement>();
  async function drawChart(): Promise<void> {
    if (!tab) return;
    // Nothing ticked yet: no chart (the list says to tick columns).
    if (kind !== "count" && !values.length) {
      chart = null;
      problem = "";
      return;
    }
    try {
      chart = await backend().chartData(tab.id, {
        kind,
        label: kind === "count" ? (label ?? col) : label,
        values: kind === "count" ? [] : $state.snapshot(values),
        skip_first: tab.header,
        read: $state.snapshot(read) as ReadOptions,
      });
      problem = "";
    } catch (e) {
      chart = null;
      problem = (e as { message?: string }).message ?? String(e);
    }
  }
  const COLORS = ["#3e63dd", "#e5484d", "#30a46c", "#f76b15", "#8e4ec6", "#12a594", "#ffc53d", "#8b8d98"];
  $effect(() => {
    const c = canvas;
    const data = chart;
    if (!c || !data) return;
    const ctx = c.getContext("2d");
    if (!ctx) return;
    const dpr = window.devicePixelRatio || 1;
    const w = c.clientWidth;
    const h = c.clientHeight;
    c.width = w * dpr;
    c.height = h * dpr;
    ctx.scale(dpr, dpr);
    ctx.clearRect(0, 0, w, h);
    const fg = getComputedStyle(c).color;
    ctx.fillStyle = fg;
    ctx.font = "11px sans-serif";
    const n = data.labels.length;
    if (!n) {
      ctx.fillText("Nothing to draw", 8, 20);
      return;
    }
    if (kind === "pie") {
      const vals = (data.series[0] ?? []).map((v) => Math.max(0, v ?? 0));
      const total = vals.reduce((a, b) => a + b, 0) || 1;
      const r = Math.min(w, h) / 2 - 8;
      let a = -Math.PI / 2;
      vals.forEach((v, i) => {
        const b = a + (v / total) * Math.PI * 2;
        ctx.beginPath();
        ctx.moveTo(w / 2, h / 2);
        ctx.arc(w / 2, h / 2, r, a, b);
        ctx.fillStyle = COLORS[i % COLORS.length]!;
        ctx.fill();
        a = b;
      });
      return;
    }
    const all = data.series.flat().filter((v): v is number => v !== null);
    let lo = Math.min(0, ...all);
    let hi = Math.max(0, ...all);
    if (hi === lo) hi = lo + 1;
    const left = 40;
    const top = 8;
    const ph = h - 28;
    const pw = w - left - 8;
    const y = (v: number) => top + ph - ((v - lo) / (hi - lo)) * ph;
    ctx.fillText(fmt(hi), 2, top + 8);
    ctx.fillText(fmt(lo), 2, top + ph);
    ctx.globalAlpha = 0.3;
    ctx.fillRect(left, y(0), pw, 1);
    ctx.globalAlpha = 1;
    data.series.forEach((s, k) => {
      ctx.strokeStyle = ctx.fillStyle = COLORS[k % COLORS.length]!;
      if (kind === "line") {
        ctx.beginPath();
        let started = false;
        s.forEach((v, i) => {
          if (v === null) return;
          const x = left + (n === 1 ? pw / 2 : (i / (n - 1)) * pw);
          if (started) ctx.lineTo(x, y(v));
          else ctx.moveTo(x, y(v));
          started = true;
        });
        ctx.stroke();
      } else {
        const bw = Math.max(1, pw / n / data.series.length);
        s.forEach((v, i) => {
          if (v === null) return;
          const x = left + (i / n) * pw + k * bw;
          ctx.fillRect(x, Math.min(y(v), y(0)), Math.max(1, bw - 1), Math.abs(y(v) - y(0)));
        });
      }
    });
    ctx.fillStyle = fg;
    ctx.fillText(data.labels[0] ?? "", left, h - 6);
    const last = data.labels[n - 1] ?? "";
    ctx.fillText(last, w - 8 - ctx.measureText(last).width, h - 6);
  });

  // ---- files ----
  let listing = $state<Listing | null>(null);
  $effect(() => {
    if (which !== "files" || !tab) return;
    const t = tab;
    if (!t.info.path) {
      listing = null;
      return;
    }
    backend()
      .folderList(t.id)
      .then((l) => (listing = l))
      .catch((e: unknown) => (problem = (e as { message?: string }).message ?? String(e)));
  });
  async function openListed(i: number): Promise<void> {
    if (!tab) return;
    try {
      onOpened(await backend().folderOpen(tab.id, i));
    } catch (e) {
      problem = (e as { message?: string }).message ?? String(e);
    }
  }
  const size = (n: number) => (n < 1024 ? `${n} B` : n < 1 << 20 ? `${(n / 1024).toFixed(0)} KB` : n < 1 << 30 ? `${(n / (1 << 20)).toFixed(1)} MB` : `${(n / (1 << 30)).toFixed(2)} GB`);
</script>

<aside class="panel" aria-label="Side panel">
  <div class="tabs" role="tablist">
    {#each [["summary", "Summary"], ["chart", "Chart"], ["files", "Files"], ["activity", "Activity"]] as [id, name] (id)}
      <button type="button" role="tab" aria-selected={which === id} class:on={which === id} onclick={() => (which = id as PanelTab)}>{name}</button>
    {/each}
    <span class="spacer"></span>
    <button type="button" class="close" aria-label="Close the panel" onclick={onClose}>✕</button>
  </div>
  <div class="content">
    {#if !tab}
      <p class="note">Open a file to see its summary and charts.</p>
    {:else if which === "summary"}
      <h3>{titles[col] ?? `Column ${col + 1}`}</h3>
      {#if summary && summary.col === col}
        <dl>
          <dt>Holds</dt><dd>{TYPE_NAMES[summary.kind]}</dd>
          <dt>Rows</dt><dd>{summary.rows.toLocaleString()}</dd>
          <dt>Blank</dt><dd>{summary.mix.blank.toLocaleString()}</dd>
          <dt>Distinct</dt><dd>{summary.distinct_capped ? "more than " : ""}{summary.distinct.toLocaleString()}</dd>
          <dt>Numbers</dt><dd>{summary.mix.number.toLocaleString()}</dd>
          <dt>Dates</dt><dd>{summary.mix.date.toLocaleString()}</dd>
          <dt>Text</dt><dd>{summary.mix.text.toLocaleString()}</dd>
          {#if summary.sum !== null}
            <dt>Sum</dt><dd>{fmt(summary.sum)}</dd>
            <dt>Average</dt><dd>{fmt(summary.mean)}</dd>
            <dt>Smallest</dt><dd>{fmt(summary.min)}</dd>
            <dt>Largest</dt><dd>{fmt(summary.max)}</dd>
          {/if}
          <dt>Length</dt><dd>{summary.min_len}–{summary.max_len} characters</dd>
        </dl>
        <h4>Most common</h4>
        <ol class="top">
          {#each summary.top as v, i (i)}
            <li><button type="button" title="Show only rows with this value" onclick={() => onFilterValue(col, v.value, v.cut)}><span class="v">{v.value === "" ? "(blank)" : v.value}</span><span class="n">{v.count.toLocaleString()}</span></button></li>
          {/each}
        </ol>
        {#if summary.top_partial}
          <p class="note">This column has more than {summary.distinct.toLocaleString()} different values, so this list holds the most common ones found. Their counts are exact; values that appear only a few times may be missing.</p>
        {/if}
        {#if summary.top.length === 0 && summary.rows > 0}
          <p class="note">No value appears often enough to list.</p>
        {/if}
        <p class="note">Shown rows only. Click a value to show only its rows.</p>
      {:else}
        <p class="note">Working…</p>
      {/if}
      {#if types.length}
        <h4>Every column</h4>
        <dl class="types">
          {#each types.slice(0, 500) as k, c (c)}
            <dt>{titles[c] ?? `Column ${c + 1}`}</dt><dd>{TYPE_NAMES[k]}</dd>
          {/each}
        </dl>
        <p class="note">Guessed from a sample of rows.</p>
      {/if}
    {:else if which === "chart"}
      <div class="row">
        <select bind:value={kind} aria-label="Chart kind">
          <option value="bar">Bars</option>
          <option value="line">Line</option>
          <option value="pie">Pie</option>
          <option value="count">Count of values</option>
        </select>
        <button type="button" class="primary" onclick={() => void drawChart()}>Draw</button>
      </div>
      <label class="row">{kind === "count" ? "Count the values of" : "Labels from"}
        <select value={label === null ? "" : String(label)} onchange={(e) => { const v = (e.currentTarget as HTMLSelectElement).value; label = v === "" ? null : Number(v); }}>
          {#if kind !== "count"}<option value="">Row numbers</option>{/if}
          {#each titles as t, c (c)}<option value={String(c)}>{t}</option>{/each}
        </select>
      </label>
      {#if kind !== "count"}
        <AllNone count={Math.min(titles.length, 200)} picked={values} onChange={(v) => (values = v)} max={kind === "pie" ? 1 : 8} />
        {#if !values.length}<p class="note">Tick the columns to chart.</p>{/if}
        <div class="values">
          {#each titles.slice(0, 200) as t, c (c)}
            <label><input type="checkbox" checked={values.includes(c)} onchange={() => (values = values.includes(c) ? values.filter((x) => x !== c) : [...values, c].slice(0, kind === "pie" ? 1 : 8))} /> {t}</label>
          {/each}
        </div>
      {/if}
      <canvas bind:this={canvas} class="chart" aria-label="Chart"></canvas>
      {#if chart}
        <p class="note">{chart.rows.toLocaleString()} rows{chart.per_point > 1 ? `, ${chart.per_point.toLocaleString()} per point (averaged)` : ""}{chart.other ? "; smaller values grouped as Other" : ""}.</p>
        {#if kind === "pie" || kind === "count"}
          <ol class="legend">
            {#each chart.labels.slice(0, 12) as l, i (i)}<li><span class="sw" style:background={COLORS[i % COLORS.length]}></span>{l === "" ? "(blank)" : l}: {fmt(chart.series[0]?.[i] ?? null)}</li>{/each}
          </ol>
        {/if}
      {/if}
    {:else if which === "files"}
      {#if !tab.info.path}
        <p class="note">This table has no file yet. Save it to see its folder.</p>
      {:else if listing}
        <h3>{listing.folder}</h3>
        <ul class="files">
          {#each listing.items as f (f.index)}
            <li><button type="button" disabled={!f.table} ondblclick={() => void openListed(f.index)} onkeydown={(e) => { if (e.key === "Enter") void openListed(f.index); }} title={f.table ? "Double-click to open" : "Not a table"}><span class="v">{f.name}</span><span class="n">{size(f.size)}</span></button></li>
          {/each}
        </ul>
        {#if listing.more}<p class="note">Only the first 2,000 files are listed.</p>{/if}
      {/if}
    {:else}
      <ol class="activity">
        {#each [...activity].reverse() as a, i (i)}<li><span class="time">{a.time}</span> {a.text}</li>{:else}<li class="note">Nothing yet.</li>{/each}
      </ol>
    {/if}
    {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  </div>
</aside>

<style>
  .panel {
    width: 20rem;
    flex: none;
    display: flex;
    flex-direction: column;
    border-left: 1px solid var(--line);
    background: var(--bg);
    min-height: 0;
  }
  .tabs {
    display: flex;
    gap: 2px;
    padding: 4px;
    border-bottom: 1px solid var(--line);
  }
  .tabs button {
    background: transparent;
    border: 1px solid transparent;
    padding: 2px 8px;
    border-radius: 4px;
  }
  .tabs button.on {
    border-color: var(--accent);
  }
  .spacer {
    flex: 1;
  }
  .content {
    overflow: auto;
    padding: 8px;
    flex: 1;
    min-height: 0;
  }
  h3 {
    margin: 0 0 6px;
    font-size: 1rem;
    overflow-wrap: anywhere;
  }
  h4 {
    margin: 10px 0 4px;
    font-size: 0.9rem;
  }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 2px 10px;
    margin: 0;
  }
  dt {
    color: var(--muted);
  }
  dd {
    margin: 0;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  ol,
  ul {
    list-style: none;
    padding: 0;
    margin: 0;
  }
  .top button,
  .files button {
    display: flex;
    width: 100%;
    gap: 8px;
    background: transparent;
    border: none;
    padding: 2px 4px;
    text-align: left;
  }
  .top button:hover,
  .files button:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .v {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .n {
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }
  .row {
    display: flex;
    gap: 6px;
    align-items: center;
    margin-bottom: 6px;
  }
  .values {
    max-height: 7rem;
    overflow: auto;
    display: flex;
    flex-direction: column;
    margin-bottom: 6px;
  }
  .chart {
    width: 100%;
    height: 14rem;
    color: var(--fg);
  }
  .legend li {
    display: flex;
    gap: 6px;
    align-items: center;
    font-size: 0.85rem;
  }
  .sw {
    width: 10px;
    height: 10px;
    border-radius: 2px;
    flex: none;
  }
  .activity li {
    font-size: 0.85rem;
    padding: 2px 0;
    border-bottom: 1px solid var(--line);
    overflow-wrap: anywhere;
  }
  .time {
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }
  .note {
    color: var(--muted);
    font-size: 0.85rem;
  }
  .problem {
    color: var(--danger, #c00);
  }
</style>
