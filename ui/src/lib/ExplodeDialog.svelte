<script lang="ts">
  // Data › Explode cells: a column whose cells pack several fields (a JSON
  // object, a JSON list, a delimited list, key=value pairs) becomes one column
  // per key, one column per item, one flag column per item, or one row per
  // item. Before Run it shows what it found, the first 20 rows before and
  // after, the caps, and the cells that did not parse.
  import { untrack } from "svelte";
  import { backend, errorText, isCmdError, type Explode, type ExplodeDetect, type ExplodeHow, type ExplodeKind, type ExplodePreview, type ExplodeScan } from "./backend";
  import { EXPLODE_BASE } from "./explode";
  import FormShell from "./FormShell.svelte";
  import HiddenNote from "./HiddenNote.svelte";

  interface Props {
    doc: number;
    titles: string[];
    /** Hidden columns are not offered. */
    hidden: number[];
    col: number;
    header: boolean;
    readOnly: boolean;
    onRun: (spec: Explode) => void;
    onShowBad: (spec: Explode) => void;
    onClose: () => void;
  }
  let { doc, titles, hidden, col, header, readOnly, onRun, onShowBad, onClose }: Props = $props();
  const init = untrack(() => ({ col, header }));
  const MAX_COLS = 1000;
  const MAX_ROWS = 10_000_000;
  const SCAN = "Reading the cells";

  let at = $state(init.col);
  let kind = $state<ExplodeKind>("object");
  let how = $state<ExplodeHow>("columns");
  let separator = $state("|");
  let kv = $state("=");
  let depth = $state(1);
  let keepOriginal = $state(true);
  let trim = $state(true);
  let nullBlank = $state(true);
  let originalCol = $state(false);
  let mark = $state("1");
  let matchCase = $state(false);
  let skipEmpty = $state(true);
  let addNew = $state(false);
  let detect = $state<ExplodeDetect | null>(null);
  let scan = $state<ExplodeScan | null>(null);
  let scanning = $state(false);
  let scanProgress = $state<number | null>(null);
  let problem = $state("");
  let preview = $state<ExplodePreview | null>(null);
  /** Keys (or tags) unticked by the person. */
  let unticked = $state(new Set<string>());

  const hasKeys = $derived(kind === "object" || kind === "key_value");
  const realHow = $derived<ExplodeHow>(hasKeys ? "columns" : how);
  const listed = $derived(hasKeys || realHow === "flags");
  const ticked = $derived(scan ? scan.keys.map((k) => k.key).filter((k) => !unticked.has(k)) : []);
  const shownCols = $derived(titles.map((t, i) => [i, t] as const).filter(([i]) => !hidden.includes(i)));

  function specOf(keys: string[] | null): Explode {
    return {
      ...EXPLODE_BASE,
      col: at,
      keep_first: init.header,
      kind,
      how: realHow,
      separator,
      kv,
      depth,
      keys,
      add_new_keys: addNew,
      keep_original: keepOriginal,
      trim,
      mark,
      null_blank: nullBlank,
      original_col: originalCol,
      match_case: matchCase,
      skip_empty: skipEmpty,
    };
  }
  /** What Run sends: the ticked keys (or tags), recorded by a recipe. */
  const spec = $derived(specOf(listed ? ticked : null));

  // What the column looks like: on opening and on another column.
  let detected = -1;
  $effect(() => {
    const c = at;
    void backend()
      .explodeDetect(doc, c, init.header)
      .then((d) => {
        if (c !== at) return;
        detect = d;
        if (detected !== c) {
          detected = c;
          kind = d.kind;
          if (d.kind === "list" || d.kind === "key_value") separator = d.separator;
          if (d.kind === "key_value") kv = d.kv;
          how = "columns";
        }
      })
      .catch((e: unknown) => (problem = errorText(e)));
  });

  // The whole column read again when what it is read as changes.
  let gen = 0;
  $effect(() => {
    const s = specOf(null);
    // Only the reading settings matter here.
    void [s.col, s.kind, s.how, s.separator, s.kv, s.depth, s.trim, s.match_case, s.skip_empty, s.null_blank];
    if ((s.kind === "list" || s.kind === "key_value") && !s.separator) return;
    if (s.kind === "key_value" && !s.kv) return;
    const mine = ++gen;
    let running = true;
    scanning = true;
    problem = "";
    // What was read before no longer fits: Run waits for the new reading.
    scan = null;
    const t = window.setTimeout(() => {
      void backend()
        .explodeScan(doc, s)
        .then((r) => {
          if (mine !== gen) return;
          scan = r;
          // The 1,000 most common are listed and ticked.
          unticked = new Set();
        })
        .catch((e: unknown) => {
          if (mine !== gen) return;
          problem = isCmdError(e) && e.kind === "cancelled" ? "Stopped: Run reads the column again." : errorText(e);
          scan = null;
        })
        .finally(() => {
          running = false;
          if (mine === gen) scanning = false;
        });
    }, 150);
    return () => {
      gen++;
      window.clearTimeout(t);
      if (running) void backend().cancelJob(doc, SCAN);
    };
  });

  $effect(() => {
    if (!scanning) return;
    const poll = window.setInterval(async () => {
      try {
        const j = await backend().jobProgress(doc);
        scanProgress = j ? j.fraction : null;
      } catch {
        // Closed meanwhile.
      }
    }, 200);
    return () => window.clearInterval(poll);
  });

  // The preview: the first 20 shown rows, as the choices change.
  let pgen = 0;
  $effect(() => {
    const s = spec;
    if ((s.kind === "list" || s.kind === "key_value") && !s.separator) return;
    const mine = ++pgen;
    void backend()
      .explodePreview(doc, $state.snapshot(s) as Explode)
      .then((p) => {
        if (mine === pgen) preview = p;
      })
      .catch(() => {
        if (mine === pgen) preview = null;
      });
  });

  const newCols = $derived.by(() => {
    if (!scan) return 0;
    const n = listed ? ticked.length : realHow === "columns" ? scan.most_items : originalCol ? 1 : 0;
    return realHow === "rows" ? n : keepOriginal ? n : Math.max(0, n - 1);
  });
  const why = $derived.by(() => {
    if (readOnly) return "the file is read-only";
    if (!scan) return scanning ? "still reading the column" : "the column could not be read";
    if (realHow !== "rows" && listed && !ticked.length) return hasKeys ? "tick at least one key" : "tick at least one item";
    if (realHow !== "rows" && !listed && scan.most_items === 0) return "no items were found";
    if (realHow !== "rows" && newCols > MAX_COLS) return `at most ${MAX_COLS.toLocaleString()} columns can be added: untick some`;
    if (realHow === "rows" && scan.new_rows > MAX_ROWS) return `at most ${MAX_ROWS.toLocaleString()} rows can be added at once`;
    if (realHow === "rows" && !scan.new_rows && !originalCol) return "no cell holds more than one item";
    if (realHow !== "rows" && !keepOriginal && scan.bad) return "some cells did not parse: keep the original column so they stay as they are";
    return "";
  });
  const what = $derived(kind === "key_value" ? `key${kv}value pairs` : "valid JSON");
  let done = false;
  function run(): void {
    if (done || why) return;
    done = true;
    onRun($state.snapshot(spec) as Explode);
  }
  function setAll(on: boolean): void {
    unticked = on ? new Set() : new Set(scan?.keys.map((k) => k.key) ?? []);
  }
  function tick(k: string, on: boolean): void {
    const next = new Set(unticked);
    if (on) next.delete(k);
    else next.add(k);
    unticked = next;
  }
  const cellText = (c: { text: string; shown: string }) => (c.shown === "missing" ? "(missing)" : c.shown === "null" ? (c.text ? "null" : "(null)") : c.shown === "empty" ? '""' : c.text);
</script>

<FormShell title="Explode cells" wide {onClose} onEnter={run}>
  <div class="row">
    <label
      >Column
      <select bind:value={at} data-testid="explode-col">
        {#each shownCols as [i, t] (i)}<option value={i}>{t}</option>{/each}
      </select></label
    >
    {#if detect}<span class="detect" data-testid="explode-detect">Looks like: {detect.words}</span>{/if}
  </div>
  <div class="row">
    <label
      >Read the cells as
      <select bind:value={kind} data-testid="explode-kind">
        <option value="object">JSON objects</option>
        <option value="json_list">JSON lists</option>
        <option value="list">Lists with a separator</option>
        <option value="key_value">key=value pairs</option>
      </select></label
    >
    {#if kind === "list" || kind === "key_value"}
      <label>{kind === "list" ? "Between items" : "Between pairs"} <input type="text" class="sep" bind:value={separator} maxlength="10" data-testid="explode-sep" /></label>
    {/if}
    {#if kind === "key_value"}
      <label>Between key and value <input type="text" class="sep" bind:value={kv} maxlength="10" data-testid="explode-kv" /></label>
    {/if}
    {#if kind === "object"}
      <label
        >Nested objects
        <select bind:value={depth} data-testid="explode-depth">
          <option value={1}>kept as JSON text</option>
          <option value={2}>flattened one level (a.b)</option>
          <option value={3}>flattened two levels (a.b.c)</option>
        </select></label
      >
    {/if}
  </div>
  {#if !hasKeys}
    <fieldset>
      <legend>Make</legend>
      <label><input type="radio" name="xhow" value="rows" bind:group={how} /> One row per item (the other cells repeated)</label>
      <label><input type="radio" name="xhow" value="columns" bind:group={how} /> One column per position (“title 1”, “title 2”…)</label>
      <label><input type="radio" name="xhow" value="flags" bind:group={how} /> One flag column per distinct item, marked where present</label>
    </fieldset>
  {/if}

  {#if scanning}
    <p class="muted" data-testid="explode-scanning">
      Reading the whole column{scanProgress !== null ? ` (${Math.round(scanProgress * 100)}%)` : ""}…
      <button type="button" onclick={() => void backend().cancelJob(doc, SCAN)}>Stop</button>
    </p>
  {/if}
  {#if problem}<p class="warn" role="alert">{problem}</p>{/if}

  {#if scan && listed}
    <fieldset class="keys">
      <legend>{hasKeys ? "Keys" : "Items"} ({ticked.length.toLocaleString()} of {scan.keys.length.toLocaleString()} ticked)</legend>
      {#if scan.distinct > scan.keys.length}
        <p class="note" data-testid="explode-capped">
          {scan.distinct.toLocaleString()}{scan.capped ? "+" : ""} different {hasKeys ? "keys" : "items"}: the {scan.keys.length.toLocaleString()} most common are listed and ticked.
        </p>
      {/if}
      {#if !hasKeys}
        <p class="note" data-testid="explode-matching">
          Items are matched {matchCase ? "with capitals counting (VIP and vip are two items)" : "ignoring capitals (VIP = vip)"}{trim ? ", spaces around them left out" : ", spaces around them counting"}.
        </p>
      {/if}
      <div class="tools">
        <button type="button" onclick={() => setAll(true)}>Tick all</button>
        <button type="button" onclick={() => setAll(false)}>Untick all</button>
      </div>
      <div class="list" data-testid="explode-keys">
        {#each scan.keys as k (k.key)}
          <label><input type="checkbox" checked={!unticked.has(k.key)} onchange={(e) => tick(k.key, (e.currentTarget as HTMLInputElement).checked)} /> {k.key} <span class="muted">({k.count.toLocaleString()})</span></label>
        {:else}
          <p class="muted">None found.</p>
        {/each}
      </div>
      <label><input type="checkbox" bind:checked={addNew} data-testid="explode-addnew" /> When a recipe runs this again, add {hasKeys ? "keys" : "items"} that are new then as columns (off: they are left out and named in the run report)</label>
    </fieldset>
  {/if}

  <fieldset>
    <legend>Options</legend>
    {#if realHow !== "rows"}
      <label><input type="checkbox" bind:checked={keepOriginal} data-testid="explode-keep" /> Keep the original column</label>
    {:else}
      <label><input type="checkbox" bind:checked={originalCol} data-testid="explode-origcol" /> Keep the whole original in an added column “… (original)”</label>
    {/if}
    <label><input type="checkbox" bind:checked={trim} /> Leave out spaces around {hasKeys ? "keys and values" : "items"}</label>
    {#if kind === "list"}<label><input type="checkbox" bind:checked={skipEmpty} /> Leave out empty items</label>{/if}
    {#if kind === "object" || kind === "json_list"}
      <label><input type="checkbox" bind:checked={nullBlank} /> JSON null as a blank cell (off: the text null)</label>
    {/if}
    {#if realHow === "flags"}
      <label><input type="checkbox" bind:checked={matchCase} /> Capitals count (VIP and vip are two items)</label>
      <label>Mark <input type="text" class="sep" bind:value={mark} maxlength="100" data-testid="explode-mark" /></label>
    {/if}
  </fieldset>

  {#if scan}
    <p class="caps" data-testid="explode-caps">
      {#if realHow === "rows"}
        Adds {scan.new_rows.toLocaleString()} {scan.new_rows === 1 ? "row" : "rows"} (at most {MAX_ROWS.toLocaleString()}){originalCol ? " and 1 column" : ""}.
      {:else}
        Adds {newCols.toLocaleString()} {newCols === 1 ? "column" : "columns"} (at most {MAX_COLS.toLocaleString()}){keepOriginal ? "" : "; the first takes the original column's place"}.
      {/if}
    </p>
    {#if scan.bad}
      <p class="bad" data-testid="explode-bad">
        {scan.bad.toLocaleString()} {scan.bad === 1 ? "cell was" : "cells were"} not {what}: {scan.bad === 1 ? "it stays exactly as it is" : "they stay exactly as they are"}.
        <button type="button" onclick={() => onShowBad($state.snapshot(spec) as Explode)} data-testid="explode-showbad">Show those rows</button>
      </p>
    {/if}
  {/if}

  {#if preview && preview.rows.length}
    <div class="preview" data-testid="explode-preview">
      <table>
        <thead
          ><tr
            ><th>Before</th>{#if realHow === "rows"}<th>{titles[at] ?? `Column ${at + 1}`}</th>{#if originalCol}<th>{preview.titles[0]}</th>{/if}{:else}{#each preview.titles as t, i (i)}<th>{t}</th>{/each}{/if}</tr
          ></thead
        >
        <tbody>
          {#each preview.rows as r (r.row)}
            {#if r.bad}
              <tr class="isbad"><td>{r.before}</td><td colspan={Math.max(1, preview.titles.length)}>did not parse: stays as it is</td></tr>
            {:else if !r.lines.length}
              <tr><td>{r.before}</td><td colspan={Math.max(1, preview.titles.length)} class="muted">blank</td></tr>
            {:else}
              {#each r.lines as line, k (k)}
                <tr>
                  <td>{k === 0 ? r.before : ""}</td>
                  {#each line as c, j (j)}<td class={c.shown}>{cellText(c)}</td>{/each}
                </tr>
              {/each}
            {/if}
          {/each}
        </tbody>
      </table>
      <p class="muted">The first {preview.rows.length} shown rows. (missing): the cell has no such key, left blank; (null): JSON null; "": an empty text.</p>
    </div>
  {/if}

  {#if realHow === "rows"}
    <p class="muted" data-testid="explode-shown">Shown rows only: hidden rows are left alone.</p>
  {:else}
    <HiddenNote kind="all" />
  {/if}
  {#snippet buttons()}
    {#if why}<span class="why" data-testid="explode-why">Run is off: {why}.</span>{/if}
    <button type="button" onclick={onClose}>Cancel</button>
    <button type="button" class="primary" onclick={run} disabled={!!why} title={why} data-testid="explode-run">Explode</button>
  {/snippet}
</FormShell>

<style>
  .row {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
    align-items: center;
    margin-bottom: 8px;
  }
  .detect {
    color: var(--muted);
  }
  .sep {
    width: 4em;
  }
  fieldset {
    border: none;
    padding: 0;
    margin: 0 0 10px;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  legend {
    font-weight: 600;
    margin-bottom: 4px;
  }
  .keys .list {
    max-height: 160px;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
    border: 1px solid var(--border, #8886);
    padding: 4px 6px;
  }
  .tools {
    display: flex;
    gap: 6px;
  }
  .muted,
  .note {
    color: var(--muted);
    font-size: 12px;
  }
  .warn,
  .bad {
    color: var(--danger, #c0392b);
  }
  .caps {
    font-weight: 600;
  }
  .preview {
    max-height: 260px;
    overflow: auto;
    margin: 6px 0;
  }
  table {
    border-collapse: collapse;
    font-size: 12px;
  }
  th,
  td {
    border: 1px solid var(--border, #8886);
    padding: 2px 6px;
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: left;
  }
  td.missing,
  td.null,
  td.empty {
    color: var(--muted);
    font-style: italic;
  }
  tr.isbad td {
    color: var(--danger, #c0392b);
  }
  .why {
    color: var(--muted);
    font-size: 12px;
    margin-right: auto;
  }
</style>
