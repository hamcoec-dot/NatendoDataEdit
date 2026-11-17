<script lang="ts">
  // The Tools menu's dialogs (Milestone 10): fill blank cells, count or
  // blank out repeats, blank rows between groups, a column to a table and
  // back, random samples, comparing two files, splitting into files and
  // combining files. Each asks for its few choices and hands back what to
  // run; the engine does the work.
  import AllNone from "./AllNone.svelte";
  import { untrack } from "svelte";
  import type { Combine, DataOp, SplitParts, TableOp } from "./backend";
  import FormShell from "./FormShell.svelte";
  import { toolsRequest, type ToolKind } from "./m10";

  export interface ToolsRun {
    data?: DataOp;
    table?: TableOp;
    split?: SplitParts;
    combine?: Combine;
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
  }
  let { kind, titles, cols, col, header, others, onRun, onClose }: Props = $props();

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
  let blankRows = $state(1);
  let perRow = $state(3);
  let down = $state(false);
  let skipBlanks = $state(true);
  let sizeKind = $state<"rows" | "percent" | "every">("rows");
  let size = $state(100);
  let seed = $state(Math.floor(Math.random() * 1_000_000));
  let other = $state(untrack(() => others[0]?.id ?? -1));
  let wholeRow = $state(false);
  let showSame = $state(false);
  let byTitle = $state(true);
  let source = $state(true);
  let problem = $state("");

  function toggle(c: number): void {
    picked = picked.includes(c) ? picked.filter((x) => x !== c) : [...picked, c].sort((a, b) => a - b);
  }

  function run(): void {
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
      blankRows,
      perRow,
      down,
      skipBlanks,
      sizeKind,
      size,
      seed,
      other,
      wholeRow,
      showSame,
      byTitle,
      source,
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
    {#if others.length}
      <label class="row">Compare this file with <select bind:value={other}>{#each others as o (o.id)}<option value={o.id}>{o.name}</option>{/each}</select></label>
    {:else}
      <p class="problem">Open the other file in a tab first.</p>
    {/if}
    <label class="row"><input type="checkbox" bind:checked={wholeRow} /> Compare whole rows (no key columns)</label>
  {/if}
  {#if kind === "combine"}
    <p class="note">Choose the files in the next window (hold Ctrl to pick several). They are stacked into a new tab; the files themselves do not change.</p>
    <label class="row"><input type="checkbox" bind:checked={myHeader} /> Each file starts with a row of titles</label>
    {#if myHeader}<label class="row"><input type="checkbox" bind:checked={byTitle} /> Match columns by their titles (otherwise by position)</label>{/if}
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
  {/if}
  {#if kind === "count_repeats" || kind === "blank_repeats" || (kind === "compare" && !wholeRow)}
    <label class="row"><input type="checkbox" bind:checked={matchCase} /> Match capitals (Rome and rome differ)</label>
    <label class="row"><input type="checkbox" bind:checked={trim} /> Ignore spaces at the start and end</label>
  {/if}
  {#if kind === "compare"}<label class="row"><input type="checkbox" bind:checked={showSame} /> List the rows that are the same too</label>{/if}
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
    {#if sizeKind !== "every"}<label class="row">Seed <input type="number" min="0" bind:value={seed} /> (the same seed picks the same rows again)</label>{/if}
  {/if}
  {#if kind !== "combine"}<label class="row"><input type="checkbox" bind:checked={myHeader} /> The first row holds titles{kind === "split" ? " (each file gets them)" : ""}</label>{/if}
  {#if kind === "reshape" || kind === "stack" || kind === "sample" || kind === "compare"}
    <p class="note">The result opens as a new table; this file does not change.</p>
  {:else if kind === "split"}
    <p class="note">Choose the folder in the next window. Files already there are only replaced if you agree. Hidden and filtered-out rows are left out.</p>
  {:else if kind !== "combine"}
    <p class="note">Only shown rows change. Undo (Ctrl+Z) puts everything back.</p>
  {/if}
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" class="primary" onclick={run} disabled={kind === "compare" && !others.length}>{kind === "split" || kind === "combine" ? "Choose…" : "Run"}</button>
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
