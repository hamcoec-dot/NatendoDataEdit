<script lang="ts">
  // Changes from a newer file (Milestone 15): pair rows with a newer copy
  // by key columns, list every changed cell, new row and missing row, tick
  // the ones to take, and apply them as one undo step.
  import AllNone from "./AllNone.svelte";
  import { untrack } from "svelte";
  import { backend, errorText, type DocInfo } from "./backend";
  import FormShell from "./FormShell.svelte";
  import type { OtherTable } from "./transfer";
  import { choiceOf, columnNotes, type UpdateList } from "./updates";

  interface Props {
    doc: number;
    titles: string[];
    header: boolean;
    col: number;
    others: OtherTable[];
    onOpen: () => Promise<OtherTable | null>;
    onDone: (info: DocInfo, summary: string) => void;
    onClose: () => void;
  }
  let { doc, titles, header, col, others, onOpen, onDone, onClose }: Props = $props();

  let list = $state<OtherTable[]>(untrack(() => others.filter((o) => o.id !== doc)));
  let other = $state<number>(untrack(() => list[list.length - 1]?.id ?? -1));
  let keys = $state<number[]>([]);
  let keepFirst = $state(untrack(() => header));
  let trim = $state(true);
  let matchCase = $state(false);
  let found = $state<UpdateList | null>(null);
  let running = $state(false);
  let problem = $state("");
  // Ticks: true = take it. Changed cells and new rows start ticked,
  // missing rows not (taking them deletes rows).
  let cellsAll = $state(true);
  let addedAll = $state(true);
  let removedAll = $state(false);
  let cellsFlip = $state(new Set<number>());
  let addedFlip = $state(new Set<number>());
  let removedFlip = $state(new Set<number>());
  let tab = $state<"cells" | "added" | "removed">("cells");
  let page = $state(0);
  const PAGE = 200;

  async function openFile(): Promise<void> {
    const o = await onOpen();
    if (o) {
      list = [...list.filter((x) => x.id !== o.id), o];
      other = o.id;
    }
  }
  async function compare(): Promise<void> {
    if (other < 0 || !keys.length || running) return;
    running = true;
    problem = "";
    try {
      found = await backend().updatesFind(doc, other, { keys: $state.snapshot(keys) as number[], keep_first: keepFirst, match_case: matchCase, trim });
      cellsFlip = new Set();
      addedFlip = new Set();
      removedFlip = new Set();
      page = 0;
      tab = found.cells_total ? "cells" : found.added_total ? "added" : "removed";
    } catch (e) {
      problem = errorText(e);
    } finally {
      running = false;
    }
  }
  const flip = (s: Set<number>, i: number): Set<number> => {
    const n = new Set(s);
    if (n.has(i)) n.delete(i);
    else n.add(i);
    return n;
  };
  const taken = (all: boolean, flips: Set<number>, total: number) => (all ? total - flips.size : flips.size);
  const nCells = $derived(found ? taken(cellsAll, cellsFlip, found.cells_total) : 0);
  const nAdded = $derived(found ? taken(addedAll, addedFlip, found.added_total) : 0);
  const nRemoved = $derived(found ? taken(removedAll, removedFlip, found.removed_total) : 0);
  async function apply(): Promise<void> {
    if (!found || running) return;
    running = true;
    problem = "";
    try {
      const info = await backend().updatesAccept(doc, {
        cells: choiceOf(cellsAll, cellsFlip),
        added: choiceOf(addedAll, addedFlip),
        removed: choiceOf(removedAll, removedFlip),
      });
      onDone(info, `Took ${plural(nCells, "changed cell")}, ${plural(nAdded, "new row")} and ${plural(nRemoved, "row removal")} from the newer file.`);
    } catch (e) {
      problem = errorText(e);
    } finally {
      running = false;
    }
  }
  const plural = (n: number, one: string, many = `${one}s`) => `${n.toLocaleString()} ${n === 1 ? one : many}`;
  const title = (c: number) => titles[c] ?? `Column ${c + 1}`;
  const short = (v: string) => (v.length > 60 ? `${v.slice(0, 60)}…` : v) || "(blank)";
</script>

<FormShell title="Changes from a newer file" wide {onClose}>
  {#if !found}
    <p class="note">Compares this table with a newer copy. Rows are paired by the columns that identify them (an ID, or a name and birth date). You then choose which changed cells, new rows and missing rows to take; nothing changes until you apply, and Undo takes it all back in one step.</p>
    <label class="row">Newer file
      <select bind:value={other}>{#each list as o (o.id)}<option value={o.id}>{o.name}</option>{/each}</select>
      <button type="button" onclick={() => void openFile()}>Open a file…</button>
    </label>
    <fieldset>
      <legend>Columns that identify a row</legend>
      <AllNone count={titles.length} picked={keys} onChange={(v) => (keys = v)} max={16} />
      <div class="keys">
        {#each titles as t, c (c)}
          <label><input type="checkbox" checked={keys.includes(c)} onchange={(e) => { keys = (e.currentTarget as HTMLInputElement).checked ? [...keys, c].slice(0, 16) : keys.filter((k) => k !== c); }} /> {t}</label>
        {/each}
      </div>
    </fieldset>
    <label class="row"><input type="checkbox" bind:checked={keepFirst} /> Both files start with a title row (columns are then matched by title, wherever they are)</label>
    <label class="row"><input type="checkbox" bind:checked={trim} /> Ignore spaces around the key values</label>
    <label class="row"><input type="checkbox" bind:checked={matchCase} /> Key values must match in case</label>
  {:else}
    <p>{plural(found.cells_total, "changed cell")}, {plural(found.added_total, "new row")} (in the newer file only), {plural(found.removed_total, "missing row")} (here only).
      {#if found.repeated_keys}<span class="warn">{plural(found.repeated_keys, "key")} appear more than once; those rows were paired in order.</span>{/if}
      {#each columnNotes(found, keepFirst) as note (note)}<span class="warn">{note}</span>{/each}
    </p>
    <div class="tabs" role="tablist">
      <button type="button" role="tab" aria-selected={tab === "cells"} onclick={() => { tab = "cells"; page = 0; }}>Changed cells ({nCells.toLocaleString()} of {found.cells_total.toLocaleString()} taken)</button>
      <button type="button" role="tab" aria-selected={tab === "added"} onclick={() => { tab = "added"; page = 0; }}>New rows ({nAdded.toLocaleString()} of {found.added_total.toLocaleString()})</button>
      <button type="button" role="tab" aria-selected={tab === "removed"} onclick={() => { tab = "removed"; page = 0; }}>Missing rows ({nRemoved.toLocaleString()} of {found.removed_total.toLocaleString()})</button>
    </div>
    {#if tab === "cells"}
      <div class="tools"><button type="button" onclick={() => { cellsAll = true; cellsFlip = new Set(); }}>Take all</button><button type="button" onclick={() => { cellsAll = false; cellsFlip = new Set(); }}>Take none</button></div>
      <div class="list">
        {#each found.cells.slice(page * PAGE, page * PAGE + PAGE) as c, k (page * PAGE + k)}
          {@const i = page * PAGE + k}
          <label class="line"><input type="checkbox" checked={cellsAll !== cellsFlip.has(i)} onchange={() => (cellsFlip = flip(cellsFlip, i))} /><span>Row {(c.row + 1).toLocaleString()}</span><span>{title(c.col)}</span><span class="old">{short(c.old)}</span><span>→ {short(c.new)}</span></label>
        {/each}
      </div>
      {#if found.cells_total > found.cells.length}<p class="note">The first {found.cells.length.toLocaleString()} are listed; "Take all" and "Take none" cover the rest too.</p>{/if}
      {#if found.cells.length > PAGE}
        <div class="tools"><button type="button" disabled={page === 0} onclick={() => page--}>Previous</button><button type="button" disabled={(page + 1) * PAGE >= found.cells.length} onclick={() => page++}>Next</button></div>
      {/if}
    {:else if tab === "added"}
      <div class="tools"><button type="button" onclick={() => { addedAll = true; addedFlip = new Set(); }}>Take all</button><button type="button" onclick={() => { addedAll = false; addedFlip = new Set(); }}>Take none</button></div>
      <div class="list">
        {#each found.added as r, i (i)}
          <label class="line"><input type="checkbox" checked={addedAll !== addedFlip.has(i)} onchange={() => (addedFlip = flip(addedFlip, i))} /><span class="wide">{r.slice(0, 8).map(short).join(" · ")}</span></label>
        {/each}
      </div>
      <p class="note">New rows go at the end of the table.{found.added_total > found.added.length ? ` The first ${found.added.length.toLocaleString()} are listed.` : ""}</p>
    {:else}
      <div class="tools"><button type="button" onclick={() => { removedAll = true; removedFlip = new Set(); }}>Take all (delete them)</button><button type="button" onclick={() => { removedAll = false; removedFlip = new Set(); }}>Keep all</button></div>
      <div class="list">
        {#each found.removed.slice(page * PAGE, page * PAGE + PAGE) as r, k (page * PAGE + k)}
          {@const i = page * PAGE + k}
          <label class="line"><input type="checkbox" checked={removedAll !== removedFlip.has(i)} onchange={() => (removedFlip = flip(removedFlip, i))} /><span>Row {(r + 1).toLocaleString()} is not in the newer file; tick to delete it</span></label>
        {/each}
      </div>
    {/if}
  {/if}
  {#if problem}<p class="warn">{problem}</p>{/if}
  {#snippet buttons()}
    {#if !found}
      <button type="button" class="primary" disabled={other < 0 || !keys.length || running} onclick={() => void compare()}>{running ? "Comparing…" : "Compare"}</button>
    {:else}
      <button type="button" class="primary" disabled={running || nCells + nAdded + nRemoved === 0} onclick={() => void apply()}>Apply the ticked changes</button>
      <button type="button" onclick={() => (found = null)}>Change the setup</button>
    {/if}
    <button type="button" onclick={onClose}>Close</button>
  {/snippet}
</FormShell>

<style>
  .keys {
    max-height: 140px;
    overflow: auto;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(12em, 1fr));
  }
  .tabs,
  .tools {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    margin: 6px 0;
  }
  .tabs [aria-selected="true"] {
    font-weight: 600;
  }
  .list {
    max-height: 320px;
    overflow: auto;
    border: 1px solid var(--border, #8884);
    border-radius: 4px;
  }
  .line {
    display: grid;
    grid-template-columns: 2em 7em 10em 1fr 1fr;
    gap: 6px;
    padding: 2px 8px;
  }
  .line span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .line .wide {
    grid-column: span 4;
  }
  .old {
    opacity: 0.7;
    text-decoration: line-through;
  }
  .warn {
    color: var(--warn, #b45309);
  }
</style>
