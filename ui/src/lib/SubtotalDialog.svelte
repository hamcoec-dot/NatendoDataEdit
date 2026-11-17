<script lang="ts">
  // Subtotals: a new table with a total row after each run of equal values
  // in one column (sort by it first) and a grand total. The file is not
  // changed.
  import AllNone from "./AllNone.svelte";
  import { untrack } from "svelte";
  import type { TableOp } from "./backend";
  import FormShell from "./FormShell.svelte";
  import type { Total } from "./views";

  interface Props {
    titles: string[];
    col: number;
    header: boolean;
    onRun: (op: TableOp, header: boolean) => void;
    onClose: () => void;
  }
  let { titles, col, header, onRun, onClose }: Props = $props();

  let by = $state(untrack(() => col));
  let total = $state<Total>("sum");
  let cols = $state<number[]>([]);
  let myHeader = $state(untrack(() => header));
  let problem = $state("");

  function run(): void {
    if (!cols.length) {
      problem = "Tick at least one column to total.";
      return;
    }
    onRun({ op: "subtotals", spec: { by, cols: [...cols].sort((a, b) => a - b), total, keep_first: myHeader, read: { decimal_comma: false, date_order: "mdy" } } }, myHeader);
  }
</script>

<FormShell title="Subtotals" {onClose} onEnter={run}>
  <label class="row">At each change in <select bind:value={by}>{#each titles as t, c (c)}<option value={c}>{t}</option>{/each}</select></label>
  <label class="row"
    >Use
    <select bind:value={total}>
      <option value="sum">Sum</option>
      <option value="count">Count</option>
      <option value="average">Average</option>
      <option value="min">Smallest</option>
      <option value="max">Largest</option>
    </select>
  </label>
  <fieldset>
    <legend>Of the columns</legend>
    <AllNone count={titles.length} picked={cols} onChange={(v) => (cols = v)} skip={(c) => c === by} />
    <div class="list">
      {#each titles as t, c (c)}
        {#if c !== by}<label><input type="checkbox" checked={cols.includes(c)} onchange={() => (cols = cols.includes(c) ? cols.filter((x) => x !== c) : [...cols, c])} /> {t}</label>{/if}
      {/each}
    </div>
  </fieldset>
  <label class="row"><input type="checkbox" bind:checked={myHeader} /> The first row holds titles</label>
  <p class="note">Sort by the column first so equal values sit together. The result opens as a new table; this file does not change.</p>
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" class="primary" onclick={run}>Make subtotals</button>
    <button type="button" onclick={onClose}>Cancel</button>
  {/snippet}
</FormShell>

<style>
  .row {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .list {
    display: flex;
    flex-direction: column;
    max-height: 12rem;
    overflow: auto;
  }
  .note {
    color: var(--muted);
    font-size: 0.85rem;
  }
  .problem {
    color: var(--danger, #c00);
  }
</style>
