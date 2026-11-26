<script lang="ts">
  // Data › Find gaps in a sequence: pick the column of numbers or IDs, see
  // the missing ranges, then show the rows beside them, copy the list or open
  // it as a tab.
  import { onMount, untrack } from "svelte";
  import AllNone from "./AllNone.svelte";
  import FormShell from "./FormShell.svelte";
  import { copyText, counted, fileRows, grouped, rangeText, type DateStep, type GapPattern, type GapReport, type GapsSpec } from "./gaps";

  /** Ranges listed in the dialog (the tab has the rest). */
  const LISTED = 500;
  /** Smaller tables are audited as soon as the column is picked. */
  const AT_ONCE = 2_000_000;

  interface Props {
    titles: string[];
    hidden: number[];
    col: number;
    header: boolean;
    rows: number;
    onRun: (spec: GapsSpec) => Promise<GapReport | null>;
    onCancelRun: () => void;
    onShowRows: (spec: GapsSpec) => void;
    onOpenTab: (spec: GapsSpec, save: boolean) => void;
    onCopy: (text: string) => void;
    onClose: () => void;
  }
  let { titles, hidden, col, header, rows, onRun, onCancelRun, onShowRows, onOpenTab, onCopy, onClose }: Props = $props();
  const init = untrack(() => ({ col, header, rows }));
  let column = $state(init.col);
  let series = $state<number[]>([]);
  let run = $state(-1);
  let shape = $state(-1);
  let eachShape = $state(false);
  let kind = $state<"numbers" | "dates">("numbers");
  let step = $state<DateStep["step"]>("day");
  let every = $state(2);
  const dated = $derived(kind === "dates");
  let start = $state("");
  let end = $state("");
  let busy = $state(false);
  let report = $state<GapReport | null>(null);
  /** The spec the report shown was made with, as text. */
  let ran = $state("");
  let shapes = $state<GapReport["patterns"]>([]);
  let shapeTotal = $state(0);

  const shown = $derived(titles.map((t, i) => [i, t] as const).filter(([i]) => !hidden.includes(i)));
  const picked = $derived<GapPattern | null>(!dated && !eachShape && shape >= 0 ? (shapes[shape]?.pattern ?? null) : null);
  const spec = $derived<GapsSpec>({
    col: column,
    series: series.filter((c) => c !== column).sort((a, b) => a - b),
    keep_first: init.header,
    run: dated || run < 0 ? null : run,
    pattern: picked,
    each_shape: !dated && eachShape,
    dates: !dated ? null : step === "days" ? { step, n: Math.trunc(every) || 0 } : { step },
    start: start.trim(),
    end: end.trim(),
  });
  const stale = $derived(!report || ran !== JSON.stringify(spec));

  async function find(): Promise<void> {
    if (busy) return;
    const s = $state.snapshot(spec) as GapsSpec;
    busy = true;
    try {
      const r = await onRun(s);
      if (!r) return;
      report = r;
      ran = JSON.stringify(s);
      // The shapes belong to the column and digit run, not to the one picked.
      shapes = r.patterns;
      shapeTotal = r.pattern_total;
      if (s.pattern) shape = r.patterns.findIndex((p) => JSON.stringify(p.pattern) === JSON.stringify(s.pattern));
    } finally {
      busy = false;
    }
  }

  /** A new column or digit run has its own shapes. */
  function reset(): void {
    shape = -1;
    shapes = [];
    shapeTotal = 0;
    report = null;
    if (init.rows <= AT_ONCE) void find();
  }

  onMount(() => {
    if (init.rows <= AT_ONCE) void find();
  });

  function toggle(c: number, on: boolean): void {
    series = on ? [...series, c].slice(-2) : series.filter((x) => x !== c);
  }
  function close(): void {
    if (busy) onCancelRun();
    onClose();
  }
</script>

<FormShell title="Find gaps in a sequence" wide onClose={close} onEnter={() => void find()}>
  <fieldset class="kind">
    <legend>The column holds</legend>
    <label><input type="radio" name="gap-kind" value="numbers" bind:group={kind} disabled={busy} onchange={reset} /> Numbers or IDs</label>
    <label><input type="radio" name="gap-kind" value="dates" bind:group={kind} disabled={busy} onchange={reset} /> Dates</label>
  </fieldset>
  <div class="pick">
    <label
      >Column
      <select bind:value={column} onchange={reset} disabled={busy}>
        {#each shown as [i, t] (i)}<option value={i}>{t}</option>{/each}
      </select></label
    >
    {#if dated}
      <label
        >Expected every
        <select bind:value={step} disabled={busy}>
          <option value="day">Day</option>
          <option value="business">Business day (Monday to Friday, no holidays)</option>
          <option value="week">Week</option>
          <option value="month">Month</option>
          <option value="days">Number of days</option>
        </select></label
      >
      {#if step === "days"}<label>Days <input type="number" min="1" max="3660" bind:value={every} disabled={busy} /></label>{/if}
    {:else}
    <label
      >Digits to follow
      <select bind:value={run} onchange={reset} disabled={busy}>
        <option value={-1}>Chosen for me</option>
        <option value={0}>Last run of digits</option>
        <option value={1}>2nd from the end</option>
        <option value={2}>3rd from the end</option>
      </select></label
    >
    <label
      >Shape
      <select bind:value={shape} disabled={busy || eachShape || !shapes.length}>
        <option value={-1}>The most common</option>
        {#each shapes as p, i (i)}<option value={i}>{p.label} ({counted(p.count, "row")})</option>{/each}
      </select></label
    >
    {/if}
  </div>
  {#if shapeTotal > shapes.length}<p class="muted">{shapeTotal.toLocaleString()} shapes in all; the {shapes.length} most common are listed.</p>{/if}
  {#if !dated}<label><input type="checkbox" bind:checked={eachShape} disabled={busy} /> Audit each shape on its own (such as one sequence per year in A-17-0042)</label>{/if}
  <div class="pick">
    <label>Start at (optional) <input type="text" bind:value={start} disabled={busy} placeholder={dated ? (step === "month" ? "2026-03" : "2026-03-31") : ""} /></label>
    <label>End at (optional) <input type="text" bind:value={end} disabled={busy} /></label>
  </div>
  <details>
    <summary>Series by (up to 2 columns): {series.length ? series.map((c) => titles[c]).join(", ") : "none"}</summary>
    <AllNone count={titles.length} picked={series} max={2} skip={(c) => c === column || hidden.includes(c)} onChange={(v) => (series = v)} />
    <div class="cols">
      {#each shown.filter(([i]) => i !== column) as [i, t] (i)}
        <label><input type="checkbox" checked={series.includes(i)} disabled={busy} onchange={(e) => toggle(i, (e.currentTarget as HTMLInputElement).checked)} /> {t}</label>
      {/each}
    </div>
  </details>
  <p class="muted">Shown rows only{init.header ? "; the title row is left out" : ""}.</p>

  {#if busy}
    <p role="status">Looking for gaps… <button type="button" onclick={onCancelRun}>Stop</button></p>
  {:else if report}
    {@const r = report}
    <section aria-label="Gaps found" class:old={stale}>
      {#if stale}<p class="warn" role="note">Something changed: press Find gaps again.</p>{/if}
      {#if r.too_many_shapes}<p class="warn" role="note">Too many shapes: pick the digit run to follow. Rows of the shapes left over count as a different shape.</p>{/if}
      {#if r.looks_like_amounts}<p class="warn" role="note">This column looks like amounts, not IDs.</p>{/if}
      <p role="status">
        <strong>{grouped(r.missing)} missing</strong> in {r.range_total.toLocaleString()}
        {r.range_total === 1 ? "range" : "ranges"}.
        {#if r.series.length === 1 && r.series[0]!.first}From {r.series[0]!.first} to {r.series[0]!.last}.{:else if r.series_total > 1}{r.series_total.toLocaleString()} series.{/if}
        {counted(r.rows, "row")}, {counted(r.distinct, r.dates ? "different step" : "different number")}.
      </p>
      <ul class="facts">
        <li>{counted(r.duplicate_values, !r.dates ? "repeated number" : step === "month" ? "month with more than one row" : "repeated date", !r.dates ? undefined : step === "month" ? "months with more than one row" : undefined)} ({counted(r.duplicate_extra, "extra row")})</li>
        <li>{counted(r.drops, "drop")} in file order{fileRows(r.drop_rows, r.drops > r.drop_rows.length)}</li>
        <li>{counted(r.other_shape, "row")} of a different shape{fileRows(r.other_rows, r.other_shape > r.other_rows.length)}</li>
        <li>{r.blank.toLocaleString()} blank</li>
        {#if r.dates}
          {@const d = r.dates}
          {#if d.ambiguous}<li class="warn">{counted(d.ambiguous, "ambiguous date")} not read{fileRows(d.ambiguous_rows, d.ambiguous > d.ambiguous_rows.length)}: some gaps listed may be these. Set the column's date order to read them.</li>{/if}
          {#if d.two_digit}<li>{counted(d.two_digit, "two-digit year")}{fileRows(d.two_digit_rows, d.two_digit > d.two_digit_rows.length)}: 00 to 29 are read as 20xx, 30 to 99 as 19xx</li>{/if}
          {#if d.weekend}<li>{d.weekend.toLocaleString()} on a weekend, outside the sequence{fileRows(d.weekend_rows, d.weekend > d.weekend_rows.length)}</li>{/if}
          {#if d.off_step}<li>{d.off_step.toLocaleString()} off the step, never rounded{fileRows(d.off_step_rows, d.off_step > d.off_step_rows.length)}</li>{/if}
          <li>A date with a time counts on its day.</li>
        {/if}
        {#if r.added}<li>{counted(r.added, "row")} added by hand, left out of the order check</li>{/if}
        {#if r.outside}<li>{r.outside.toLocaleString()} outside the start and end</li>{/if}
        {#if r.series.some((s) => s.too_few)}<li>A series with fewer than 2 rows is too few to check.</li>{/if}
      </ul>
      {#if r.ranges.length}
        <ol class="ranges" aria-label="Missing ranges">
          {#each r.ranges.slice(0, LISTED) as g, i (i)}<li>{rangeText(g)}</li>{/each}
        </ol>
        {#if r.range_total > LISTED}<p class="muted">The first {LISTED} are listed here. Open as a tab for up to {r.ranges.length.toLocaleString()}.</p>{/if}
      {/if}
      {#if r.duplicates.length}
        <details>
          <summary>{r.dates ? "Repeats" : "Repeated numbers"}</summary>
          <ul class="ranges">
            {#each r.duplicates.slice(0, LISTED) as d, i (i)}<li>{d.series.length ? `${d.series.join(" / ")}: ` : ""}{d.value}: {counted(d.count, "row")}{fileRows(d.rows, d.count > d.rows.length)}</li>{/each}
          </ul>
        </details>
      {/if}
    </section>
  {/if}
  {#snippet buttons()}
    <button type="button" onclick={close}>Close</button>
    <button type="button" disabled={busy || stale || !report?.range_total} onclick={() => onShowRows(spec)}>Show the rows next to each gap</button>
    <button type="button" disabled={busy || stale || !report?.range_total} onclick={() => report && onCopy(copyText(report))}>Copy missing list</button>
    <button type="button" disabled={busy || stale} onclick={() => onOpenTab(spec, false)}>Open as a tab</button>
    <button type="button" disabled={busy || stale} onclick={() => onOpenTab(spec, true)}>Save as CSV…</button>
    <button type="button" class="primary" disabled={busy} onclick={() => void find()}>Find gaps</button>
  {/snippet}
</FormShell>

<style>
  .kind {
    border: none;
    padding: 0;
    margin: 0 0 8px;
    display: flex;
    gap: 14px;
    align-items: baseline;
  }
  .kind legend {
    float: left;
    margin-right: 10px;
    font-weight: 600;
  }
  .pick {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
    margin: 0 0 8px;
  }
  .pick select {
    max-width: 220px;
  }
  .cols {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 14px;
    max-height: 130px;
    overflow: auto;
  }
  .ranges {
    max-height: 200px;
    overflow: auto;
    margin: 6px 0;
    font-variant-numeric: tabular-nums;
  }
  .facts {
    margin: 4px 0;
    font-size: 13px;
  }
  .old {
    opacity: 0.6;
  }
  .muted {
    color: var(--muted);
    font-size: 12px;
  }
  .warn {
    color: var(--danger, #c0392b);
  }
</style>
