<script lang="ts">
  // Record panel: the active cell's row as a list of fields. It follows the selection, can pin fields to the top,
  // search them, show hidden columns dimmed, flip to the next or previous row, jump to a cell and edit in place
  // (one undo step per field). It reads one row at a time and keeps nothing but what is on screen.
  import { untrack } from "svelte";
  import { backend, errorText, type DocInfo } from "./backend";
  import { prefs } from "./prefs.svelte";
  import { recordList, togglePin } from "./record";
  import RecordFields from "./RecordFields.svelte";
  import type { Tab } from "./tab.svelte";
  import { parseTsv } from "./tsv";

  interface Props {
    tab: Tab;
    titles: string[];
    /** Move the active cell (shown row, column). */
    onGo: (row: number, col: number) => void;
    onInfo: (info: DocInfo) => void;
  }
  let { tab, titles, onGo, onInfo }: Props = $props();

  /** Most fields read at once (as in the Form view). */
  const MAX_FIELDS = 500;
  const n = $derived(Math.min(tab.info.cols, MAX_FIELDS, titles.length || tab.info.cols));
  const row = $derived(tab.cell.row);
  const doc = $derived(tab.id);

  let values = $state<string[]>([]);
  let original = $state<string[]>([]);
  let loadedRow = $state(-1);
  let problem = $state("");
  let query = $state("");
  let editing = $state<number | null>(null);
  let seq = 0;

  // ---- reading the row ----
  $effect(() => {
    const r = row;
    const d = doc;
    const count = n;
    void tab.version;
    if (count <= 0 || r < 0 || r >= tab.info.rows) {
      untrack(() => {
        values = [];
        original = [];
        loadedRow = -1;
      });
      return;
    }
    const mine = ++seq;
    const frame = requestAnimationFrame(() => {
      backend()
        .copyCells(d, r, 1, 0, count)
        .then((text) => {
          if (mine !== seq) return;
          const line = parseTsv(text)[0] ?? [];
          const fresh = Array.from({ length: count }, (_, i) => line[i] ?? "");
          // A field being typed in keeps what was typed.
          const keep = editing;
          const typed = keep !== null ? values[keep] : "";
          original = fresh;
          values = fresh.map((v, i) => (i === keep && loadedRow === r ? (typed ?? v) : v));
          loadedRow = r;
          problem = "";
        })
        .catch((e: unknown) => {
          if (mine === seq) problem = errorText(e);
        });
    });
    return () => cancelAnimationFrame(frame);
  });

  // ---- what shows ----
  const showHidden = $derived(prefs.data.recordHidden === true);
  const list = $derived(
    recordList({
      n: values.length,
      titles,
      values: original,
      hidden: (c) => (void tab.version, tab.columns.isHidden(c)),
      showHidden,
      query,
      pins: tab.recordPins,
    }),
  );
  const protectedCols = $derived(new Set(tab.info.protected ?? []));
  const reason = (c: number): string => (tab.info.read_only ? "This file is locked, so nothing can be changed." : protectedCols.has(c) ? "This column is protected from changes." : "");
  const pos = $derived(tab.header && row === 0 ? "Title row" : `Row ${(row - tab.firstRow + 1).toLocaleString()} of ${Math.max(0, tab.info.rows - tab.firstRow).toLocaleString()} shown`);

  // ---- editing ----
  async function commit(c: number): Promise<void> {
    const text = values[c] ?? "";
    if (text === original[c] || reason(c) !== "") return;
    const at = row;
    try {
      const d = await backend().setCells(doc, [[at, c, text]]);
      original[c] = text;
      problem = "";
      onInfo(d);
    } catch (e) {
      problem = errorText(e);
      values[c] = original[c] ?? "";
    }
  }

  // ---- long values ----
  let big = $state<{ col: number; text: string; row: number } | null>(null);
  async function expand(c: number): Promise<void> {
    try {
      const full = await backend().cellText(doc, row, c);
      big = { col: c, text: full.text, row };
    } catch (e) {
      problem = errorText(e);
    }
  }
  async function saveBig(): Promise<void> {
    const b = big;
    if (!b) return;
    big = null;
    if (reason(b.col) !== "") return;
    try {
      const d = await backend().setCells(doc, [[b.row, b.col, b.text]]);
      onInfo(d);
    } catch (e) {
      problem = errorText(e);
    }
  }

  const flip = (d: number) => {
    const to = Math.max(tab.firstRow, Math.min(tab.info.rows - 1, row + d));
    if (to !== row) onGo(to, tab.cell.col);
  };
  function onKey(e: KeyboardEvent): void {
    if (e.key === "PageDown" && !e.ctrlKey && !e.altKey) {
      e.preventDefault();
      flip(1);
    } else if (e.key === "PageUp" && !e.ctrlKey && !e.altKey) {
      e.preventDefault();
      flip(-1);
    }
  }
</script>

<div class="record" data-testid="record-panel" role="presentation" aria-label="Record" onkeydown={onKey}>
  <div class="top">
    <button type="button" class="small" aria-label="Previous row" title="Previous row (Page Up here, Ctrl+Alt+Up anywhere)" disabled={row <= tab.firstRow} onclick={() => flip(-1)}>◀</button>
    <span class="pos" data-testid="record-pos" aria-live="polite">{pos}</span>
    <button type="button" class="small" aria-label="Next row" title="Next row (Page Down here, Ctrl+Alt+Down anywhere)" disabled={row >= tab.info.rows - 1} onclick={() => flip(1)}>▶</button>
  </div>
  <input type="search" class="search" placeholder="Search fields" aria-label="Search fields by title or value" bind:value={query} />
  <label class="opt"><input type="checkbox" checked={showHidden} onchange={(e) => { prefs.data.recordHidden = e.currentTarget.checked; prefs.save(); }} /> Show hidden columns (dimmed)</label>
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#if values.length === 0}
    <p class="note">{tab.info.rows === 0 ? "This file has no rows." : "Select a cell to see its row here."}</p>
  {:else if list.order.length === 0}
    <p class="note">No field matches.</p>
  {:else}
    <RecordFields
      windowed
      order={list.order}
      {titles}
      bind:values
      {original}
      {reason}
      dimmed={(c) => tab.columns.isHidden(c)}
      pinned={(c) => list.pinned.has(c)}
      onPin={(c) => (tab.recordPins = togglePin(tab.recordPins, titles, c))}
      onJump={(c) => onGo(row, c)}
      onCommit={(c) => void commit(c)}
      onExpand={(c) => void expand(c)}
      onEditing={(c) => (editing = c)}
    />
  {/if}
  {#if tab.info.cols > n}<p class="note">The first {n.toLocaleString()} of {tab.info.cols.toLocaleString()} columns are shown.</p>{/if}
  {#if big}
    <div class="big" role="dialog" aria-label="Edit a long value" tabindex="-1" onkeydown={(e) => { e.stopPropagation(); if (e.key === "Escape") big = null; }}>
      <textarea rows="8" bind:value={big.text} readonly={reason(big.col) !== ""} aria-label={titles[big.col] || `Column ${big.col + 1}`}></textarea>
      <div class="row">
        <button type="button" class="small primary" disabled={reason(big.col) !== ""} onclick={() => void saveBig()}>Save</button>
        <button type="button" class="small" onclick={() => (big = null)}>Cancel</button>
      </div>
    </div>
  {/if}
</div>

<style>
  .record {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-height: 0;
    height: 100%;
    position: relative;
  }
  .top {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .pos {
    flex: 1;
    text-align: center;
    color: var(--muted);
  }
  .search {
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 3px 8px;
  }
  .opt {
    font-size: 12px;
    color: var(--muted);
  }
  .note {
    color: var(--muted);
    font-size: 12px;
  }
  .problem {
    color: #c0392b;
    margin: 0;
  }
  .big {
    position: absolute;
    inset: 40px 0 auto 0;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.3);
    z-index: 5;
  }
  .big textarea {
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 5px;
    resize: vertical;
  }
  .row {
    display: flex;
    gap: 6px;
  }
</style>
