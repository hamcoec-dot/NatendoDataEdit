<script lang="ts">
  // List of my changes (Milestone 14): every cell that differs from the
  // file as it was opened or last saved, and rows and columns added,
  // deleted, moved or sorted. Click a cell change to go to it.
  import type { ChangeList } from "./backend";
  import FormShell from "./FormShell.svelte";

  interface Props {
    list: ChangeList | null;
    /** Column titles now. */
    titles: string[];
    showEdits: boolean;
    onGo: (row: number, col: number) => void;
    onSave: () => void;
    onShowEdits: (on: boolean) => void;
    /** Stop comparing (while the list is being made). */
    onStop: () => void;
    onClose: () => void;
  }
  let { list, titles, showEdits, onGo, onSave, onShowEdits, onStop, onClose }: Props = $props();

  const plural = (n: number, one: string, many = `${one}s`) => `${n.toLocaleString()} ${n === 1 ? one : many}`;
  const title = (c: number) => titles[c] ?? `Column ${c + 1}`;
  const empty = $derived(
    !!list && !list.cells_total && !list.added_total && !list.deleted_total && !list.added_cols.length && !list.deleted_cols.length && !list.moved_cols && !list.reordered,
  );
  const shorten = (v: string) => (v.length > 80 ? `${v.slice(0, 80)}…` : v);
</script>

<FormShell title="List of my changes" wide {onClose}>
  {#if list === null}
    <p class="note">Comparing with the file… <button type="button" onclick={onStop}>Stop</button></p>
  {:else if empty}
    <p>No changes since the file was opened or last saved.</p>
  {:else}
    <ul class="summary">
      {#if list.cells_total}<li>{plural(list.cells_total, "cell")} changed</li>{/if}
      {#if list.added_total}<li>{plural(list.added_total, "row")} added</li>{/if}
      {#if list.deleted_total}<li>{plural(list.deleted_total, "row")} deleted (rows {list.deleted_rows.slice(0, 8).map((r) => (r + 1).toLocaleString()).join(", ")}{list.deleted_total > 8 ? ", …" : ""} of the file)</li>{/if}
      {#if list.added_cols.length}<li>{plural(list.added_cols.length, "column")} added: {list.added_cols.map(title).join(", ")}</li>{/if}
      {#if list.deleted_cols.length}<li>{plural(list.deleted_cols.length, "column")} of the file deleted (columns {list.deleted_cols.map((c) => c + 1).join(", ")})</li>{/if}
      {#if list.moved_cols}<li>Columns moved</li>{/if}
      {#if list.reordered}<li>Rows sorted or moved</li>{/if}
    </ul>
    {#if list.cells.length}
      <div class="table" role="table" aria-label="Changed cells">
        <div class="head" role="row"><span>Row</span><span>Column</span><span>Was</span><span>Now</span></div>
        {#each list.cells as c, i (i)}
          <button type="button" class="line" role="row" onclick={() => onGo(c.row, c.col)} title="Go to this cell">
            <span>{(c.row + 1).toLocaleString()}</span><span>{title(c.col)}</span><span class="old">{shorten(c.old) || "(blank)"}</span><span>{shorten(c.new) || "(blank)"}</span>
          </button>
        {/each}
      </div>
      {#if list.cells_total > list.cells.length}<p class="note">The first {list.cells.length.toLocaleString()} are shown; the report has them all.</p>{/if}
    {/if}
  {/if}
  <label class="row"><input type="checkbox" checked={showEdits} onchange={(e) => onShowEdits((e.currentTarget as HTMLInputElement).checked)} /> Show my edits in the table (a corner mark on changed cells)</label>
  {#snippet buttons()}
    <button type="button" class="primary" onclick={onSave} disabled={!list || empty}>Save as a report…</button>
    <button type="button" onclick={onClose}>Close</button>
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
  .line:hover {
    background: var(--hover, #8882);
    cursor: pointer;
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
</style>
