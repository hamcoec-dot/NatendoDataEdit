<script lang="ts">
  // Form view: one row at a time, each column as a labelled field. Changes
  // are applied when moving to another row or closing (one undo step per
  // row). Page Up / Page Down move between rows.
  import { untrack } from "svelte";
  import { backend, errorText, type DocInfo } from "./backend";
  import { parseTsv } from "./tsv";

  interface Props {
    doc: number;
    titles: string[];
    /** Columns in the document (fields shown are capped). */
    cols: number;
    /** First data row (1 when the first row holds titles). */
    firstRow: number;
    rows: number;
    start: number;
    onChanged: (d: DocInfo) => void;
    onClose: (row: number) => void;
  }
  let { doc, titles, cols, firstRow, rows, start, onChanged, onClose }: Props = $props();

  /** Most fields shown at once. */
  const MAX_FIELDS = 500;
  const shown = untrack(() => Math.min(cols, MAX_FIELDS, titles.length || cols));
  let row = $state(untrack(() => Math.max(firstRow, Math.min(start, rows - 1))));
  let total = $state(untrack(() => rows));
  let original = $state<string[]>([]);
  let values = $state<string[]>([]);
  let problem = $state("");
  let busy = $state(false);
  let box: HTMLDivElement;

  const dirty = $derived(values.some((v, i) => v !== original[i]));

  async function load(r: number): Promise<void> {
    problem = "";
    try {
      const text = await backend().copyCells(doc, r, 1, 0, shown);
      const line = parseTsv(text)[0] ?? [];
      original = Array.from({ length: shown }, (_, i) => line[i] ?? "");
      values = [...original];
      row = r;
    } catch (e) {
      problem = errorText(e);
    }
  }

  async function applyChanges(): Promise<boolean> {
    if (!dirty) return true;
    const cells: [number, number, string][] = [];
    values.forEach((v, i) => {
      if (v !== original[i]) cells.push([row, i, v]);
    });
    try {
      const d = await backend().setCells(doc, cells);
      onChanged(d);
      total = d.rows;
      original = [...values];
      return true;
    } catch (e) {
      problem = errorText(e);
      return false;
    }
  }

  async function go(r: number): Promise<void> {
    if (busy || r < firstRow || r >= total) return;
    busy = true;
    if (await applyChanges()) await load(r);
    busy = false;
  }

  async function newRow(): Promise<void> {
    if (busy) return;
    busy = true;
    try {
      if (await applyChanges()) {
        const d = await backend().change(doc, { op: "insert_rows", at: total, count: 1 });
        onChanged(d);
        total = d.rows;
        await load(total - 1);
      }
    } catch (e) {
      problem = errorText(e);
    }
    busy = false;
  }

  async function deleteRow(): Promise<void> {
    if (busy) return;
    busy = true;
    try {
      const d = await backend().change(doc, { op: "delete_rows", at: row, count: 1 });
      onChanged(d);
      total = d.rows;
      if (total <= firstRow) {
        onClose(firstRow);
        return;
      }
      await load(Math.min(row, total - 1));
    } catch (e) {
      problem = errorText(e);
    }
    busy = false;
  }

  async function close(): Promise<void> {
    if (await applyChanges()) onClose(row);
  }

  function onKey(e: KeyboardEvent): void {
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      // Escape drops unapplied changes to this row, like Excel's form.
      onClose(row);
    } else if (e.key === "PageDown" || (e.key === "Enter" && !(e.target instanceof HTMLTextAreaElement) && !(e.target instanceof HTMLButtonElement))) {
      e.preventDefault();
      void go(row + 1);
    } else if (e.key === "PageUp") {
      e.preventDefault();
      void go(row - 1);
    }
  }

  $effect(() => {
    void untrack(() => load(row)).then(() => box.querySelector<HTMLElement>("input, textarea")?.focus());
  });
</script>

<div class="backdrop" role="presentation">
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="form-title" tabindex="-1" bind:this={box} onkeydown={onKey}>
    <h2 id="form-title">Row {(row + 1).toLocaleString()} of {total.toLocaleString()}</h2>
    <div class="fields">
      {#each values as _, i (i)}
        <label>
          <span class="name" title={titles[i] ?? `Column ${i + 1}`}>{titles[i] ?? `Column ${i + 1}`}</span>
          {#if (values[i] ?? "").includes("\n") || (values[i] ?? "").length > 80}
            <textarea rows="3" bind:value={values[i]} class:changed={values[i] !== original[i]}></textarea>
          {:else}
            <input type="text" bind:value={values[i]} class:changed={values[i] !== original[i]} />
          {/if}
        </label>
      {/each}
    </div>
    {#if cols > shown}<p class="note">The first {shown.toLocaleString()} of {cols.toLocaleString()} columns are shown.</p>{/if}
    {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
    <div class="buttons">
      <button type="button" onclick={() => go(firstRow)} disabled={busy || row <= firstRow} title="First row">⏮</button>
      <button type="button" onclick={() => go(row - 1)} disabled={busy || row <= firstRow} title="Previous row (Page Up)">◀ Previous</button>
      <button type="button" onclick={() => go(row + 1)} disabled={busy || row >= total - 1} title="Next row (Page Down or Enter)">Next ▶</button>
      <button type="button" onclick={() => go(total - 1)} disabled={busy || row >= total - 1} title="Last row">⏭</button>
      <span class="gap"></span>
      <button type="button" onclick={newRow} disabled={busy}>New row</button>
      <button type="button" class="danger" onclick={deleteRow} disabled={busy}>Delete row</button>
      <button type="button" onclick={() => load(row)} disabled={busy || !dirty}>Undo changes</button>
      <button type="button" class="primary" onclick={close} disabled={busy}>Close</button>
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
    width: min(640px, 100%);
    max-height: 90vh;
    display: flex;
    flex-direction: column;
    box-shadow: 0 12px 40px rgb(0 0 0 / 0.3);
    outline: none;
    font-size: 13px;
  }
  h2 {
    font-size: 16px;
    margin: 0 0 10px;
  }
  .fields {
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-right: 4px;
  }
  label {
    display: grid;
    grid-template-columns: minmax(90px, 30%) 1fr;
    gap: 10px;
    align-items: start;
  }
  .name {
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding-top: 4px;
  }
  input,
  textarea,
  button {
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 3px 8px;
  }
  textarea {
    resize: vertical;
  }
  .changed {
    border-color: var(--accent);
  }
  button {
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.45;
    cursor: default;
  }
  button.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-ink);
  }
  button.danger {
    border-color: #c0392b;
    color: #c0392b;
  }
  .buttons {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 12px;
    align-items: center;
  }
  .gap {
    flex: 1;
  }
  .note {
    color: var(--muted);
    font-size: 12px;
  }
  .problem {
    color: #c0392b;
  }
</style>
