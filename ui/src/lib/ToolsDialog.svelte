<script lang="ts">
  // The Tools menu's dialogs (Milestone 10): fill blank cells, count or
  // blank out repeats, blank rows between groups, a column to a table and
  // back, random samples, comparing two files, splitting into files and
  // combining files. Each asks for its few choices and hands back what to
  // run; the engine does the work.
  import HiddenNote from "./HiddenNote.svelte";
  import AllNone from "./AllNone.svelte";
  import { untrack } from "svelte";
  import { backend, type Combine, type DataOp, type DiffSpec, type FillCount, type SplitParts, type TableOp } from "./backend";
  import FormShell from "./FormShell.svelte";
  import { GUARD_LABEL } from "./csvguard";
  import { prefs } from "./prefs.svelte";
  import { fillStops, toolsRequest, type ToolKind } from "./m10";

  export interface ToolsRun {
    data?: DataOp;
    table?: TableOp;
    split?: SplitParts;
    combine?: Combine;
    /** Compare by key: the other tab and what to compare (the result opens as a new tab; marks go on all three). */
    diff?: { other: number; spec: DiffSpec };
  }

  interface Props {
    kind: ToolKind;
    titles: string[];
    /** Selected (or active) columns. */
    cols: number[];
    col: number;
    header: boolean;
    /** Other open tabs, for comparing. */
    others: { id: number; name: string }[];
    onRun: (r: ToolsRun, header: boolean) => void;
    onClose: () => void;
    /** Open another file (a file picker) so it need not be open already: it opens as a tab and is returned. */
    onPickFile?: () => Promise<{ id: number; name: string } | null>;
    /** The tab, for the fill's count line. */
    doc?: number;
  }
  let { kind, titles, cols, col, header, others, onRun, onClose, onPickFile, doc }: Props = $props();
  /** Files picked here (opened as tabs) for the compare. */
  let extra = $state<{ id: number; name: string }[]>([]);
  const choices = $derived([...others, ...extra.filter((e) => !others.some((o) => o.id === e.id))]);
  async function pickFile(): Promise<void> {
    const f = await onPickFile?.();
    if (f) {
      extra = [...extra, f];
      other = f.id;
    }
  }

  const TITLES: Record<ToolKind, string> = {
    fill_blanks: "Fill blank cells",
    count_repeats: "Count repeats",
    blank_repeats: "Blank out repeats",
    breaks: "Blank rows between groups",
    reshape: "Column to a table",
    stack: "Columns to one column",
    sample: "Random sample",
    compare: "Compare two files",
    split: "Split into files",
    combine: "Combine files",
  };

  let picked = $state<number[]>([]);
  let one = $state(untrack(() => col));
  let myHeader = $state(untrack(() => header));
  let from = $state<"above" | "below" | "value">("above");
  let text = $state("");
  let spaces = $state(true);
  let matchCase = $state(false);
  let trim = $state(true);
  let byCol = $state(true);
  let every = $state(100);
  // The Split dialog's own tick: off unless the person chose it last time.
  let guard = $state(prefs.data.csvFormulaGuardSplit);
  let blankRows = $state(1);
  let perRow = $state(3);
  let down = $state(false);
  let skipBlanks = $state(true);
  let sizeKind = $state<"rows" | "percent" | "every">("rows");
  let size = $state(100);
  let seed = $state(Math.floor(Math.random() * 1_000_000));
  let per = $state(-1);
  let drawnOrder = $state(false);
  let rowNumbers = $state(true);
  let other = $state(untrack(() => others[0]?.id ?? -1));
  let wholeRow = $state(false);
  let showSame = $state(false);
  let byTitle = $state(true);
  let source = $state(true);
  let problem = $state("");
  // Fill blank cells: where the fill stops.
  let fillGuard = $state(-1);
  let stopBlankRows = $state(false);
  let stopTotalRows = $state(false);
  const stopsOn = $derived(from !== "value" && (fillGuard >= 0 || stopBlankRows || stopTotalRows));
  /** The count line (null while counting or with no stops), or why there is none. */
  let count = $state<FillCount | null>(null);
  let countNote = $state("");
  $effect(() => {
    if (kind !== "fill_blanks" || doc === undefined || !stopsOn) {
      count = null;
      countNote = "";
      return;
    }
    const spec = {
      cols: [...picked],
      from: from === "below" ? { kind: "below" as const } : { kind: "above" as const },
      spaces,
      keep_first: myHeader,
      ...fillStops({ from, fillGuard, stopBlankRows, stopTotalRows }),
    };
    count = null;
    if (!spec.cols.length) {
      countNote = "Tick the columns to fill to see the count.";
      return;
    }
    if (spec.guard !== undefined && spec.cols.includes(spec.guard)) {
      countNote = "";
      return;
    }
    countNote = "Counting…";
    const id = doc;
    let live = true;
    const timer = setTimeout(() => {
      backend()
        .fillBlanksCount(id, spec)
        .then((c) => {
          if (!live) return;
          count = c;
          countNote = "";
        })
        .catch((e: unknown) => {
          if (live) countNote = (e as { code?: string })?.code === "cancelled" ? "" : "The count is not available right now.";
        });
    }, 200);
    // A newer choice, or the dialog closing, drops this count (the engine stops it).
    return () => {
      live = false;
      clearTimeout(timer);
      void backend().fillBlanksCountStop(id).catch(() => {});
    };
  });
  const countText = $derived(
    count
      ? `Will fill ${count.fill.toLocaleString()} blank ${count.fill === 1 ? "cell" : "cells"}; ${count.skipped.toLocaleString()} left blank by the stops.` +
          (count.counted < count.rows ? ` (Counted in the first ${count.counted.toLocaleString()} of ${count.rows.toLocaleString()} shown rows.)` : "")
      : countNote,
  );

  function toggle(c: number): void {
    picked = picked.includes(c) ? picked.filter((x) => x !== c) : [...picked, c].sort((a, b) => a - b);
  }

  function run(): void {
    if (kind === "compare" && !wholeRow) {
      // By key: the new engine pairs the rows by the key columns and the columns by title.
      if (!picked.length) {
        problem = "Tick the key columns (what identifies a row), or compare whole rows.";
        return;
      }
      if (other < 0) {
        problem = "Choose the other file.";
        return;
      }
      onRun({ diff: { other, spec: { keys: picked, keep_first: myHeader, match_case: matchCase, trim } } }, myHeader);
      return;
    }
    const r = toolsRequest(kind, {
      cols: picked,
      col: one,
      header: myHeader,
      from,
      text,
      spaces,
      matchCase,
      trim,
      byCol,
      every,
      guard,
      blankRows,
      perRow,
      down,
      skipBlanks,
      sizeKind,
      size,
      seed,
      per,
      drawnOrder,
      rowNumbers,
      other,
      wholeRow,
      showSame,
      byTitle,
      source,
      fillGuard,
      stopBlankRows,
      stopTotalRows,
    });
    if (typeof r === "string") {
      problem = r;
      return;
    }
    onRun(r, myHeader);
  }

  const many = $derived(kind === "fill_blanks" || kind === "count_repeats" || kind === "blank_repeats" || kind === "stack" || (kind === "compare" && !wholeRow));
  const single = $derived(kind === "reshape" || ((kind === "breaks" || kind === "split") && byCol));
</script>

<FormShell title={TITLES[kind]} {onClose} onEnter={run}>
  {#if kind === "compare"}
    {#if choices.length}
      <label class="row">Compare this file with <select bind:value={other}>{#each choices as o (o.id)}<option value={o.id}>{o.name}</option>{/each}</select></label>
    {:else}
      <p class="note">Open the other file in a tab, or choose it here.</p>
    {/if}
    {#if onPickFile}<div class="row"><button type="button" onclick={() => void pickFile()}>Choose another file…</button><span class="muted">It opens as a tab; the files themselves do not change.</span></div>{/if}
    <label class="row"><input type="checkbox" bind:checked={wholeRow} /> Compare whole rows (no key columns)</label>
  {/if}
  {#if kind === "combine"}
    <p class="note">Choose the files in the next window (hold Ctrl to pick several). They are stacked into a new tab; the files themselves do not change.</p>
    <label class="row"><input type="checkbox" bind:checked={myHeader} /> Each file starts with a row of titles</label>
    {#if myHeader}<label class="row"><input type="checkbox" bind:checked={byTitle} /> Match columns by their titles (otherwise by position)</label>{/if}
    {#if myHeader && byTitle}<p class="note">A title repeated within one file pairs in order: Note, Note (2). After choosing the files you see how their columns line up, and can change it, merge columns with different titles, or use a saved column map.</p>{/if}
    <label class="row"><input type="checkbox" bind:checked={source} /> Add a column with each row's file name</label>
  {/if}
  {#if many}
    <fieldset>
      <legend>{kind === "compare" ? "Key columns (what identifies a row)" : kind === "stack" ? "Columns, in this order" : "Columns"}</legend>
      <AllNone count={titles.length} picked={picked} onChange={(v) => (picked = v)} />
      <div class="list">
        {#each titles as t, c (c)}<label><input type="checkbox" checked={picked.includes(c)} onchange={() => toggle(c)} /> {t}</label>{/each}
      </div>
    </fieldset>
  {/if}
  {#if kind === "breaks" || kind === "split"}
    <div class="row">
      <label><input type="radio" value={true} bind:group={byCol} /> {kind === "split" ? "One file for each value of" : "Where the value changes in"}</label>
      <label><input type="radio" value={false} bind:group={byCol} /> Every</label>
    </div>
    {#if !byCol}<label class="row"><input type="number" min="1" bind:value={every} /> rows</label>{/if}
  {/if}
  {#if single}
    <label class="row">Column <select bind:value={one}>{#each titles as t, c (c)}<option value={c}>{t}</option>{/each}</select></label>
  {/if}
  {#if kind === "fill_blanks"}
    <div class="col">
      <label><input type="radio" value="above" bind:group={from} /> From the cell above</label>
      <label><input type="radio" value="below" bind:group={from} /> From the cell below</label>
      <label><input type="radio" value="value" bind:group={from} /> With this text: <input type="text" bind:value={text} disabled={from !== "value"} /></label>
    </div>
    <label class="row"><input type="checkbox" bind:checked={spaces} /> Cells holding only spaces count as blank</label>
    <fieldset data-testid="fill-stops" disabled={from === "value"} title={from === "value" ? "Stops apply when copying from above or below" : ""}>
      <legend>Stop the fill at</legend>
      <label class="row">Rows where this column is blank:
        <select bind:value={fillGuard} aria-label="Stop at rows where this column is blank">
          <option value={-1}>(none)</option>
          {#each titles as t, c (c)}<option value={c} disabled={picked.includes(c)} title={picked.includes(c) ? "Ticked for filling: a column cannot both stop the fill and be filled" : ""}>{t}</option>{/each}
        </select>
      </label>
      <p class="note">For example an ID column: a row with no ID is not filled and starts a new group.</p>
      <label class="row"><input type="checkbox" bind:checked={stopBlankRows} /> Blank rows (every cell empty, hidden columns included)</label>
      <label class="row"><input type="checkbox" bind:checked={stopTotalRows} /> Total rows (first filled cell says Total, Subtotal, Grand total or Sum, and a number)</label>
      {#if from === "value"}<p class="note">Stops apply when copying from above or below.</p>{/if}
      {#if fillGuard >= 0 && picked.includes(fillGuard)}<p class="problem" role="alert">The column that stops the fill cannot also be filled: untick it, or choose another column to stop at.</p>{/if}
    </fieldset>
    {#if stopsOn && countText}<p class="note" data-testid="fill-count" aria-live="polite">{countText}</p>{/if}
  {/if}
  {#if kind === "count_repeats" || kind === "blank_repeats" || (kind === "compare" && !wholeRow)}
    <label class="row"><input type="checkbox" bind:checked={matchCase} /> Match capitals (Rome and rome differ)</label>
    <label class="row"><input type="checkbox" bind:checked={trim} /> Ignore spaces at the start and end</label>
  {/if}
  {#if kind === "compare" && wholeRow}<label class="row"><input type="checkbox" bind:checked={showSame} /> List the rows that are the same too</label>{/if}
  {#if kind === "compare" && !wholeRow}<p class="note">By key: rows are paired by the key columns and columns by their titles (a re-sorted file gives the same result). The result lists what changed; the three tabs show the differences in colour (F7-style jumps: Alt+F7 next, Alt+Shift+F7 previous).</p>{/if}
  {#if kind === "breaks"}<label class="row">Blank rows at each break <input type="number" min="1" max="100" bind:value={blankRows} /></label>{/if}
  {#if kind === "reshape"}
    <label class="row">Cells on each new row <input type="number" min="1" max="16384" bind:value={perRow} /></label>
    <div class="row">
      <label><input type="radio" value={false} bind:group={down} /> Across first (1 2 3 / 4 5 6)</label>
      <label><input type="radio" value={true} bind:group={down} /> Down first (1 3 5 / 2 4 6)</label>
    </div>
  {/if}
  {#if kind === "stack"}<label class="row"><input type="checkbox" bind:checked={skipBlanks} /> Leave out blank cells</label>{/if}
  {#if kind === "sample"}
    <div class="row">
      <select bind:value={sizeKind}>
        <option value="rows">Rows at random:</option>
        <option value="percent">Percent at random:</option>
        <option value="every">Every nth row, n =</option>
      </select>
      <input type="number" min="0" step="any" bind:value={size} />
    </div>
    {#if sizeKind !== "every"}
      <label class="row">From <select bind:value={per} aria-label="Draw from">
        <option value={-1}>all the rows at once</option>
        {#each titles as t, c (c)}<option value={c}>each group of {t}</option>{/each}
      </select>{per >= 0 ? (sizeKind === "rows" ? `: ${size} from each` : `: ${size}% of each`) : ""}</label>
      <label class="row">Seed <input type="number" min="0" bind:value={seed} /> <button type="button" class="tiny" onclick={() => (seed = Math.floor(Math.random() * 1_000_000))}>New seed</button></label>
      <p class="note">The same seed draws the same rows of the same table again, so anyone can check the sample. A record of the seed, the method, the file's SHA-256 fingerprint and the counts per group opens beside it.</p>
    {/if}
    <label class="row"><input type="checkbox" bind:checked={rowNumbers} /> Add a Row column with each row's number</label>
    <label class="row"><input type="checkbox" bind:checked={drawnOrder} /> Keep the order drawn (otherwise as in the table)</label>
  {/if}
  {#if kind !== "combine"}<label class="row"><input type="checkbox" bind:checked={myHeader} /> The first row holds titles{kind === "split" ? " (each file gets them)" : ""}</label>{/if}
  {#if kind === "split"}
    <label class="row"><input type="checkbox" data-testid="split-guard" bind:checked={guard} onchange={() => { prefs.data.csvFormulaGuardSplit = guard; prefs.save(); }} /> {GUARD_LABEL}</label>
    <p class="note">Off by default: the files keep your values exactly. Ticked, cells starting with = + - @ get an apostrophe in front (plain numbers do not).</p>
  {/if}
  {#if kind === "reshape" || kind === "stack" || kind === "sample" || kind === "compare"}
    <p class="note">The result opens as a new table; this file does not change.</p>
  {:else if kind === "split"}
    <p class="note">Choose the folder in the next window. Files already there are only replaced if you agree. Hidden and filtered-out rows are left out.</p>
  {:else if kind !== "combine"}
    <p class="note">Only shown rows change. Undo (Ctrl+Z) puts everything back.</p>
  {/if}
  {#if kind === "count_repeats"}<HiddenNote kind="empty" />{/if}
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" class="primary" onclick={run} disabled={kind === "compare" && !choices.length}>{kind === "split" || kind === "combine" ? "Choose…" : "Run"}</button>
    <button type="button" onclick={onClose}>Cancel</button>
  {/snippet}
</FormShell>

<style>
  .row {
    display: flex;
    gap: 6px;
    align-items: center;
    flex-wrap: wrap;
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .list {
    display: flex;
    flex-direction: column;
    max-height: 12rem;
    overflow: auto;
  }
  input[type="number"] {
    width: 7rem;
  }
  .note {
    color: var(--muted);
    font-size: 0.85rem;
  }
  .problem {
    color: var(--danger, #c00);
  }
</style>
