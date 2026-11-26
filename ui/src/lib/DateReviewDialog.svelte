<script lang="ts" module>
  import type { CleanSpec } from "./clean";

  /** What the person chose to do. */
  export interface DateReviewDone {
    spec: CleanSpec;
    /** "Type each": shown row and the new text. */
    typed: [number, string][];
    /** Set the column to this day order afterwards (null: leave the column's type). */
    remember: "mdy" | "dmy" | null;
  }
</script>

<script lang="ts">
  // The date review screen: what the column holds, what to do with the dates both orders can read, a preview with
  // exact counts, and Apply. Nothing is guessed: what is not chosen stays exactly as typed.
  import { untrack } from "svelte";
  import { backend, errorText, type DateScan } from "./backend";
  import { localNow, type CleanPreview } from "./clean";
  import { TWO_DIGIT_RULE, dateReadings, dateSummary, firstChoice, noClue, reviewSpec, type DateChoice } from "./dates";
  import FormShell from "./FormShell.svelte";

  interface Props {
    doc: number;
    col: number;
    column: string;
    header: boolean;
    /** Counts over every row. */
    scan: DateScan;
    onApply: (done: DateReviewDone) => void;
    onClose: () => void;
  }
  let { doc, col, column, header, scan, onApply, onClose }: Props = $props();

  let choice = $state<DateChoice>(untrack(() => firstChoice(scan)));
  let compact = $state(false);
  let twoDigit = $state(false);
  let remember = $state(false);
  let preview = $state<CleanPreview | null>(null);
  let problem = $state("");
  /** "Type each": the first ambiguous cells, and what has been typed for them (by row). */
  let each = $state<[number, number, string][] | null>(null);
  let typed = $state<Record<number, string>>({});

  const spec = $derived(reviewSpec(col, header, choice, compact, twoDigit));
  const typedCells = $derived(
    choice === "each" ? (each ?? []).flatMap(([row, , was]): [number, string][] => (typed[row]?.trim() && typed[row] !== was ? [[row, typed[row]!]] : [])) : [],
  );
  const ready = $derived(choice !== "" && !!preview && preview.changed + typedCells.length > 0);

  // Preview, again after each choice.
  $effect(() => {
    const s = spec;
    let stale = false;
    preview = null;
    backend()
      .cleanPreviewPage(doc, s, localNow(), 0, 8)
      .then((p) => {
        if (stale) return;
        preview = p;
        problem = "";
      })
      .catch((e: unknown) => {
        if (!stale) problem = errorText(e);
      });
    return () => {
      stale = true;
    };
  });

  $effect(() => {
    if (choice !== "each" || each) return;
    backend()
      .dateList(doc, col, header)
      .then((list) => (each = list))
      .catch((e: unknown) => (problem = errorText(e)));
  });

  function apply(): void {
    if (!ready) return;
    onApply({ spec, typed: typedCells, remember: remember && (choice === "mdy" || choice === "dmy") ? choice : null });
  }

  const count = (v: number) => v.toLocaleString();
</script>

<FormShell title={`Review dates: ${column}`} wide {onClose}>
  <p data-testid="date-review-summary">{dateSummary(scan, column)}</p>

  {#if scan.ambiguous}
    <fieldset>
      <legend>{count(scan.ambiguous)} {scan.ambiguous === 1 ? "date" : "dates"} could be read two ways</legend>
      {#if noClue(scan)}<p class="warn">Nothing in this column tells the order; pick one.</p>{/if}
      {#if scan.month_first_only && scan.day_first_only}<p class="warn">This column also holds dates that can only be read month first and others only day first: one order for all may be wrong.</p>{/if}
      <label><input type="radio" name="amb" value="leave" bind:group={choice} /> Leave them as typed</label>
      <label><input type="radio" name="amb" value="mdy" bind:group={choice} /> Month first (03/04/2026 is 4 March)</label>
      <label><input type="radio" name="amb" value="dmy" bind:group={choice} /> Day first (03/04/2026 is 3 April)</label>
      <label><input type="radio" name="amb" value="each" bind:group={choice} /> Type each one</label>
      {#if choice === "mdy" || choice === "dmy"}
        <label class="sub"><input type="checkbox" bind:checked={remember} /> Remember this order for the column</label>
      {/if}
      {#if choice === "each"}
        {#if !each}
          <p>Listing them…</p>
        {:else}
          {#if scan.ambiguous > each.length}<p>The first {count(each.length)} are listed; the others stay as typed.</p>{/if}
          <div class="each" data-testid="date-each">
            <table>
              <tbody>
                {#each each as [row, number, was] (row)}
                  {@const two = dateReadings(was)}
                  <tr>
                    <th scope="row">Row {count(number + 1)}</th>
                    <td><code>{was}</code></td>
                    <td><input aria-label={`New date for row ${number + 1}`} value={typed[row] ?? ""} placeholder="as typed" oninput={(e) => (typed[row] = (e.currentTarget as HTMLInputElement).value)} /></td>
                    <td>
                      {#if two}
                        <button type="button" class="small" onclick={() => (typed[row] = two.mdy)}>{two.mdy}</button>
                        <button type="button" class="small" onclick={() => (typed[row] = two.dmy)}>{two.dmy}</button>
                      {/if}
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        {/if}
      {/if}
    </fieldset>
  {/if}

  {#if scan.compact}
    <label><input type="checkbox" bind:checked={compact} /> Also change 8-digit numbers that look like dates ({count(scan.compact)})</label>
  {/if}
  {#if scan.compact_numbers}
    <p>{count(scan.compact_numbers)} eight-digit {scan.compact_numbers === 1 ? "value is" : "values are"} taken as numbers and not changed.</p>
  {/if}
  {#if scan.two_digit_year}
    <label><input type="checkbox" bind:checked={twoDigit} /> Also change dates with a two-digit year ({count(scan.two_digit_year)}): {TWO_DIGIT_RULE}</label>
  {/if}

  <div class="preview" aria-live="polite" data-testid="date-review-preview">
    {#if preview}
      <p>
        <strong>{count(preview.changed)} {preview.changed === 1 ? "cell becomes" : "cells become"} like 2026-04-25.</strong>
        {count(Math.max(0, preview.left - typedCells.length))} stay as typed.
        {#if typedCells.length}{count(typedCells.length)} typed by you (undone together with the rest).{/if}
      </p>
      {#if preview.changes.length}
        <table>
          <thead><tr><th>Row</th><th>Before</th><th>After</th></tr></thead>
          <tbody>
            {#each preview.changes as c (c.row)}
              <tr><td>{count(c.number + 1)}</td><td><code>{c.before}</code></td><td><code>{c.after}</code></td></tr>
            {/each}
          </tbody>
        </table>
      {/if}
    {:else if !problem}
      <p>Counting…</p>
    {/if}
  </div>
  {#if problem}<p class="warn" role="alert">{problem}</p>{/if}

  {#snippet buttons()}
    <button type="button" data-enter disabled={!ready} title={choice === "" ? "Pick what to do with the ambiguous dates" : ""} onclick={apply}>Apply</button>
    <button type="button" onclick={onClose}>Cancel</button>
  {/snippet}
</FormShell>

<style>
  fieldset {
    border: 1px solid var(--border, #8884);
    margin: 0 0 0.5rem;
  }
  label {
    display: block;
  }
  .sub {
    margin-left: 1.5rem;
  }
  .each {
    max-height: 12rem;
    overflow: auto;
  }
  .preview table,
  .each table {
    border-collapse: collapse;
  }
  th,
  td {
    text-align: left;
    padding: 0 0.6rem 0.1rem 0;
    font-weight: normal;
  }
  thead th {
    font-weight: 600;
  }
  .warn {
    font-weight: 600;
  }
</style>
