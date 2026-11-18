<script lang="ts" module>
  export type CompareAction = "mark" | "show" | "column";
</script>

<script lang="ts">
  // Data › Compare two columns: for each row, are the two cells the same,
  // different, or is only one filled? Light up the rows that differ, show
  // only them, or add a column saying which.
  import { untrack } from "svelte";
  import FormShell from "./FormShell.svelte";
  import type { CompareCols } from "./backend";

  interface Props {
    /** Column titles (by position). */
    titles: string[];
    /** Hidden columns are not offered. */
    hidden: number[];
    first: number;
    second: number;
    header: boolean;
    readOnly: boolean;
    onRun: (spec: CompareCols, action: CompareAction) => void;
    onClose: () => void;
  }
  let { titles, hidden, first, second, header, readOnly, onRun, onClose }: Props = $props();
  const init = untrack(() => ({ first, second, header }));
  let a = $state(init.first);
  let b = $state(init.second);
  let caseCounts = $state(false);
  let spaces = $state(false);
  let numbers = $state(true);
  let action = $state<CompareAction>("mark");
  const shown = $derived(titles.map((t, i) => [i, t] as const).filter(([i]) => !hidden.includes(i)));
  let done = false;
  function run(): void {
    if (done || a === b) return;
    done = true;
    onRun({ a, b, case: caseCounts, spaces, numbers, keep_first: init.header, title: "", read: { decimal_comma: false, date_order: "mdy" } }, action);
  }
</script>

<FormShell title="Compare two columns" {onClose} onEnter={run}>
  <div class="pick">
    <label
      >First
      <select bind:value={a}>
        {#each shown as [i, t] (i)}<option value={i}>{t}</option>{/each}
      </select></label
    >
    <label
      >Second
      <select bind:value={b}>
        {#each shown as [i, t] (i)}<option value={i}>{t}</option>{/each}
      </select></label
    >
  </div>
  {#if a === b}<p class="warn" role="note">Pick two different columns.</p>{/if}
  <fieldset>
    <legend>Count as the same</legend>
    <label><input type="checkbox" checked={!caseCounts} onchange={(e) => (caseCounts = !(e.currentTarget as HTMLInputElement).checked)} /> Upper and lower case (Main St = MAIN ST)</label>
    <label><input type="checkbox" checked={!spaces} onchange={(e) => (spaces = !(e.currentTarget as HTMLInputElement).checked)} /> Extra spaces</label>
    <label><input type="checkbox" bind:checked={numbers} /> Numbers of equal value (5 = 5.00 = $5)</label>
  </fieldset>
  <fieldset>
    <legend>Then</legend>
    <label><input type="radio" name="cmp" value="mark" bind:group={action} /> Light up the rows that differ and go to the first</label>
    <label><input type="radio" name="cmp" value="show" bind:group={action} /> Show only the rows that differ (a filter)</label>
    <label><input type="radio" name="cmp" value="column" bind:group={action} disabled={readOnly} /> Add a column: Same, Different, Only in first, Only in second</label>
  </fieldset>
  <p class="muted">Shown rows only{init.header ? "; the title row is left out" : ""}. Two empty cells count as the same.</p>
  {#snippet buttons()}
    <button type="button" onclick={onClose}>Cancel</button>
    <button type="button" class="primary" onclick={run} disabled={a === b}>Compare</button>
  {/snippet}
</FormShell>

<style>
  .pick {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
    margin-bottom: 10px;
  }
  .pick select {
    max-width: 220px;
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
  .muted {
    color: var(--muted);
    font-size: 12px;
  }
  .warn {
    color: var(--danger, #c0392b);
  }
</style>
