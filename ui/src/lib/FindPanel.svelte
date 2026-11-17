<script lang="ts">
  // Find and replace bar. Searches run in the engine (safe regular
  // expressions, whole files of any size); results select cells in the grid.
  import { backend, errorText, type DocInfo, type FindAll, type FindSpec, type ReplaceReport } from "./backend";
  import type { Tab } from "./tab.svelte";
  import { findSpecOf, type FindOptions } from "./filters";
  import { rectOf } from "./selection";

  interface Props {
    tab: Tab;
    replace: boolean;
    options: FindOptions;
    /** Runs a long engine call with progress shown and Cancel offered. */
    run: <T>(label: string, f: () => Promise<T>) => Promise<T | null>;
    onSelect: (row: number, col: number) => void;
    onChanged: (info: DocInfo) => void;
    onClose: () => void;
  }
  let { tab, replace = $bindable(), options = $bindable(), run, onSelect, onChanged, onClose }: Props = $props();

  let findInput = $state<HTMLInputElement>();
  let replacement = $state("");
  let status = $state("");
  let results = $state<FindAll | null>(null);
  let pending = $state<ReplaceReport | null>(null);

  export function focus(): void {
    findInput?.focus();
    findInput?.select();
  }

  $effect(() => {
    void replace;
    focus();
  });

  /** The search as set up now. A one-cell "selection" scope searches all. */
  function spec(): FindSpec {
    const r = rectOf(tab.sel);
    const o = { ...options };
    if (o.scope === "selection" && r.rows * r.cols <= 1) o.scope = "all";
    return findSpecOf(o, { row: r.row, rows: r.rows, col: r.col, cols: r.cols }, tab.header);
  }

  function ready(): boolean {
    if (options.text === "") {
      status = "Type what to look for.";
      focus();
      return false;
    }
    return true;
  }

  export async function next(backward = false): Promise<void> {
    if (!ready()) return;
    const f = tab.cell;
    try {
      const hit = await run("Searching", () => backend().findNext(tab.id, spec(), [f.row, f.col], backward));
      if (hit === null) return;
      if (!hit) {
        status = "No matches.";
        return;
      }
      onSelect(hit.row, hit.col);
      status = hit.wrapped ? `Found (search started over from the ${backward ? "end" : "top"}).` : "";
    } catch (e) {
      status = errorText(e);
    }
  }

  async function findAll(): Promise<void> {
    if (!ready()) return;
    try {
      const r = await run("Searching", () => backend().findAll(tab.id, spec()));
      if (!r) return;
      results = r;
      status = r.cells === 0 ? "No matches." : "";
    } catch (e) {
      status = errorText(e);
    }
  }

  /** Replace in the active cell if it matches, then go to the next match. */
  async function replaceOne(): Promise<void> {
    if (!ready()) return;
    const f = tab.cell;
    const one: FindSpec = { ...spec(), scope: { kind: "range", row: f.row, rows: 1, col: f.col, cols: 1 }, skip_first: false };
    try {
      const r = await backend().replaceAll(tab.id, one, replacement);
      if (r.info) onChanged(r.info);
    } catch (e) {
      status = errorText(e);
      return;
    }
    await next(false);
  }

  async function previewAll(): Promise<void> {
    if (!ready()) return;
    try {
      const r = await run("Looking for matches", () => backend().replacePreview(tab.id, spec(), replacement));
      if (!r) return;
      if (r.cells === 0) {
        status = "Nothing to replace.";
        pending = null;
        return;
      }
      pending = r;
      status = "";
    } catch (e) {
      status = errorText(e);
    }
  }

  async function replaceAll(): Promise<void> {
    pending = null;
    try {
      const r = await run("Replacing", () => backend().replaceAll(tab.id, spec(), replacement));
      if (!r) return;
      if (r.info) onChanged(r.info);
      status = `Replaced ${r.hits.toLocaleString()} match${r.hits === 1 ? "" : "es"} in ${r.cells.toLocaleString()} cell${r.cells === 1 ? "" : "s"}. Undo (Ctrl+Z) reverts it all.`;
      results = null;
    } catch (e) {
      status = errorText(e);
    }
  }

  function onKey(e: KeyboardEvent, inReplace: boolean): void {
    if (e.key === "Enter") {
      e.preventDefault();
      e.stopPropagation();
      if (inReplace && !e.shiftKey) void replaceOne();
      else void next(e.shiftKey);
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onClose();
    }
  }

  function title(c: number): string {
    const t = tab.header ? tab.cache.get(0, c) : undefined;
    return t ? `${t} (column ${c + 1})` : `Column ${c + 1}`;
  }
</script>

<div class="find" role="search" aria-label="Find and replace">
  <div class="line">
    <button type="button" class="twist" aria-label={replace ? "Hide replace" : "Show replace"} title="Replace (Ctrl+H)" onclick={() => (replace = !replace)}>{replace ? "▾" : "▸"}</button>
    <input bind:this={findInput} class="text" aria-label="Find" placeholder="Find" bind:value={options.text} onkeydown={(e) => onKey(e, false)} oninput={() => { results = null; pending = null; status = ""; }} />
    <button type="button" title="Find previous (Shift+F3, Shift+Enter)" aria-label="Find previous" onclick={() => void next(true)}>↑</button>
    <button type="button" title="Find next (F3, Enter)" aria-label="Find next" onclick={() => void next(false)}>↓</button>
    <button type="button" onclick={() => void findAll()}>Find all</button>
    <select aria-label="Pattern kind" bind:value={options.kind}>
      <option value="plain">Plain text</option>
      <option value="wildcard">Wildcards (* ? ~)</option>
      <option value="regex">Regular expression</option>
    </select>
    <label><input type="checkbox" bind:checked={options.matchCase} /> Match case</label>
    <label><input type="checkbox" bind:checked={options.wholeCell} /> Whole cell</label>
    <select aria-label="Search in" bind:value={options.scope}>
      <option value="all">Whole file</option>
      <option value="selection">Selection</option>
      <option value="column">Selected columns</option>
    </select>
    <span class="spacer"></span>
    <button type="button" class="x" aria-label="Close find" title="Close (Esc)" onclick={onClose}>✕</button>
  </div>
  {#if replace}
    <div class="line">
      <span class="twist"></span>
      <input class="text" aria-label="Replace with" placeholder={options.kind === "regex" ? "Replace with ($1 = first group)" : "Replace with"} bind:value={replacement} onkeydown={(e) => onKey(e, true)} />
      <button type="button" onclick={() => void replaceOne()}>Replace</button>
      <button type="button" onclick={() => void previewAll()}>Replace all…</button>
    </div>
  {/if}
  {#if pending}
    <div class="confirm" role="alert">
      <div>
        Replace {pending.hits.toLocaleString()} match{pending.hits === 1 ? "" : "es"} in {pending.cells.toLocaleString()} cell{pending.cells === 1 ? "" : "s"}?
        It is one step to undo.
      </div>
      <table>
        <tbody>
          {#each pending.preview.slice(0, 8) as c, i (i)}
            <tr><td class="where">Row {c.number + 1}, column {c.col + 1}</td><td class="before">{c.before}</td><td>→</td><td class="after">{c.after}</td></tr>
          {/each}
        </tbody>
      </table>
      <div class="line">
        <button type="button" class="primary" onclick={() => void replaceAll()}>Replace all</button>
        <button type="button" onclick={() => (pending = null)}>Cancel</button>
      </div>
    </div>
  {/if}
  {#if status}<div class="status" role="status">{status}</div>{/if}
  {#if results && results.cells > 0}
    <div class="results">
      <div class="rhead">
        {results.cells.toLocaleString()} cell{results.cells === 1 ? "" : "s"} match{results.cells === 1 ? "es" : ""}{results.hits.length < results.cells ? ` (first ${results.hits.length.toLocaleString()} listed)` : ""}
        <button type="button" class="x" aria-label="Close results" onclick={() => (results = null)}>✕</button>
      </div>
      <div class="rlist">
        {#each results.hits.slice(0, 2000) as h, i (i)}
          {@const chars = [...h.text]}
          <button type="button" class="hit" onclick={() => onSelect(h.row, h.col)}>
            <span class="where">{title(h.col)}, row {h.number + 1}</span>
            <span class="snip">{chars.slice(0, h.start).join("")}<mark>{chars.slice(h.start, h.start + h.len).join("")}</mark>{chars.slice(h.start + h.len).join("")}</span>
          </button>
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  .find {
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    padding: 4px 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 13px;
  }
  .line {
    display: flex;
    gap: 6px;
    align-items: center;
    flex-wrap: wrap;
  }
  .text {
    flex: 0 1 320px;
    min-width: 160px;
  }
  input,
  select,
  button {
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 2px 8px;
  }
  input[type="checkbox"] {
    padding: 0;
  }
  button {
    cursor: pointer;
  }
  button.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-ink);
  }
  .twist {
    width: 24px;
    padding: 2px 0;
    border: none;
    background: none;
  }
  .x {
    border: none;
    background: none;
  }
  .spacer {
    flex: 1;
  }
  .status {
    color: var(--muted);
    padding-left: 30px;
  }
  .confirm {
    margin-left: 30px;
    border: 1px solid var(--accent);
    border-radius: 6px;
    padding: 6px 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  table {
    border-collapse: collapse;
    font-size: 12px;
  }
  td {
    padding: 1px 6px;
    max-width: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .before {
    color: var(--muted);
    text-decoration: line-through;
  }
  .where {
    color: var(--muted);
    white-space: nowrap;
  }
  .results {
    margin-left: 30px;
    border: 1px solid var(--border);
    border-radius: 6px;
  }
  .rhead {
    display: flex;
    justify-content: space-between;
    padding: 2px 8px;
    color: var(--muted);
  }
  .rlist {
    max-height: 180px;
    overflow: auto;
  }
  .hit {
    display: flex;
    gap: 10px;
    width: 100%;
    border: none;
    border-top: 1px solid var(--border);
    border-radius: 0;
    background: none;
    text-align: left;
    padding: 2px 8px;
  }
  .hit:hover {
    background: var(--bg);
  }
  .snip {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  mark {
    background: var(--accent);
    color: var(--accent-ink);
    border-radius: 2px;
  }
</style>
