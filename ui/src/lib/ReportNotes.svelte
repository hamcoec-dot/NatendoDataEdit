<script lang="ts">
  // The Report notes panel (C4): the lines above and below the table that are
  // not rows of it (a report's title, its date, a total line), read-only, with
  // Copy; the "Label: value" lines listed as report info; the switch that
  // keeps them in saves; and Put back into the table. Text only (no HTML).
  import { backend, errorText, type ReportNotes } from "./backend";
  import type { Tab } from "./tab.svelte";

  interface Props {
    tab: Tab;
    onPutBack: () => void;
    onKeep: (keep: boolean) => void;
  }
  let { tab, onPutBack, onKeep }: Props = $props();

  let notes = $state<ReportNotes | null>(null);
  let problem = $state("");
  let copied = $state("");
  $effect(() => {
    void tab.info.history_pos;
    void tab.info.step_mark;
    void tab.info.notes?.keep;
    const doc = tab.id;
    backend()
      .reportNotes(doc)
      .then((n) => {
        if (tab.id === doc) {
          notes = n;
          problem = "";
        }
      })
      .catch((e) => (problem = errorText(e)));
  });
  const parts = $derived(
    notes
      ? ([
          ["Above the table, from the file", notes.above_file, notes.above_file_lines],
          ["Set aside above the table", notes.above_set, notes.above_set_lines],
          ["Set aside below the table", notes.below_set, notes.below_set_lines],
          ["Below the table, from the file", notes.below_file, notes.below_file_lines],
        ] as [string, string, number][]).filter(([, , n]) => n > 0)
      : [],
  );
  const lines = (n: number) => `${n.toLocaleString()} ${n === 1 ? "line" : "lines"}`;
  async function copy(what: string, text: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
      copied = what;
      window.setTimeout(() => (copied = ""), 1500);
    } catch (e) {
      problem = errorText(e);
    }
  }
</script>

<section class="notes" aria-label="Report notes">
  <h3>Report notes</h3>
  {#if problem}
    <p class="note" role="alert">{problem}</p>
  {/if}
  {#if notes && parts.length === 0}
    <p class="note">This file has no report notes: lines above or below the table that are not rows of it, such as a report's title or a total line.</p>
    <p class="note">Right-click the row that holds the column titles › <b>Headers start here</b> sets the rows above it aside. Select the last rows › <b>Set aside rows as footer notes</b> does the same below. Saving writes them back where they were.</p>
  {:else if notes}
    <label class="keep">
      <input type="checkbox" checked={notes.keep} disabled={tab.info.read_only} onchange={(e) => onKeep((e.currentTarget as HTMLInputElement).checked)} />
      Save report notes with the file
    </label>
    {#if !notes.keep}
      <p class="note warn">Saving leaves them out. They stay here until then: switch it back on and nothing is lost.</p>
    {/if}
    {#each parts as [title, text, n] (title)}
      <div class="part">
        <div class="head">
          <span>{title} ({lines(n)})</span>
          <button type="button" class="small" onclick={() => void copy(title, text)}>{copied === title ? "Copied" : "Copy"}</button>
        </div>
        <pre aria-label={title}>{text}</pre>
      </div>
    {/each}
    {#if notes.cut}
      <p class="note">Long notes are shown in part; saving writes them whole.</p>
    {/if}
    {#if notes.info.length}
      <h4>Report info</h4>
      <dl>
        {#each notes.info as [label, value], i (i)}
          <dt>{label}</dt><dd>{value}</dd>
        {/each}
      </dl>
    {/if}
    <p>
      <button type="button" disabled={tab.info.read_only || tab.info.filtered} title={tab.info.filtered ? "Show all rows first" : "The lines become rows again (one undo step)"} onclick={onPutBack}>Put back into the table</button>
    </p>
    <p class="note">Rows set aside go back exactly as they were; lines from the file are read with its separator, and lines with another number of fields become ragged rows.</p>
  {/if}
</section>

<style>
  .notes h3 {
    margin: 0 0 6px;
  }
  .keep {
    display: flex;
    gap: 6px;
    align-items: center;
    margin: 4px 0 8px;
  }
  .part {
    margin: 8px 0;
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-weight: 600;
    margin-bottom: 2px;
  }
  pre {
    margin: 0;
    max-height: 12em;
    overflow: auto;
    white-space: pre-wrap;
    word-break: break-word;
    border: 1px solid var(--border);
    padding: 4px 6px;
    font-size: 0.9em;
  }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 2px 8px;
    margin: 0;
  }
  dt {
    font-weight: 600;
  }
  dd {
    margin: 0;
    word-break: break-word;
  }
  .note {
    color: var(--muted);
    font-size: 0.92em;
  }
  .warn {
    color: var(--text);
    font-weight: 600;
  }
</style>
