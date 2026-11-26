<script lang="ts">
  // Custom sort: one or more columns, each with its order and how values
  // compare. The first row stays in place when it holds the titles.
  import HiddenNote from "./HiddenNote.svelte";
  import { hiddenRows, sortWords, includeWhile } from "./shown.svelte";
  import { untrack } from "svelte";
  import type { SortKey, SortKind, SortSpec } from "./backend";
  import { prefs } from "./prefs.svelte";
  import { BUILTIN_LISTS, parseList } from "./m10";

  interface Props {
    /** Column titles (or numbers), one per column. */
    titles: string[];
    header: boolean;
    start: SortKey[];
    onSort: (spec: SortSpec, header: boolean) => void;
    onClose: () => void;
  }
  let { titles, header, start, onSort, onClose }: Props = $props();

  const MAX_LEVELS = 64;
  // Starting values; the dialog edits its own copies.
  let keys = $state<SortKey[]>(untrack(() => start.map((k) => ({ ...k }))));
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
  const lists = $derived([...BUILTIN_LISTS, ...prefs.data.sortLists]);
  const kindId = (k: SortKind): string => (typeof k === "object" ? `list:${k.list}` : k);
  function setKind(k: SortKey, id: string, el: HTMLSelectElement): void {
    if (id === "new") {
      // The list picker shows the key's current kind again until the new order is added.
      el.value = kindId(k.kind);
      addingFor = k;
      adding = true;
      return;
    }
    k.kind = id.startsWith("list:") ? { list: Number(id.slice(5)) } : (id as SortKind);
  }
  let adding = $state(false);
  let newName = $state("");
  let newItems = $state("");
  /** The sort level that asked for the new order; it uses the order once added. */
  let addingFor: SortKey | null = null;
  function addList(): void {
    const items = parseList(newItems);
    if (!items.length) return;
    const name = newName.trim().slice(0, 200) || items.slice(0, 3).join(", ");
    prefs.data.sortLists = [...prefs.data.sortLists, { name, items }].slice(-60);
    prefs.save();
    if (addingFor) addingFor.kind = { list: BUILTIN_LISTS.length + prefs.data.sortLists.length - 1 };
    addingFor = null;
    adding = false;
    newName = "";
    newItems = "";
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
    const at = new Map<number, number>();
    const ks = $state.snapshot(keys).map((k) => {
      if (typeof k.kind !== "object") return k;
      const i = k.kind.list;
      const items = lists[i]?.items;
      if (!items) return { ...k, kind: "natural" as const };
      if (!at.has(i)) {
        at.set(i, used.length);
        used.push(items);
      }
      return { ...k, kind: { list: at.get(i)! } };
    });
    const scope = hiddenRows.hidden && !sortAll ? "shown" : "all";
    onSort({ keys: ks, keep_first: hasHeader, scope, ...(used.length ? { lists: used } : {}) }, hasHeader);
  }

  function onKey(e: KeyboardEvent): void {
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "Enter" && !(e.target instanceof HTMLButtonElement)) {
      e.preventDefault();
      ok();
    }
  }

  $effect(() => {
    box.querySelector<HTMLSelectElement>("select")?.focus();
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
          <select aria-label="Compare as" value={kindId(k.kind)} onchange={(e) => setKind(k, (e.currentTarget as HTMLSelectElement).value, e.currentTarget as HTMLSelectElement)}>
            {#each KINDS as [v, label] (v)}<option value={v}>{label}</option>{/each}
            <optgroup label="Custom order">
              {#each lists as l, i (i)}<option value={`list:${i}`}>{l.name}</option>{/each}
              <option value="new">New custom order…</option>
            </optgroup>
          </select>
          <label title="Lowercase before uppercase of the same letter"><input type="checkbox" bind:checked={k.match_case} /> Case</label>
          <button type="button" aria-label="Move up" disabled={i === 0} onclick={() => moveKey(i, -1)}>↑</button>
          <button type="button" aria-label="Move down" disabled={i === keys.length - 1} onclick={() => moveKey(i, 1)}>↓</button>
          <button type="button" aria-label="Remove" disabled={keys.length === 1} onclick={() => keys.splice(i, 1)}>✕</button>
        </div>
      {/each}
    </div>
    <button type="button" class="link" disabled={keys.length >= MAX_LEVELS} onclick={add}>+ Add a level</button>
    {#if adding}
      <fieldset class="newlist">
        <legend>New custom order</legend>
        <label>Name <input type="text" bind:value={newName} placeholder="Priority" /></label>
        <label>Values in order (one per line, or separated by commas)<textarea rows="4" bind:value={newItems} placeholder={"Urgent\nHigh\nNormal\nLow"}></textarea></label>
        <div class="buttons">
          <button type="button" onclick={addList} disabled={!parseList(newItems).length}>Add the order</button>
          <button type="button" onclick={() => { adding = false; addingFor = null; }}>Cancel</button>
        </div>
        <p class="note">Values on the list sort in that order (capitals and accents ignored); other values follow.</p>
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
</style>
