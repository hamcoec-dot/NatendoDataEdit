<script lang="ts">
  // Data › Compare lists: 2 to 5 open tabs, a key in each, and how many keys
  // every combination of the lists shares. Any slice opens as a tab.
  import { untrack } from "svelte";
  import AllNone from "./AllNone.svelte";
  import FormShell from "./FormShell.svelte";
  import { countsText, listsIn, MAX_LISTS, MAX_SET_KEYS, sliceName, type SetsRun, type SetsSpec } from "./sets";

  export interface SetsTab {
    id: number;
    name: string;
    titles: string[];
    hidden: number[];
    header: boolean;
  }

  interface Props {
    tabs: SetsTab[];
    /** The tab the screen was opened on. */
    current: number;
    onRun: (tabs: number[], spec: SetsSpec) => Promise<SetsRun | null>;
    onCancelRun: () => void;
    /** List `list`'s rows, or the keys as it spells them. `header`: that
     *  list has a title row. */
    onOpen: (id: number, mask: number, list: number, keys: boolean, header: boolean) => Promise<boolean>;
    onTable: (id: number, save: boolean) => Promise<boolean>;
    onCopy: (text: string) => void;
    /** `id` of the comparison to forget, if one was made. */
    onClose: (id: number | null) => void;
  }
  let { tabs, current, onRun, onCancelRun, onOpen, onTable, onCopy, onClose }: Props = $props();

  type Pick = { tab: number; cols: number[] };
  const init = untrack(() => {
    const other = tabs.find((t) => t.id !== current)?.id ?? current;
    return [
      { tab: current, cols: [] as number[] },
      { tab: other, cols: [] as number[] },
    ];
  });
  let lists = $state<Pick[]>(init);
  let matchCase = $state(false);
  let trim = $state(true);
  let loose = $state(false);
  let empty = $state(false);
  let busy = $state(false);
  let result = $state<SetsRun | null>(null);
  /** What the result was made with. */
  let ran = $state("");
  /** For each slice, whose rows to open ("k": just the keys). */
  let pick = $state<Record<number, string>>({});

  const tabOf = (id: number) => tabs.find((t) => t.id === id)!;
  const letter = (i: number) => String.fromCharCode(65 + i);
  const spec = $derived<SetsSpec>({
    lists: lists.map((l) => ({ cols: [...l.cols].sort((a, b) => a - b), header: tabOf(l.tab).header })),
    match_case: matchCase,
    trim,
    loose,
  });
  const key = $derived(JSON.stringify([lists.map((l) => l.tab), spec]));
  const problem = $derived.by(() => {
    const n = lists[0]!.cols.length;
    if (!n || lists.some((l) => !l.cols.length)) return "Tick the key column (or columns) of every list.";
    if (lists.some((l) => l.cols.length !== n)) return "Tick the same number of key columns in every list.";
    if (lists.some((l, i) => lists.slice(0, i).some((m) => m.tab === l.tab && JSON.stringify(spec.lists[i]) === JSON.stringify(spec.lists[lists.indexOf(m)])))) return "A tab that is in twice needs different key columns.";
    return "";
  });
  const stale = $derived(!result || ran !== key);

  function setTab(i: number, id: number): void {
    lists[i] = { tab: id, cols: [] };
  }
  function tick(i: number, c: number, on: boolean): void {
    const cols = lists[i]!.cols;
    lists[i]!.cols = on ? [...cols, c].slice(-MAX_SET_KEYS) : cols.filter((x) => x !== c);
  }
  async function compare(): Promise<void> {
    if (busy || problem) return;
    const at = key;
    const s = $state.snapshot(spec) as SetsSpec;
    busy = true;
    try {
      const r = await onRun(
        lists.map((l) => l.tab),
        s,
      );
      if (!r) return;
      result = r;
      ran = at;
      pick = {};
    } finally {
      busy = false;
    }
  }
  /** A refusal (a list changed) leaves the screen up, its result gone. */
  async function open(mask: number): Promise<void> {
    if (!result) return;
    const first = listsIn(mask, result.names.length)[0]!;
    const choice = pick[mask] ?? String(first);
    const keys = choice === "k";
    const list = keys ? first : Number(choice);
    if (!(await onOpen(result.id, mask, list, keys, spec.lists[list]!.header))) result = null;
  }
  async function table(save: boolean): Promise<void> {
    if (result && !(await onTable(result.id, save))) result = null;
  }
  function close(): void {
    if (busy) onCancelRun();
    onClose(result?.id ?? null);
  }
</script>

<FormShell title="Compare lists" wide onClose={close} onEnter={() => void compare()}>
  <div class="lists">
    {#each lists as l, i (i)}
      {@const t = tabOf(l.tab)}
      <fieldset>
        <legend>List {letter(i)}</legend>
        <select aria-label={`Tab of list ${letter(i)}`} value={l.tab} disabled={busy} onchange={(e) => setTab(i, Number((e.currentTarget as HTMLSelectElement).value))}>
          {#each tabs as o (o.id)}<option value={o.id}>{o.name}</option>{/each}
        </select>
        <p class="muted">Key columns</p>
        <AllNone count={t.titles.length} picked={l.cols} max={MAX_SET_KEYS} skip={(c) => t.hidden.includes(c)} onChange={(v) => (lists[i]!.cols = v)} />
        <div class="cols">
          {#each t.titles as title, c (c)}
            {#if !t.hidden.includes(c)}
              <label><input type="checkbox" checked={l.cols.includes(c)} disabled={busy} onchange={(e) => tick(i, c, (e.currentTarget as HTMLInputElement).checked)} /> {title}</label>
            {/if}
          {/each}
        </div>
        {#if lists.length > 2}<button type="button" disabled={busy} onclick={() => (lists = lists.filter((_, j) => j !== i))}>Remove list {letter(i)}</button>{/if}
      </fieldset>
    {/each}
    {#if lists.length < MAX_LISTS}<button type="button" class="add" disabled={busy} onclick={() => lists.push({ tab: current, cols: [] })}>Add a list</button>{/if}
  </div>
  <div class="options">
    <label><input type="checkbox" checked={!matchCase} disabled={busy} onchange={(e) => (matchCase = !(e.currentTarget as HTMLInputElement).checked)} /> Ignore capitals</label>
    <label><input type="checkbox" bind:checked={trim} disabled={busy} /> Trim spaces</label>
    <label><input type="checkbox" bind:checked={loose} disabled={busy} /> Loose keys (as in Join: punctuation and leading zeros ignored; Id and Text columns stay exact)</label>
  </div>
  <p class="muted">One set of rules for every list. Shown rows only. With several key columns, a row counts as blank only when all of them are empty. A slice opens with all columns of the list you choose.</p>
  {#if problem}<p class="muted" role="note">{problem}</p>{/if}

  {#if busy}
    <p role="status">Comparing… <button type="button" onclick={onCancelRun}>Stop</button></p>
  {:else if result && stale}
    <p class="warn" role="status">Something changed: press Compare again.</p>
  {:else if result}
    {@const r = result}
    <label><input type="checkbox" bind:checked={empty} /> Show empty slices</label>
    <div class="scroll">
      <table aria-label="Slices">
        <thead>
          <tr><th>Slice</th><th>Different keys</th>{#each r.names as n, i (i)}<th>Rows in {n}</th>{/each}<th>Open as a tab</th></tr>
        </thead>
        <tbody>
          {#each r.report.slices.filter((s) => empty || s.keys) as s (s.mask)}
            {@const within = listsIn(s.mask, r.names.length)}
            <tr>
              <th scope="row">{sliceName(s.mask, r.names)}</th>
              <td>{s.keys.toLocaleString()}</td>
              {#each s.rows as n, i (i)}<td>{n.toLocaleString()}</td>{/each}
              <td class="open">
                <select aria-label={`What to open for ${sliceName(s.mask, r.names)}`} value={pick[s.mask] ?? String(within[0])} disabled={!s.keys} onchange={(e) => (pick[s.mask] = (e.currentTarget as HTMLSelectElement).value)}>
                  {#each within as i (i)}<option value={String(i)}>Rows of {r.names[i]}</option>{/each}
                  <option value="k">Just the keys, as {r.names[within[0]!]} spells them</option>
                </select>
                <button type="button" disabled={!s.keys} aria-label={`Open ${sliceName(s.mask, r.names)}`} onclick={() => void open(s.mask)}>Open</button>
              </td>
            </tr>
          {/each}
        </tbody>
        <tfoot>
          {#each r.report.lists as l, i (i)}
            <tr><td colspan={r.names.length + 3}>{r.names[i]}: {l.rows.toLocaleString()} rows, {l.keys.toLocaleString()} different keys, {l.blank.toLocaleString()} with a blank key</td></tr>
          {/each}
        </tfoot>
      </table>
    </div>
  {/if}
  {#snippet buttons()}
    <button type="button" onclick={close}>Close</button>
    <button type="button" disabled={busy || stale} onclick={() => result && onCopy(countsText(result, empty))}>Copy counts</button>
    <button type="button" disabled={busy || stale} onclick={() => void table(false)}>Counts as a tab</button>
    <button type="button" disabled={busy || stale} onclick={() => void table(true)}>Save counts as CSV…</button>
    <button type="button" class="primary" disabled={busy || !!problem} onclick={() => void compare()}>Compare</button>
  {/snippet}
</FormShell>

<style>
  .lists {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    align-items: flex-start;
  }
  fieldset {
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 4px 8px 8px;
    margin: 0;
    width: 168px;
  }
  legend {
    font-weight: 600;
  }
  fieldset select {
    width: 100%;
  }
  .cols {
    display: flex;
    flex-direction: column;
    max-height: 110px;
    overflow: auto;
    margin-bottom: 4px;
    white-space: nowrap;
  }
  .options {
    display: flex;
    gap: 14px;
    flex-wrap: wrap;
    margin-top: 8px;
  }
  .scroll {
    max-height: 220px;
    overflow: auto;
  }
  table {
    border-collapse: collapse;
    font-size: 12.5px;
    width: 100%;
  }
  th,
  td {
    border-bottom: 1px solid var(--border);
    padding: 3px 6px;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  tbody th,
  tfoot td {
    text-align: left;
  }
  .open {
    white-space: nowrap;
  }
  .open select {
    max-width: 170px;
  }
  .muted {
    color: var(--muted);
    font-size: 12px;
    margin: 6px 0;
  }
  .warn {
    color: var(--danger, #c0392b);
  }
</style>
