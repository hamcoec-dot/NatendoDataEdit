<script lang="ts">
  // What changed since a checkpoint: the counts first, then the changed cells a page at a time (was / now; a click goes
  // to the cell), and a report to save. It runs only when asked; a big change (Replace all over every row) is a job
  // with progress and Stop. Rows are followed by their id through sorts and deletes, and columns by their identity
  // through inserts, deletes and moves.
  import { backend, errorText, type Checkpoint, type SinceCell, type SinceView } from "./backend";
  import FormShell from "./FormShell.svelte";

  interface Props {
    doc: number;
    checkpoint: Checkpoint;
    /** The first row holds titles. */
    header: boolean;
    /** Column titles now. */
    titles: string[];
    onGo: (row: number, col: number) => void;
    /** Stop the comparison while it is running. */
    onStop: () => void;
    onClose: () => void;
  }
  let { doc, checkpoint, header, titles, onGo, onStop, onClose }: Props = $props();

  const PAGE = 50;
  let view = $state<SinceView | null>(null);
  let page = $state(0);
  let problem = $state("");
  let busy = $state(true);
  let saved = $state("");

  async function load(p: number): Promise<void> {
    busy = true;
    try {
      view = await backend().checkpointSince(doc, checkpoint.id, header, p * PAGE, PAGE);
      page = p;
      problem = "";
    } catch (e) {
      problem = errorText(e);
    }
    busy = false;
  }
  $effect(() => {
    void load(0);
  });

  async function report(): Promise<void> {
    try {
      const n = await backend().checkpointSinceReport(doc, checkpoint.id, header);
      saved = n === null ? "" : `Saved ${n.toLocaleString()} ${n === 1 ? "line" : "lines"}.`;
    } catch (e) {
      problem = errorText(e);
    }
  }

  const plural = (n: number, one: string, many = `${one}s`) => `${n.toLocaleString()} ${n === 1 ? one : many}`;
  const title = (c: number) => titles[c] ?? `Column ${c + 1}`;
  const shorten = (v: string) => (v.length > 80 ? `${v.slice(0, 80)}…` : v);
  const t = $derived(view?.totals);
  const nothing = $derived(!!t && !t.cells_changed && !t.rows_added && !t.rows_removed && !t.cols_added && !t.cols_removed && !t.cols_moved && !t.reordered);
  const pages = $derived(t ? Math.max(1, Math.ceil(t.entries / PAGE)) : 1);
  /** A column taken out: its title at the checkpoint, or where it was. */
  const gone = (c: { at: number; title: string }) => c.title || `Column ${c.at + 1} (at the checkpoint)`;
  const list = (names: string[]) => (names.length > 12 ? `${names.slice(0, 12).join(", ")} and ${(names.length - 12).toLocaleString()} more` : names.join(", "));
  const columnOf = (c: SinceCell) =>
    c.kind === "cell" ? title(c.col ?? 0) : c.kind === "col_removed" ? gone(t!.removed_cols[c.removed_col ?? 0]!) : "(whole row)";
</script>

<FormShell title={`What changed since “${checkpoint.name}”`} wide onClose={() => { if (busy) onStop(); onClose(); }}>
  <div data-testid="since" aria-live="polite">
    {#if problem}
      <p class="problem" role="alert">{problem}</p>
    {:else if busy && !view}
      <p class="note">Comparing with the checkpoint… <button type="button" onclick={onStop}>Stop</button></p>
    {:else if view && t}
      {#if nothing}
        <p data-testid="since-none">Nothing has changed since this checkpoint ({plural(t.steps, "step")}).</p>
      {:else}
        <ul class="summary" data-testid="since-summary">
          {#if t.cells_changed}<li>{plural(t.cells_changed, "cell")} changed in {plural(t.rows_changed, "row")}</li>{/if}
          {#if t.rows_added}<li>{plural(t.rows_added, "row")} added</li>{/if}
          {#if t.rows_removed}<li>{plural(t.rows_removed, "row")} taken out</li>{/if}
          {#if t.cols_added}<li>{plural(t.cols_added, "column")} added: {list(t.added_cols.map(title))}</li>{/if}
          {#if t.cols_removed}<li>{plural(t.cols_removed, "column")} taken out: {list(t.removed_cols.map(gone))}</li>{/if}
          {#if t.cols_moved}<li>Columns moved: {list(t.moved_cols.map(title))}</li>{/if}
          {#if t.reordered}<li>Rows sorted or moved</li>{/if}
          <li class="note">{plural(t.steps, "step")} since the checkpoint.</li>
        </ul>
      {/if}
      {#if view.cells.length}
        <div class="table" role="table" aria-label="What changed">
          <div class="head" role="row"><span>Row</span><span>Column</span><span>Was</span><span>Now</span></div>
          {#each view.cells as c, i (i)}
            <button type="button" class="line" role="row" disabled={c.row === null} onclick={() => c.row !== null && onGo(c.row, c.col ?? 0)} title={c.row === null ? "This row is not in the table any more" : "Go to this row"}>
              <span>{c.row === null ? "—" : (c.row + 1).toLocaleString()}</span><span>{columnOf(c)}</span><span class:old={c.kind !== "row_added"}>{shorten(c.old) || "(blank)"}</span><span>{shorten(c.new) || "(blank)"}</span>
            </button>
          {/each}
        </div>
        <div class="pager" data-testid="since-pager">
          <button type="button" class="small" disabled={busy || page === 0} onclick={() => void load(page - 1)}>Previous {PAGE}</button>
          <span class="note">Lines {(page * PAGE + 1).toLocaleString()} to {Math.min(t.entries, (page + 1) * PAGE).toLocaleString()} of {t.entries.toLocaleString()} (page {page + 1} of {pages})</span>
          <button type="button" class="small" disabled={busy || page + 1 >= pages} onclick={() => void load(page + 1)}>Next {PAGE}</button>
        </div>
      {/if}
    {/if}
    {#if saved}<p class="note" role="status">{saved}</p>{/if}
  </div>
  {#snippet buttons()}
    <button type="button" class="primary" data-enter disabled={!view || !view.totals.entries || busy} onclick={() => void report()}>Save as a report…</button>
    <button type="button" onclick={() => { if (busy) onStop(); onClose(); }}>Close</button>
  {/snippet}
</FormShell>

<style>
  .summary {
    margin: 4px 0 8px 1.2em;
    padding: 0;
  }
  .table {
    max-height: 300px;
    overflow: auto;
    border: 1px solid var(--border, #8884);
    border-radius: 4px;
  }
  .head,
  .line {
    display: grid;
    grid-template-columns: 6em 12em 1fr 1fr;
    gap: 8px;
    padding: 3px 8px;
    text-align: left;
    width: 100%;
    font: inherit;
    background: none;
    border: 0;
    color: inherit;
  }
  .head {
    font-weight: 600;
    position: sticky;
    top: 0;
    background: var(--panel, #8881);
  }
  .line:hover:not(:disabled) {
    background: var(--hover, #8882);
    cursor: pointer;
  }
  .line:disabled {
    cursor: default;
  }
  .line span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .old {
    opacity: 0.7;
    text-decoration: line-through;
  }
  .pager {
    display: flex;
    gap: 8px;
    align-items: center;
    margin-top: 6px;
  }
  .note {
    color: var(--muted);
    font-size: 12px;
  }
  .problem {
    color: var(--danger, #b00020);
  }
</style>
