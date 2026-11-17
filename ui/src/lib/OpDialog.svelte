<script lang="ts" module>
  export type OpKind =
    | "remove_duplicates"
    | "show_duplicates"
    | "split_column"
    | "merge_columns"
    | "split_rows"
    | "merge_rows"
    | "pivot"
    | "unpivot"
    | "series"
    | "paste_special";
</script>

<script lang="ts">
  // One dialog for the Data menu's operations, Paste Special and Fill
  // Series. Each kind shows only its own options and hands back the request
  // for the engine; nothing changes until the user presses the main button.
  import AllNone from "./AllNone.svelte";
  import { untrack } from "svelte";
  import type { Aggregate, DataOp, DateUnit, Duplicates, FillMode, PasteOp, SplitColumn, TableOp } from "./backend";
  import { KEEP_COPIES, parsePositions, SPLIT_MODES } from "./editing";
  import type { Rect } from "./selection";

  interface Props {
    kind: OpKind;
    /** Column titles (or numbers), one per column (may be cut short). */
    titles: string[];
    /** Every column in the document (titles may list fewer). */
    cols: number;
    header: boolean;
    sel: Rect;
    /** Paste Special: clipboard text, when it could be read. */
    clip?: string;
    onRun: (r: { data?: DataOp; table?: TableOp }, header: boolean) => void;
    onClose: () => void;
  }
  let { kind, titles, cols, header, sel, clip = "", onRun, onClose }: Props = $props();

  let box: HTMLDivElement;
  let hasHeader = $state(untrack(() => header));
  const selCols = untrack(() => Array.from({ length: Math.min(sel.cols, titles.length) }, (_, i) => sel.col + i));
  const focusCol = untrack(() => Math.min(sel.col, Math.max(0, titles.length - 1)));

  // Shared options.
  let picked = $state<number[]>(
    // Nothing ticked to start (Select all ticks every column).
    [],
  );
  let matchCase = $state(false);
  let trim = $state(true);
  let sepChoice = $state(untrack(() => (kind === "merge_columns" ? "space" : kind === "merge_rows" || kind === "pivot" ? "comma_space" : "comma")));
  let sepCustom = $state("");
  let col = $state(focusCol);
  // Split column.
  let maxParts = $state(0);
  let keepOriginal = $state(false);
  let splitMode = $state<NonNullable<SplitColumn["mode"]>>("text");
  let positions = $state("");
  // Remove duplicates: which copy stays.
  let keepCopy = $state<NonNullable<Duplicates["keep"]>>("first");
  let keepDayFirst = $state(false);
  let dateCol = $state(untrack(() => Math.max(0, titles.findIndex((t) => /date|time|updated|changed|when/i.test(t)))));
  let fillBlanks = $state(false);
  // Merge.
  let skipBlanks = $state(untrack(() => kind !== "paste_special"));
  let distinct = $state(true);
  // Split rows.
  let skipEmpty = $state(true);
  // Pivot.
  let pivotCol = $state(untrack(() => Math.min(focusCol + 1, Math.max(0, titles.length - 1))));
  let valueCol = $state(untrack(() => Math.min(focusCol + 2, Math.max(0, titles.length - 1))));
  let aggregate = $state<Aggregate>("sum");
  // Unpivot.
  let valuesAll = $state(true);
  let valuePicked = $state<number[]>([]);
  let nameTitle = $state("Name");
  let valueTitle = $state("Value");
  // Series.
  let seriesType = $state<"auto" | "linear" | "growth" | "date">("linear");
  let step = $state("1");
  let stop = $state("");
  let unit = $state<DateUnit>("day");
  // Paste Special.
  let pasteOp = $state<PasteOp>("none");
  let transpose = $state(false);
  let text = $state(untrack(() => clip));

  const SEPARATORS: [string, string, string][] = [
    ["comma", "Comma", ","],
    ["comma_space", "Comma and space", ", "],
    ["semicolon", "Semicolon", ";"],
    ["space", "Space", " "],
    ["tab", "Tab", "\t"],
    ["pipe", "Vertical bar |", "|"],
    ["newline", "Line break", "\n"],
    ["none", "Nothing", ""],
    ["custom", "Other:", ""],
  ];
  const separator = $derived(sepChoice === "custom" ? sepCustom : (SEPARATORS.find((s) => s[0] === sepChoice)?.[2] ?? ","));

  const TITLES: Record<OpKind, string> = {
    remove_duplicates: "Remove duplicates",
    show_duplicates: "Show duplicates",
    split_column: "Split column",
    merge_columns: "Merge columns",
    split_rows: "Split into rows",
    merge_rows: "Merge rows",
    pivot: "Pivot",
    unpivot: "Unpivot",
    series: "Fill series",
    paste_special: "Paste Special",
  };
  const AGGREGATES: [Aggregate, string][] = [
    ["sum", "Sum"],
    ["count", "Count (non-blank)"],
    ["average", "Average"],
    ["min", "Smallest"],
    ["max", "Largest"],
    ["first", "First value"],
    ["join", "All values, joined"],
  ];

  function toggle(list: number[], c: number): number[] {
    return list.includes(c) ? list.filter((x) => x !== c) : [...list, c].sort((a, b) => a - b);
  }

  let problem = $state("");

  function ok(): void {
    problem = "";
    const keep_first = hasHeader;
    const need = (cond: boolean, msg: string): boolean => {
      if (!cond) problem = msg;
      return cond;
    };
    switch (kind) {
      case "remove_duplicates":
      case "show_duplicates": {
        const spec: Duplicates = { cols: $state.snapshot(picked), match_case: matchCase, trim, keep_first };
        if (kind === "remove_duplicates") {
          spec.keep = keepCopy;
          spec.date_col = keepCopy === "newest" || keepCopy === "oldest" ? dateCol : null;
          spec.fill_blanks = fillBlanks;
          spec.read = { decimal_comma: false, date_order: keepDayFirst ? "dmy" : "mdy" };
        }
        onRun({ data: { op: kind, spec } }, hasHeader);
        return;
      }
      case "split_column": {
        const needsSep = splitMode === "text" || splitMode === "last" || splitMode === "any";
        const sep = splitMode === "any" ? sepCustom : separator;
        if (!need(!needsSep || sep !== "", splitMode === "any" ? "Type the characters to split at." : "Choose a separator to split at.")) return;
        const pos = parsePositions(positions);
        if (!need(splitMode !== "positions" || pos.length > 0, "Give the positions to split at, like 3, 7.")) return;
        onRun(
          { data: { op: "split_column", spec: { col, separator: sep, max_parts: Math.max(0, Math.floor(maxParts) || 0), keep_original: keepOriginal, trim, keep_first, mode: splitMode, positions: pos } } },
          hasHeader,
        );
        return;
      }
      case "merge_columns": {
        const count = Math.max(sel.cols, 2);
        if (!need(sel.col + count <= cols, "Select two or more neighbouring columns first.")) return;
        onRun({ data: { op: "merge_columns", spec: { col: sel.col, count, separator, skip_blanks: skipBlanks } } }, hasHeader);
        return;
      }
      case "split_rows":
        if (!need(separator !== "", "Choose a separator to split at.")) return;
        onRun({ data: { op: "split_rows", spec: { col, separator, trim, skip_empty: skipEmpty, keep_first } } }, hasHeader);
        return;
      case "merge_rows":
        if (!need(picked.length > 0, "Tick the column(s) that identify a row.")) return;
        onRun({ data: { op: "merge_rows", spec: { keys: $state.snapshot(picked), separator, distinct, match_case: matchCase, trim, keep_first } } }, hasHeader);
        return;
      case "pivot":
        if (!need(picked.length > 0, "Tick the column(s) whose values make the rows.")) return;
        onRun({ table: { op: "pivot", spec: { rows: $state.snapshot(picked), col: pivotCol, value: valueCol, aggregate, separator, keep_first } } }, hasHeader);
        return;
      case "unpivot": {
        const values = valuesAll ? [] : $state.snapshot(valuePicked);
        if (!need(valuesAll || values.length > 0, "Tick the columns to turn into rows.")) return;
        onRun({ table: { op: "unpivot", spec: { ids: $state.snapshot(picked), values, name_title: nameTitle, value_title: valueTitle, skip_blanks: skipBlanks, keep_first } } }, hasHeader);
        return;
      }
      case "series": {
        const s = Number(step.replace(",", "."));
        const st = stop.trim() === "" ? null : Number(stop.replace(",", "."));
        if (!need(Number.isFinite(s), "The step must be a number.")) return;
        if (!need(st === null || Number.isFinite(st), "The stop value must be a number.")) return;
        const vertical = sel.rows > 1 || sel.cols === 1;
        const mode: FillMode =
          seriesType === "auto" ? { kind: "auto" } : seriesType === "date" ? { kind: "date", unit, step: Math.round(s) } : { kind: seriesType, step: s, stop: st };
        const spec = vertical
          ? { row: sel.row, col: sel.col, rows: 1, cols: sel.cols, dir: "down" as const, count: sel.rows - 1, mode }
          : { row: sel.row, col: sel.col, rows: sel.rows, cols: 1, dir: "right" as const, count: sel.cols - 1, mode };
        if (!need(spec.count > 0, "Select the cells to fill first (the first one holds the start value).")) return;
        onRun({ data: { op: "fill", spec } }, hasHeader);
        return;
      }
      case "paste_special":
        if (!need(text !== "", "Nothing to paste: copy some cells first, or paste them into the box below.")) return;
        onRun({ data: { op: "paste", spec: { row: sel.row, col: sel.col, op: pasteOp, skip_blanks: skipBlanks, transpose }, text } }, hasHeader);
        return;
    }
  }

  function onKey(e: KeyboardEvent): void {
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "Enter" && !(e.target instanceof HTMLButtonElement) && !(e.target instanceof HTMLTextAreaElement)) {
      e.preventDefault();
      ok();
    }
  }

  $effect(() => {
    box.querySelector<HTMLElement>("select, input, textarea")?.focus();
  });

  const usesHeader = $derived(kind !== "series" && kind !== "paste_special" && kind !== "merge_columns");
  const usesSep = $derived(["split_column", "merge_columns", "split_rows", "merge_rows", "pivot"].includes(kind));
</script>

{#snippet columnList(list: number[], set: (v: number[]) => void, label: string)}
  <fieldset class="cols">
    <legend>{label}</legend>
    <AllNone count={titles.length} picked={list} onChange={set} />
    <div class="list">
      {#each titles as t, c (c)}
        <label><input type="checkbox" checked={list.includes(c)} onchange={() => set(toggle(list, c))} /> {t}</label>
      {/each}
    </div>
    {#if titles.length < cols}<p class="note">Only the first {titles.length.toLocaleString()} of {cols.toLocaleString()} columns are listed.</p>{/if}
  </fieldset>
{/snippet}

{#snippet columnSelect(value: number, set: (v: number) => void, label: string)}
  <label class="row">{label}
    <select value={value} onchange={(e) => set(Number((e.currentTarget as HTMLSelectElement).value))}>
      {#each titles as t, c (c)}<option value={c}>{t}</option>{/each}
    </select>
  </label>
{/snippet}

<div class="backdrop" role="presentation">
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="op-title" tabindex="-1" bind:this={box} onkeydown={onKey}>
    <h2 id="op-title">{TITLES[kind]}</h2>
    {#if usesHeader}
      <label class="row"><input type="checkbox" bind:checked={hasHeader} /> First row is header (leave it as it is)</label>
    {/if}

    {#if kind === "remove_duplicates" || kind === "show_duplicates"}
      <p class="note">Rows count as the same when the ticked columns match. Tick none to compare whole rows.</p>
      {@render columnList(picked, (v) => (picked = v), "Compare these columns")}
      <label class="row"><input type="checkbox" bind:checked={matchCase} /> Match case</label>
      <label class="row"><input type="checkbox" bind:checked={trim} /> Ignore spaces at the start and end</label>
      {#if kind === "remove_duplicates"}
        <label class="row">Keep
          <select bind:value={keepCopy}>
            {#each KEEP_COPIES as [v, label] (v)}<option value={v}>{label}</option>{/each}
          </select>
        </label>
        {#if keepCopy === "newest" || keepCopy === "oldest"}
          {@render columnSelect(dateCol, (v) => (dateCol = v), "Date column")}
          <label class="row"><input type="checkbox" bind:checked={keepDayFirst} /> Dates are day first (31/12)</label>
        {/if}
        <label class="row"><input type="checkbox" bind:checked={fillBlanks} /> Fill the kept copy's blank cells from the copies removed</label>
      {/if}
      <p class="note">
        {kind === "remove_duplicates"
          ? "One copy of each group stays; the others are removed (one undo step). With a filter on, only shown rows are touched."
          : "Shows every row that has a duplicate, the first one too. Clear the filter to see all rows again."}
      </p>
    {:else if kind === "split_column" || kind === "split_rows"}
      {@render columnSelect(col, (v) => (col = v), "Column")}
    {:else if kind === "merge_columns"}
      <p class="note">Merges the {Math.max(sel.cols, 2)} selected columns, starting at “{titles[sel.col] ?? sel.col + 1}”, into the first of them; the others are removed. Every row is merged, hidden ones too.</p>
    {:else if kind === "merge_rows"}
      <p class="note">Rows whose ticked columns match are merged into the first of them; the other columns' values are joined.</p>
      {@render columnList(picked, (v) => (picked = v), "Rows match on")}
      <label class="row"><input type="checkbox" bind:checked={distinct} /> Each different value once</label>
      <label class="row"><input type="checkbox" bind:checked={matchCase} /> Match case</label>
      <label class="row"><input type="checkbox" bind:checked={trim} /> Ignore spaces at the start and end</label>
    {:else if kind === "pivot"}
      {@render columnList(picked, (v) => (picked = v), "One row for each value of")}
      {@render columnSelect(pivotCol, (v) => (pivotCol = v), "One column for each value of")}
      {@render columnSelect(valueCol, (v) => (valueCol = v), "Cells show values of")}
      <label class="row">Combine values by
        <select bind:value={aggregate}>
          {#each AGGREGATES as [v, label] (v)}<option value={v}>{label}</option>{/each}
        </select>
      </label>
      <p class="note">The table opens in a new tab; this file is not changed.</p>
    {:else if kind === "unpivot"}
      {@render columnList(picked, (v) => (picked = v), "Keep these columns on every row")}
      <label class="row"><input type="checkbox" bind:checked={valuesAll} /> Turn every other column into rows</label>
      {#if !valuesAll}{@render columnList(valuePicked, (v) => (valuePicked = v), "Turn these columns into rows")}{/if}
      <label class="row">Title for the column names <input type="text" bind:value={nameTitle} maxlength="200" /></label>
      <label class="row">Title for the values <input type="text" bind:value={valueTitle} maxlength="200" /></label>
      <label class="row"><input type="checkbox" bind:checked={skipBlanks} /> Leave out blank values</label>
      <p class="note">The table opens in a new tab; this file is not changed.</p>
    {:else if kind === "series"}
      <p class="note">Fills the selection from its first cell{sel.rows > 1 || sel.cols === 1 ? " down" : " to the right"}.</p>
      <label class="row">Type
        <select bind:value={seriesType}>
          <option value="linear">Add the step each time</option>
          <option value="growth">Multiply by the step each time</option>
          <option value="date">Dates</option>
          <option value="auto">Continue the pattern of the first cells</option>
        </select>
      </label>
      {#if seriesType === "date"}
        <label class="row">Step by
          <select bind:value={unit}>
            <option value="day">Day</option>
            <option value="weekday">Weekday (Monday to Friday)</option>
            <option value="month">Month</option>
            <option value="year">Year</option>
          </select>
        </label>
      {/if}
      {#if seriesType !== "auto"}
        <label class="row">Step <input type="text" inputmode="decimal" bind:value={step} maxlength="40" /></label>
      {/if}
      {#if seriesType === "linear" || seriesType === "growth"}
        <label class="row">Stop at (optional) <input type="text" inputmode="decimal" bind:value={stop} maxlength="40" /></label>
      {/if}
    {:else if kind === "paste_special"}
      <fieldset>
        <legend>Then</legend>
        {#each [["none", "Just paste the values"], ["add", "Add to the cells"], ["subtract", "Subtract from the cells"], ["multiply", "Multiply the cells"], ["divide", "Divide the cells"]] as [v, label] (v)}
          <label class="row"><input type="radio" name="pasteop" value={v} bind:group={pasteOp} /> {label}</label>
        {/each}
      </fieldset>
      <label class="row"><input type="checkbox" bind:checked={skipBlanks} /> Skip blanks (blank pasted cells leave the cell under them as it is)</label>
      <label class="row"><input type="checkbox" bind:checked={transpose} /> Transpose (rows become columns)</label>
      <label class="block">What to paste (from the clipboard; you can also press Ctrl+V here)
        <textarea rows="5" bind:value={text} spellcheck="false"></textarea>
      </label>
      <p class="note">With add, subtract, multiply or divide, only numbers combine; a cell is left as it is when either side is not a number.</p>
    {/if}

    {#if kind === "split_column"}
      <label class="row">Split
        <select bind:value={splitMode}>
          {#each SPLIT_MODES as [v, label] (v)}<option value={v}>{label}</option>{/each}
        </select>
      </label>
      {#if splitMode === "any"}
        <label class="row">Characters <input type="text" bind:value={sepCustom} maxlength="1000" placeholder=",;/" aria-label="Characters to split at" /></label>
      {:else if splitMode === "positions"}
        <label class="row">After character <input type="text" bind:value={positions} placeholder="3, 7" aria-label="Positions to split at" /></label>
      {/if}
    {/if}
    {#if usesSep && kind !== "pivot" && (kind !== "split_column" || splitMode === "text" || splitMode === "last")}
      <label class="row">Separator
        <select bind:value={sepChoice}>
          {#each SEPARATORS.filter((s) => s[0] !== "none" || kind === "merge_columns" || kind === "merge_rows") as [v, label] (v)}<option value={v}>{label}</option>{/each}
        </select>
        {#if sepChoice === "custom"}<input type="text" bind:value={sepCustom} maxlength="1000" aria-label="Separator text" />{/if}
      </label>
    {:else if kind === "pivot" && aggregate === "join"}
      <label class="row">Join with
        <select bind:value={sepChoice}>
          {#each SEPARATORS.filter((s) => s[0] !== "custom") as [v, label] (v)}<option value={v}>{label}</option>{/each}
        </select>
      </label>
    {/if}
    {#if kind === "split_column"}
      <label class="row">At most <input type="number" min="0" max="1000" bind:value={maxParts} /> parts (0: no limit; the last part keeps the rest)</label>
      <label class="row"><input type="checkbox" bind:checked={keepOriginal} /> Keep the original column</label>
      <label class="row"><input type="checkbox" bind:checked={trim} /> Remove spaces around each part</label>
      <p class="note">New columns are added to the right; every row is split, hidden ones too.</p>
    {:else if kind === "split_rows"}
      <label class="row"><input type="checkbox" bind:checked={trim} /> Remove spaces around each part</label>
      <label class="row"><input type="checkbox" bind:checked={skipEmpty} /> Leave out empty parts</label>
      <p class="note">Each part gets its own row; the other cells are repeated.</p>
    {:else if kind === "merge_columns" || kind === "merge_rows"}
      {#if kind === "merge_columns"}<label class="row"><input type="checkbox" bind:checked={skipBlanks} /> Leave out blank cells</label>{/if}
    {/if}

    {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
    <div class="buttons">
      <button type="button" class="primary" onclick={ok}>{kind === "show_duplicates" ? "Show" : kind === "pivot" || kind === "unpivot" ? "Make table" : "OK"}</button>
      <button type="button" onclick={onClose}>Cancel</button>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgb(0 0 0 / 0.35);
    display: grid;
    place-items: center;
    z-index: 50;
    padding: 16px;
  }
  .dialog {
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 18px 20px 14px;
    width: min(560px, 100%);
    max-height: 90vh;
    overflow: auto;
    box-shadow: 0 12px 40px rgb(0 0 0 / 0.3);
    outline: none;
    font-size: 13px;
  }
  h2 {
    font-size: 16px;
    margin: 0 0 10px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 6px 0;
    flex-wrap: wrap;
  }
  .block {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 8px 0;
  }
  fieldset {
    border: 1px solid var(--border);
    border-radius: 6px;
    margin: 8px 0;
    padding: 6px 10px;
  }
  .cols .list {
    max-height: 180px;
    overflow: auto;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 2px 10px;
  }
  .cols label {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  select,
  input[type="text"],
  input[type="number"],
  textarea,
  button {
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 3px 8px;
  }
  select {
    max-width: 260px;
  }
  input[type="number"] {
    width: 70px;
  }
  textarea {
    width: 100%;
    box-sizing: border-box;
    font-family: ui-monospace, monospace;
    resize: vertical;
  }
  button {
    cursor: pointer;
  }
  button.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-ink);
    padding: 6px 16px;
  }
  .note {
    color: var(--muted);
    font-size: 12px;
    margin: 6px 0;
  }
  .problem {
    color: #c0392b;
    margin: 6px 0;
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 10px;
  }
</style>
