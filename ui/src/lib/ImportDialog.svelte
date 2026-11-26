<script lang="ts">
  // Import: a spreadsheet (sheet choice), JSON, XML, a web page table, a
  // Markdown table, delimited text, fixed-width text (column edges on a
  // ruler) or a log file (lines cut by a pattern), from a file or the
  // clipboard, with a preview of the first rows. The result opens as a new,
  // unsaved tab.
  import { untrack } from "svelte";
  import { backend, errorText, type ImportFormat, type ImportInfo, type ImportNotes, type ImportOptions, type ImportStart } from "./backend";
  import { recallPad, rememberPad } from "./headerMemory";
  import FormShell from "./FormShell.svelte";
  import { looksLikeHeader } from "./header";
  import { FORMAT_NAMES } from "./transfer";
  import { LOG_PRESETS, toggleStart } from "./m8";
  import { DBF_ENCODINGS, headerChoice, importHeader, previewLine } from "./dbf";

  interface Props {
    start: ImportStart;
    onImport: (o: ImportOptions, header: boolean, notes: ImportNotes | null) => void;
    onClose: () => void;
  }
  let { start, onImport, onClose }: Props = $props();

  let info = $state<ImportInfo>(untrack(() => start.info));
  let o = $state<ImportOptions>(untrack(() => withDefaults({ format: start.info.format, part: firstVisible(start.info), raw_dates: false, record: "" }, start.info)));
  let rows = $state<string[][]>([]);
  let problem = $state("");
  let loading = $state(false);
  let header = $state(true);
  let headerTouched = false;

  /** Fixed width and log files start from what was guessed. */
  function withDefaults(x: ImportOptions, i: ImportInfo): ImportOptions {
    return { ...x, starts: i.starts ?? [], keep_spaces: false, pattern: i.pattern ?? "", unmatched: "join", dbf: { encoding: "", fid: "from0", show_deleted: false } };
  }

  function firstVisible(i: ImportInfo): number {
    const k = i.hidden.findIndex((h) => !h);
    return k < 0 ? 0 : k;
  }

  async function changeFormat(f: ImportFormat): Promise<void> {
    try {
      info = await backend().importInspect(start.id, f);
      o = withDefaults({ format: f, part: firstVisible(info), raw_dates: false, record: "" }, info);
      problem = "";
    } catch (e) {
      problem = errorText(e);
    }
  }

  let seq = 0;
  $effect(() => {
    const snap = $state.snapshot(o);
    const mine = ++seq;
    loading = true;
    backend()
      .importPreview(start.id, snap)
      .then((r) => {
        if (mine !== seq) return;
        rows = r;
        problem = "";
        // A dBase table's first row is always its field names.
        if (!headerTouched) header = info.format === "dbf" || looksLikeHeader(r.slice(0, 20));
      })
      .catch((e: unknown) => {
        if (mine !== seq) return;
        rows = [];
        problem = errorText(e);
      })
      .finally(() => {
        if (mine === seq) loading = false;
      });
  });

  // What the "exactly as written" guards did (counts and column names only), and the one question they ask.
  let notes = $state<ImportNotes | null>(null);
  let nseq = 0;
  let padRecalled = false;
  $effect(() => {
    if (info.format !== "spreadsheet" && info.format !== "dbf") {
      notes = null;
      return;
    }
    const snap = $state.snapshot(o);
    const mine = ++nseq;
    backend()
      .importNotes(start.id, snap)
      .then(async (n) => {
        if (mine !== nseq) return;
        notes = n;
        if (!padRecalled && n.remember_path && n.padded.length > 0) {
          padRecalled = true;
          const r = await recallPad(n.remember_path);
          if (r !== null && o.pad_zeros == null) o.pad_zeros = r;
        }
      })
      .catch(() => {
        if (mine === nseq) notes = null;
      });
  });
  const paddedCells = $derived((notes?.padded ?? []).reduce((a, p) => a + p.count, 0));

  const width = $derived(Math.min(50, Math.max(0, ...rows.map((r) => r.length))));
  const partLabel = $derived(info.format === "spreadsheet" ? "Sheet" : "Table");

  // ---- fixed-width ruler ----
  const lines = $derived(info.lines ?? []);
  const longest = $derived(Math.max(10, ...lines.map((l) => Array.from(l).length)));
  let charW = $state(8);
  let measure = $state<HTMLSpanElement | null>(null);
  $effect(() => {
    if (measure) charW = measure.getBoundingClientRect().width / 10 || 8;
  });
  function rulerClick(e: MouseEvent): void {
    const box = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const at = Math.round((e.clientX - box.left + (e.currentTarget as HTMLElement).scrollLeft) / charW);
    o.starts = toggleStart(o.starts ?? [], Math.min(at, longest));
  }
  const ticks = $derived(Array.from({ length: Math.floor(longest / 10) + 1 }, (_, k) => k * 10));
  let preset = $state("");

  function run(): void {
    if (problem || loading || rows.length === 0) return;
    if (notes?.remember_path && notes.padded.length > 0) void rememberPad(notes.remember_path, !!o.pad_zeros);
    onImport($state.snapshot(o), importHeader(info.format, header), $state.snapshot(notes));
  }
</script>

<FormShell title={`Import ${start.name}`} wide {onClose} onEnter={run}>
  {#if start.note}<p class="note" data-testid="import-why">{start.note}</p>{/if}
  <div class="grid2">
    <label class="row">Read as
      <select value={o.format} onchange={(e) => void changeFormat((e.currentTarget as HTMLSelectElement).value as ImportFormat)}>
        {#each Object.entries(FORMAT_NAMES) as [v, label] (v)}<option value={v}>{label}</option>{/each}
      </select>
    </label>
    {#if info.parts.length > 0}
      <label class="row">{partLabel}
        <select bind:value={o.part}>
          {#each info.parts as p, i (i)}<option value={i}>{p}{info.hidden[i] ? " (hidden)" : ""}</option>{/each}
        </select>
      </label>
    {/if}
    {#if info.format === "spreadsheet"}
      <label class="row"><input type="checkbox" bind:checked={o.raw_dates} /> Keep dates as the numbers they are stored as</label>
      {#if notes && notes.padded.length > 0}
        <label class="row">
          <input type="checkbox" checked={!!o.pad_zeros} onchange={(e) => (o.pad_zeros = e.currentTarget.checked)} />
          Write numbers with leading zeros the way the spreadsheet shows them ({paddedCells} {paddedCells === 1 ? "cell" : "cells"} in {notes.padded.map((p) => p.name).slice(0, 3).join(", ")})
        </label>
      {/if}
    {/if}
    {#if info.format === "dbf" && o.dbf}
      <label class="row">Text encoding
        <select bind:value={o.dbf.encoding}>
          {#each DBF_ENCODINGS as [v, label] (v)}<option value={v}>{label}</option>{/each}
        </select>
      </label>
      <label class="row">Record number column (FID)
        <select bind:value={o.dbf.fid}>
          <option value="from0">from 0, as GIS programs number shapes</option>
          <option value="from1">from 1</option>
          <option value="off">none</option>
        </select>
      </label>
      <label class="row"><input type="checkbox" bind:checked={o.dbf.show_deleted} /> Show records marked as deleted</label>
    {/if}
    {#if info.format === "xml" && info.records.length > 0}
      <label class="row">Each row is a
        <select bind:value={o.record}>
          <option value="">{info.records[0]} (most repeated)</option>
          {#each info.records.slice(1) as r (r)}<option value={r}>{r}</option>{/each}
        </select>
      </label>
    {/if}
    {#if info.format === "fixed_width"}
      <label class="row"><input type="checkbox" bind:checked={o.keep_spaces} /> Keep the spaces around values</label>
    {/if}
    {#if info.format === "pattern"}
      <label class="row">Ready-made
        <select bind:value={preset} onchange={() => { if (preset) o.pattern = preset; }}>
          <option value="">Choose a kind of log…</option>
          {#each LOG_PRESETS as p (p.name)}<option value={p.pattern}>{p.name}</option>{/each}
        </select>
      </label>
      <label class="row">Lines that do not fit
        <select bind:value={o.unmatched}>
          <option value="join">join the row above (stack traces)</option>
          <option value="row">become rows of their own</option>
          <option value="skip">are left out</option>
        </select>
      </label>
    {/if}
    {#if headerChoice(info.format)}
      <label class="row"><input type="checkbox" bind:checked={header} onchange={() => (headerTouched = true)} /> First row is header</label>
    {/if}
  </div>
  {#if info.format === "pattern"}
    <label class="wide">Pattern for a line (a regular expression; each group in brackets is a column, <code>(?P&lt;name&gt;…)</code> names it)
      <input class="mono" type="text" bind:value={o.pattern} spellcheck="false" />
    </label>
  {/if}
  {#if info.format === "fixed_width" && lines.length > 0}
    <p class="note">Click between characters to add a column edge; click an edge to take it away. Or type where columns start (characters, from 1):</p>
    <input type="text" aria-label="Column starts" value={(o.starts ?? []).filter((x) => x > 0).map((x) => x + 1).join(", ")}
      onchange={(e) => (o.starts = [0, ...e.currentTarget.value.split(/[,\s]+/).map(Number).filter((n) => Number.isInteger(n) && n > 1).map((n) => n - 1)].sort((a, b) => a - b))} />
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
    <div class="ruler" role="group" aria-label="Column edges" onclick={rulerClick}>
      <span class="measure" bind:this={measure}>0000000000</span>
      <div class="ticks" style:width={`${longest * charW}px`}>
        {#each ticks as t (t)}<span style:left={`${t * charW}px`}>{t}</span>{/each}
      </div>
      <pre style:width={`${longest * charW}px`}>{lines.join("\n")}</pre>
      {#each o.starts ?? [] as s (s)}
        {#if s > 0}<div class="edge" style:left={`${s * charW}px`} title={`Column starts at character ${s + 1}`}></div>{/if}
      {/each}
    </div>
  {/if}
  {#if info.format === "spreadsheet"}
    <p class="note">Values are imported as they were last calculated; formulas, formatting and pictures are left out.</p>
    {#if notes && notes.summary}
      <p class="note" data-testid="import-notes">{notes.summary}</p>
    {/if}
  {/if}
  {#if info.format === "dbf" && notes && notes.summary}
    <p class="note" data-testid="import-notes">{notes.summary}</p>
  {/if}
  <div class="table-preview" aria-label="Preview of the first rows">
    <table>
      <tbody>
        {#each rows.slice(0, 100) as r, i (i)}
          <tr class:head={i === 0 && importHeader(info.format, header)}>
            {#each Array.from({ length: width }, (_, c) => r[c] ?? "") as v, c (c)}<td>{v}</td>{/each}
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
  <p class="note">{loading ? "Reading…" : previewLine(info.format, rows.length, notes?.records)} The import opens as a new tab, not saved yet.</p>
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" class="primary" onclick={run} disabled={!!problem || loading || rows.length === 0}>Import</button>
    <button type="button" onclick={onClose}>Cancel</button>
  {/snippet}
</FormShell>

<style>
  .ruler {
    position: relative;
    overflow: auto;
    max-height: 14em;
    border: 1px solid var(--border);
    background: var(--bg);
    cursor: crosshair;
    font: 13px/1.35 ui-monospace, Consolas, monospace;
  }
  .ruler pre {
    margin: 0;
    padding: 0;
    font: inherit;
    white-space: pre;
  }
  .ticks {
    position: relative;
    height: 1.2em;
    border-bottom: 1px solid var(--border);
    color: var(--muted);
    font-size: 11px;
  }
  .ticks span {
    position: absolute;
    top: 0;
  }
  .edge {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 0;
    border-left: 2px solid var(--accent);
    pointer-events: none;
  }
  .measure {
    position: absolute;
    visibility: hidden;
    white-space: pre;
  }
  .mono {
    font-family: ui-monospace, Consolas, monospace;
  }
  .wide {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
</style>
