<script lang="ts">
  // Notes and flags (Milestone 15): a note and a coloured flag on the
  // current row, kept in a small file beside the table; the list of every
  // noted row; show only noted or flagged rows; and the columns that
  // identify a row (so notes follow their rows).
  import AllNone from "./AllNone.svelte";
  import { untrack } from "svelte";
  import { backend, errorText } from "./backend";
  import FormShell from "./FormShell.svelte";
  import { FLAG_NAMES, type NoteMark } from "./notes";

  interface Props {
    doc: number;
    row: number;
    titles: string[];
    header: boolean;
    current: { flag: number; text: string } | null;
    onChanged: () => void;
    onGo: (row: number) => void;
    onShow: (flag: number) => void;
    onClose: () => void;
  }
  let { doc, row, titles, header, current, onChanged, onGo, onShow, onClose }: Props = $props();

  let flag = $state(untrack(() => current?.flag ?? 0));
  let text = $state(untrack(() => current?.text ?? ""));
  let list = $state<NoteMark[] | null>(null);
  let orphans = $state(0);
  let keys = $state<number[]>([]);
  let keysHeader = $state(untrack(() => header));
  let count = $state(0);
  let problem = $state("");
  let editKeys = $state(false);
  let showFlag = $state(0);

  async function load(): Promise<void> {
    try {
      const [k, h, n] = await backend().notesInfo(doc);
      keys = k;
      keysHeader = h;
      count = n;
      [list, orphans] = await backend().noteList(doc);
    } catch (e) {
      problem = errorText(e);
    }
  }
  void load();

  async function save(): Promise<void> {
    problem = "";
    try {
      await backend().noteSet(doc, row, flag, text);
      onChanged();
      await load();
    } catch (e) {
      problem = errorText(e);
    }
  }
  async function saveKeys(): Promise<void> {
    problem = "";
    try {
      await backend().notesSetKeys(doc, $state.snapshot(keys) as number[], keysHeader);
      editKeys = false;
      onChanged();
      await load();
    } catch (e) {
      problem = errorText(e);
    }
  }
  const article = (name: string) => ("aeiou".includes(name[0]!.toLowerCase()) ? "an" : "a");
  const title = (c: number) => titles[c] ?? `Column ${c + 1}`;
</script>

<FormShell title="Notes and flags" wide {onClose}>
  <p class="note">Notes and flags are kept in a small file beside this one (its name plus .ndnotes); the table itself is not changed. A row is found by {keys.map(title).join(" + ") || "its key columns"}, so its note stays with it when you sort, filter or open a newer copy.</p>
  <fieldset>
    <legend>Row {(row + 1).toLocaleString()}</legend>
    <div class="flags" role="radiogroup" aria-label="Flag">
      {#each FLAG_NAMES as [n, name, color] (n)}
        <label class="flag"><input type="radio" name="flag" value={n} checked={flag === n} onchange={() => (flag = n)} /><span class="swatch" style:background={color || "transparent"}></span>{name}</label>
      {/each}
    </div>
    <textarea bind:value={text} rows="3" maxlength="4000" placeholder="Note (optional)"></textarea>
    <div class="tools">
      <button type="button" class="primary" onclick={() => void save()}>Save note and flag</button>
      <button type="button" onclick={() => { flag = 0; text = ""; void save(); }}>Remove</button>
    </div>
  </fieldset>
  <fieldset>
    <legend>All notes ({count.toLocaleString()})</legend>
    <div class="tools">
      <label>Show only rows with
        <select bind:value={showFlag}>
          <option value={0}>any note or flag</option>
          {#each FLAG_NAMES.slice(1) as [n, name] (n)}<option value={n}>{article(name)} {name.toLowerCase()} flag</option>{/each}
        </select>
      </label>
      <button type="button" onclick={() => onShow(showFlag)}>Show them</button>
    </div>
    {#if list}
      <div class="list">
        {#each list as m (m.row)}
          <button type="button" class="line" onclick={() => onGo(m.row)}>
            <span class="swatch" style:background={FLAG_NAMES[m.flag]?.[2] || "transparent"}></span>
            <span>Row {(m.row + 1).toLocaleString()}</span><span class="text">{m.text || "(flag only)"}</span>
          </button>
        {/each}
      </div>
      {#if orphans}<p class="note">{orphans.toLocaleString()} {orphans === 1 ? "note matches" : "notes match"} no row now (the row was deleted or its key changed).</p>{/if}
    {/if}
  </fieldset>
  <details bind:open={editKeys}>
    <summary>Columns that identify a row</summary>
    <p class="note">Changing them leaves notes made with the old columns without a row.</p>
    <AllNone count={titles.length} picked={keys} onChange={(v) => (keys = v)} max={8} />
    <div class="keys">
      {#each titles as t, c (c)}
        <label><input type="checkbox" checked={keys.includes(c)} onchange={(e) => { keys = (e.currentTarget as HTMLInputElement).checked ? [...keys, c].slice(0, 8) : keys.filter((k) => k !== c); }} /> {t}</label>
      {/each}
    </div>
    <label class="row"><input type="checkbox" bind:checked={keysHeader} /> The first row is titles</label>
    <button type="button" disabled={!keys.length} onclick={() => void saveKeys()}>Use these columns</button>
  </details>
  {#if problem}<p class="warn">{problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" onclick={onClose}>Close</button>
  {/snippet}
</FormShell>

<style>
  .flags {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    margin-bottom: 6px;
  }
  .flag {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .swatch {
    display: inline-block;
    width: 12px;
    height: 12px;
    border-radius: 2px;
    border: 1px solid var(--border, #8884);
  }
  textarea {
    width: 100%;
    box-sizing: border-box;
  }
  .tools {
    display: flex;
    gap: 6px;
    align-items: center;
    flex-wrap: wrap;
    margin: 6px 0;
  }
  .list {
    max-height: 220px;
    overflow: auto;
    border: 1px solid var(--border, #8884);
    border-radius: 4px;
  }
  .line {
    display: grid;
    grid-template-columns: 18px 8em 1fr;
    gap: 6px;
    width: 100%;
    text-align: left;
    background: none;
    border: 0;
    color: inherit;
    font: inherit;
    padding: 2px 8px;
    align-items: center;
  }
  .line:hover {
    background: var(--hover, #8882);
  }
  .text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .keys {
    max-height: 120px;
    overflow: auto;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(12em, 1fr));
  }
  .warn {
    color: var(--warn, #b45309);
  }
</style>
