<script lang="ts">
  // Custom sort: one or more columns, each with its order and how values
  // compare. The first row stays in place when it holds the titles.
  import HiddenNote from "./HiddenNote.svelte";
  import { hiddenRows, sortWords, includeWhile } from "./shown.svelte";
  import { untrack } from "svelte";
  import type { SortKey, SortKind, SortSpec } from "./backend";
  import { prefs } from "./prefs.svelte";
  import { BUILTIN_LISTS, FILL_ONE_CELL_MAX, fillByDefault, MAX_ORDERS, newOrderId, parseList, type FromCells, type SavedOrder } from "./m10";

  interface Props {
    /** Column titles (or numbers), one per column. */
    titles: string[];
    header: boolean;
    start: SortKey[];
    /** Open with "New custom order" filled in from these cells. */
    fromCells?: FromCells | null;
    onSort: (spec: SortSpec, header: boolean) => void;
    onClose: () => void;
  }
  let { titles, header, start, fromCells = null, onSort, onClose }: Props = $props();

  /** A sort level; `order` is the id of the custom order it uses (kept
   *  through rename and move), `gone` says its order was deleted. */
  type Level = SortKey & { order?: string; gone?: boolean };

  const MAX_LEVELS = 64;
  // Starting values; the dialog edits its own copies.
  let keys = $state<Level[]>(untrack(() => start.map((k) => ({ ...k }))));
  let hasHeader = $state(untrack(() => header));
  /** Sort hidden rows too (offered only while some are hidden). */
  let sortAll = $state(false);
  let box: HTMLDivElement;

  const KINDS: [SortKind, string][] = [
    ["auto", "Automatic"],
    ["natural", "Text, numbers by value (A2 < A10)"],
    ["text", "Text"],
    ["number", "Numbers"],
    ["date", "Dates"],
    ["length", "Length of the text"],
    ["last_word", "Last word (surnames)"],
  ];

  /** Custom orders: the built-in ones, then the user's own. */
  const builtins: SavedOrder[] = BUILTIN_LISTS.map((l, i) => ({ id: `builtin-${i}`, name: l.name, items: l.items }));
  const lists = $derived([...builtins, ...prefs.data.sortLists]);
  const kindId = (k: Level): string => (k.order ? `list:${k.order}` : typeof k.kind === "object" ? "auto" : k.kind);
  function setKind(k: Level, id: string, el: HTMLSelectElement): void {
    if (id === "new") {
      // The list picker shows the key's current kind again until the new order is added.
      el.value = kindId(k);
      addingFor = k;
      startAdding(false);
      return;
    }
    k.gone = false;
    if (id.startsWith("list:")) {
      k.order = id.slice(5);
      k.kind = "auto";
    } else {
      k.order = undefined;
      k.kind = id as SortKind;
    }
  }

  // ---- a new order ----
  let adding = $state(false);
  let newName = $state("");
  let newItems = $state("");
  /** The new order's values came from cells: one per line (a value may hold a comma). */
  let newFromCells = $state(false);
  let addProblem = $state("");
  /** The sort level that asked for the new order; it uses the order once added. */
  let addingFor: Level | null = null;
  const newParsed = $derived(parseList(newItems, !newFromCells));
  function startAdding(cells: boolean): void {
    adding = true;
    newFromCells = cells;
    addProblem = "";
  }
  function addList(): void {
    const items = newParsed;
    if (!items.length) return;
    if (prefs.data.sortLists.length >= MAX_ORDERS) {
      addProblem = `${MAX_ORDERS} custom orders is the most: delete one first (Manage custom orders).`;
      return;
    }
    const name = newName.trim().slice(0, 200) || items.slice(0, 3).join(", ");
    const id = newOrderId([...builtins, ...prefs.data.sortLists]);
    prefs.data.sortLists = [...prefs.data.sortLists, { id, name, items, ...(newFromCells ? { fromCells: true } : {}), fill: fillByDefault(items, newFromCells) }];
    prefs.save();
    if (addingFor) {
      addingFor.order = id;
      addingFor.kind = "auto";
      addingFor.gone = false;
    }
    addingFor = null;
    adding = false;
    newName = "";
    newItems = "";
    newFromCells = false;
  }
  // Opened from selected cells: the new order's box, filled in.
  untrack(() => {
    if (fromCells) {
      newItems = fromCells.items.join("\n");
      startAdding(true);
    }
  });

  // ---- managing the user's orders ----
  let managing = $state(false);
  /** The order whose values are being edited, and the text. */
  let editing = $state<string | null>(null);
  let editText = $state("");
  /** The order asked about before it is deleted. */
  let deleting = $state<string | null>(null);
  function saveOrders(next: SavedOrder[]): void {
    prefs.data.sortLists = next;
    prefs.save();
  }
  function rename(id: string, name: string): void {
    const n = name.trim().slice(0, 200);
    if (!n) return;
    saveOrders(prefs.data.sortLists.map((o) => (o.id === id ? { ...o, name: n } : o)));
  }
  function setFill(id: string, on: boolean): void {
    saveOrders(prefs.data.sortLists.map((o) => (o.id === id ? { ...o, fill: on } : o)));
  }
  function moveOrder(i: number, d: number): void {
    const j = i + d;
    const next = [...prefs.data.sortLists];
    if (j < 0 || j >= next.length) return;
    const [o] = next.splice(i, 1);
    next.splice(j, 0, o!);
    saveOrders(next);
  }
  function startEdit(o: SavedOrder): void {
    editing = o.id;
    editText = o.items.join("\n");
  }
  function saveEdit(o: SavedOrder): void {
    // Values from cells may hold commas: one per line for them.
    const items = parseList(editText, !o.fromCells);
    if (!items.length) return;
    saveOrders(prefs.data.sortLists.map((x) => (x.id === o.id ? { ...x, items } : x)));
    editing = null;
  }
  function remove(id: string): void {
    saveOrders(prefs.data.sortLists.filter((o) => o.id !== id));
    deleting = null;
    if (editing === id) editing = null;
    // A level that used it sorts automatically, and says so.
    for (const k of keys)
      if (k.order === id) {
        k.order = undefined;
        k.kind = "auto";
        k.gone = true;
      }
  }

  function add(): void {
    const used = new Set(keys.map((k) => k.col));
    const col = titles.findIndex((_, i) => !used.has(i));
    keys.push({ col: Math.max(0, col), descending: false, kind: "auto", match_case: false });
  }

  function moveKey(i: number, d: number): void {
    const j = i + d;
    if (j < 0 || j >= keys.length) return;
    const [k] = keys.splice(i, 1);
    keys.splice(j, 0, k!);
  }

  function ok(): void {
    if (keys.length === 0) return;
    // Only the custom orders used travel with the sort, renumbered.
    const used: string[][] = [];
    const at = new Map<string, number>();
    const ks: SortKey[] = $state.snapshot(keys).map(({ order, gone: _gone, ...k }) => {
      const items = order ? lists.find((l) => l.id === order)?.items : undefined;
      if (!order || !items) return { ...k, kind: typeof k.kind === "object" ? ("auto" as const) : k.kind };
      if (!at.has(order)) {
        at.set(order, used.length);
        used.push(items);
      }
      return { ...k, kind: { list: at.get(order)! } };
    });
    const scope = hiddenRows.hidden && !sortAll ? "shown" : "all";
    onSort({ keys: ks, keep_first: hasHeader, scope, ...(used.length ? { lists: used } : {}) }, hasHeader);
  }

  function onKey(e: KeyboardEvent): void {
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "Enter" && !(e.target instanceof HTMLButtonElement) && !(e.target instanceof HTMLTextAreaElement) && !(e.target instanceof HTMLInputElement && e.target.type === "text")) {
      e.preventDefault();
      ok();
    }
  }

  $effect(() => {
    (fromCells ? box.querySelector<HTMLInputElement>(".newlist input") : box.querySelector<HTMLSelectElement>("select"))?.focus();
  });

  // The status pill turns amber while this includes hidden rows.
  $effect(() => includeWhile(sortAll));
</script>

<div class="backdrop" role="presentation">
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="sort-title" tabindex="-1" bind:this={box} onkeydown={onKey}>
    <h2 id="sort-title">Sort</h2>
    <label class="hdr"><input type="checkbox" bind:checked={hasHeader} /> First row is header (keep it at the top)</label>
    {#if hiddenRows.hidden}
      <HiddenNote text={sortAll ? "Including hidden rows: every row is put in order." : sortWords(hiddenRows.hidden, hiddenRows.total)} amber={sortAll} />
      <label class="hdr"><input type="checkbox" bind:checked={sortAll} /> Sort all rows, including hidden ones</label>
    {/if}
    <div class="levels">
      {#each keys as k, i (i)}
        <div class="level">
          <span class="by">{i === 0 ? "Sort by" : "Then by"}</span>
          <select aria-label="Column" bind:value={k.col}>
            {#each titles as t, c (c)}<option value={c}>{t}</option>{/each}
          </select>
          <select aria-label="Order" value={k.descending ? "d" : "a"} onchange={(e) => (k.descending = (e.currentTarget as HTMLSelectElement).value === "d")}>
            <option value="a">A → Z, smallest first</option>
            <option value="d">Z → A, largest first</option>
          </select>
          <select aria-label="Compare as" value={kindId(k)} onchange={(e) => setKind(k, (e.currentTarget as HTMLSelectElement).value, e.currentTarget as HTMLSelectElement)}>
            {#each KINDS as [v, label] (v)}<option value={v}>{label}</option>{/each}
            <optgroup label="Custom order">
              {#each lists as l (l.id)}<option value={`list:${l.id}`}>{l.name}</option>{/each}
              <option value="new">New custom order…</option>
            </optgroup>
          </select>
          <label title="Lowercase before uppercase of the same letter"><input type="checkbox" bind:checked={k.match_case} /> Case</label>
          <button type="button" aria-label="Move up" disabled={i === 0} onclick={() => moveKey(i, -1)}>↑</button>
          <button type="button" aria-label="Move down" disabled={i === keys.length - 1} onclick={() => moveKey(i, 1)}>↓</button>
          <button type="button" aria-label="Remove" disabled={keys.length === 1} onclick={() => keys.splice(i, 1)}>✕</button>
          {#if k.gone}<span class="gone">Its custom order was deleted: this level sorts automatically.</span>{/if}
        </div>
      {/each}
    </div>
    <button type="button" class="link" disabled={keys.length >= MAX_LEVELS} onclick={add}>+ Add a level</button>
    <button type="button" class="link" onclick={() => (managing = !managing)}>{managing ? "Close custom orders" : "Manage custom orders…"}</button>
    {#if adding}
      <fieldset class="newlist">
        <legend>New custom order{newFromCells ? " from the selected cells" : ""}</legend>
        <label>Name <input type="text" bind:value={newName} placeholder="Priority" /></label>
        <label>{newFromCells ? "Values in order (one per line)" : "Values in order (one per line, or separated by commas)"}<textarea rows="4" bind:value={newItems} placeholder={"Urgent\nHigh\nNormal\nLow"}></textarea></label>
        {#if newFromCells && fromCells}
          <p class="note">{newParsed.length.toLocaleString()} different values from the selected cells, row by row{fromCells.leftOut ? `; ${fromCells.leftOut.toLocaleString()} hidden ${fromCells.leftOut === 1 ? "cell was" : "cells were"} left out` : ""}. Check them before adding.</p>
          {#if fromCells.private}<p class="warn" role="alert">These values will be kept in this app's settings, outside the file. Nothing is saved until you add the order.</p>{/if}
        {/if}
        {#if addProblem}<p class="warn" role="alert">{addProblem}</p>{/if}
        <div class="buttons">
          <button type="button" onclick={addList} disabled={!newParsed.length}>Add the order</button>
          <button type="button" onclick={() => { adding = false; addingFor = null; newFromCells = false; }}>Cancel</button>
        </div>
        <p class="note">Values on the list sort in that order (capitals and accents ignored); other values follow.</p>
      </fieldset>
    {/if}
    {#if managing}
      <fieldset class="newlist manage">
        <legend>Your custom orders</legend>
        {#if !prefs.data.sortLists.length}<p class="note">None yet: "New custom order…" in Compare as adds one.</p>{/if}
        {#each prefs.data.sortLists as o, i (o.id)}
          <div class="order">
            <input type="text" aria-label="Name of the order" value={o.name} onchange={(e) => rename(o.id, (e.currentTarget as HTMLInputElement).value)} />
            <span class="count">{o.items.length.toLocaleString()} {o.items.length === 1 ? "value" : "values"}{o.fromCells ? ", from cells" : ""}</span>
            <button type="button" onclick={() => (editing === o.id ? (editing = null) : startEdit(o))}>{editing === o.id ? "Close" : "Edit values"}</button>
            <button type="button" aria-label="Move up" disabled={i === 0} onclick={() => moveOrder(i, -1)}>↑</button>
            <button type="button" aria-label="Move down" disabled={i === prefs.data.sortLists.length - 1} onclick={() => moveOrder(i, 1)}>↓</button>
            {#if deleting === o.id}
              <span class="ask">Delete "{o.name}"?</span>
              <button type="button" onclick={() => remove(o.id)}>Delete</button>
              <button type="button" onclick={() => (deleting = null)}>Keep it</button>
            {:else}
              <button type="button" onclick={() => (deleting = o.id)}>Delete…</button>
            {/if}
          </div>
          <label class="fill" title={o.items.length > FILL_ONE_CELL_MAX ? `Only orders of up to ${FILL_ONE_CELL_MAX} values are followed from one cell` : "Dragging the fill handle from one cell holding a value on this order fills the next values"}>
            <input type="checkbox" checked={o.fill === true && o.items.length <= FILL_ONE_CELL_MAX} disabled={o.items.length > FILL_ONE_CELL_MAX} onchange={(e) => setFill(o.id, (e.currentTarget as HTMLInputElement).checked)} />
            Fill handle follows this order from one cell{o.items.length > FILL_ONE_CELL_MAX ? ` (not for more than ${FILL_ONE_CELL_MAX} values)` : ""}
          </label>
          {#if editing === o.id}
            <div class="edit">
              <textarea rows="4" aria-label="Values in order" bind:value={editText}></textarea>
              <button type="button" disabled={!parseList(editText, !o.fromCells).length} onclick={() => saveEdit(o)}>Save the values</button>
            </div>
          {/if}
        {/each}
        <p class="note">When a value is on more than one order, the fill handle uses the higher one. Two or more starting cells that step evenly along an order follow it whether ticked or not; Ctrl while dragging copies instead.</p>
        <p class="note">The built-in orders (Low, Medium, High; weekdays; months) cannot be changed. A recipe or saved view that sorts by a custom order keeps its values, so they go wherever you send it.</p>
      </fieldset>
    {/if}
    <p class="note">Empty cells always go last. Rows with equal values keep their order. Undo (Ctrl+Z) puts the rows back.</p>
    <div class="buttons">
      <button type="button" class="primary" onclick={ok}>Sort</button>
      <button type="button" onclick={onClose}>Cancel</button>
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
    width: min(760px, 100%);
    max-height: 90vh;
    overflow: auto;
    box-shadow: 0 12px 40px rgb(0 0 0 / 0.3);
    outline: none;
    font-size: 13px;
  }
  h2 {
    font-size: 16px;
    margin: 0 0 10px;
  }
  .levels {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 10px 0 6px;
  }
  .level {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
  }
  .by {
    width: 56px;
    color: var(--muted);
  }
  select,
  button {
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 3px 8px;
  }
  select {
    max-width: 220px;
  }
  .newlist {
    display: flex;
    flex-direction: column;
    gap: 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
  }
  .newlist label {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .newlist input,
  .newlist textarea {
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 4px 8px;
    box-sizing: border-box;
    width: 100%;
  }
  .newlist textarea {
    resize: vertical;
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
    padding: 6px 16px;
  }
  button.link {
    border: none;
    background: none;
    color: var(--accent);
    padding: 2px 0;
  }
  .note {
    color: var(--muted);
    font-size: 12px;
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  button.link + button.link {
    margin-left: 14px;
  }
  .gone,
  .warn {
    color: var(--warn, #b45309);
    font-size: 12px;
  }
  .manage .order {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
    margin: 4px 0;
  }
  .manage .count,
  .manage .ask {
    color: var(--muted);
  }
  .manage .fill {
    display: block;
    margin: 0 0 8px;
    color: var(--muted);
  }
  .manage .edit {
    display: flex;
    gap: 6px;
    align-items: flex-start;
    margin: 0 0 6px;
  }
</style>
