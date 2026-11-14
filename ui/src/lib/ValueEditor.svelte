<script lang="ts">
  // The value editor: every distinct value of a column with how many rows
  // hold it. Type a new value next to one to change every cell holding it,
  // merge look-alike spellings in one click, or apply a translation list
  // (old → new) pasted in or read from two columns of another tab. One undo
  // step; with a filter on, only shown rows change.
  import { untrack } from "svelte";
  import { backend, errorText, type DataOp, type ValueTally } from "./backend";
  import { editPairs, parsePairs } from "./editing";
  import FormShell from "./FormShell.svelte";
  import type { OtherTable } from "./transfer";

  interface Props {
    doc: number;
    titles: string[];
    col: number;
    header: boolean;
    others: OtherTable[];
    titlesOf: (id: number, header: boolean) => Promise<string[]>;
    onRun: (op: DataOp, header: boolean) => void;
    onClose: () => void;
  }
  let { doc, titles, col: startCol, header, others, titlesOf, onRun, onClose }: Props = $props();

  let col = $state(untrack(() => startCol));
  let myHeader = $state(untrack(() => header));
  let mode = $state<"edit" | "list">("edit");
  let tally = $state<ValueTally | null>(null);
  let loading = $state(false);
  let problem = $state("");
  let search = $state("");
  /** Old value → new value typed in. */
  let edits = $state(new Map<string, string>());
  // Translation list.
  let from = $state<"paste" | "tab">("paste");
  let pasted = $state("");
  let other = $state<number>(untrack(() => others.find((o) => o.id !== doc)?.id ?? -1));
  let otherHeader = $state(true);
  let otherTitles = $state<string[]>([]);
  let oldCol = $state(0);
  let newCol = $state(1);
  let loose = $state(true);

  const SHOWN = 2000;

  $effect(() => {
    const c = col;
    const h = myHeader;
    loading = true;
    problem = "";
    edits = new Map();
    let live = true;
    backend()
      .valueTally(doc, { col: c, keep_first: h })
      .then((t) => {
        if (live) tally = t;
      })
      .catch((e: unknown) => {
        if (live) problem = errorText(e);
      })
      .finally(() => {
        if (live) loading = false;
      });
    return () => {
      live = false;
    };
  });

  $effect(() => {
    const id = other;
    const h = otherHeader;
    if (mode !== "list" || from !== "tab" || id < 0) return;
    void titlesOf(id, h).then((t) => (otherTitles = t));
  });

  const shown = $derived.by(() => {
    if (!tally) return [];
    const q = search.trim().toLowerCase();
    const list = tally.values.map((v, i) => ({ ...v, i }));
    return (q ? list.filter((v) => v.value.toLowerCase().includes(q)) : list).slice(0, SHOWN);
  });

  function setEdit(old: string, value: string): void {
    const next = new Map(edits);
    if (value === old || value === "") next.delete(old);
    else next.set(old, value);
    edits = next;
  }

  /** Every member of a look-alike group becomes `to`. */
  function merge(members: number[], to: string): void {
    if (!tally) return;
    const next = new Map(edits);
    for (const i of members) {
      const v = tally.values[i]!.value;
      if (v !== to) next.set(v, to);
      else next.delete(v);
    }
    edits = next;
  }

  const pairs = $derived(mode === "edit" ? editPairs(edits) : from === "paste" ? parsePairs(pasted) : []);
  const changing = $derived(
    mode === "edit" && tally ? editPairs(edits).reduce((n, [old]) => n + (tally!.values.find((v) => v.value === old)?.count ?? 0), 0) : 0,
  );

  function run(): void {
    problem = "";
    if (mode === "edit") {
      if (pairs.length === 0) {
        problem = "Type a new value next to at least one value, or merge a group.";
        return;
      }
      onRun({ op: "map_values", spec: { cols: [col], pairs: pairs.map(([a, b]) => [a, b]), match_case: true, trim: false, keep_first: myHeader } }, myHeader);
      return;
    }
    if (from === "paste") {
      if (pairs.length === 0) {
        problem = "Paste the list: one old value and its new value per line, separated by a tab, = or a comma.";
        return;
      }
      onRun({ op: "map_values", spec: { cols: [col], pairs, match_case: !loose, trim: loose, keep_first: myHeader } }, myHeader);
      return;
    }
    if (other < 0) {
      problem = "Open the tab that holds the list first.";
      return;
    }
    if (oldCol === newCol) {
      problem = "Choose two different columns for the old and new values.";
      return;
    }
    onRun(
      {
        op: "map_values",
        spec: { cols: [col], pairs: [], match_case: !loose, trim: loose, keep_first: myHeader },
        from: { doc: other, old: oldCol, new: newCol, header: otherHeader },
      },
      myHeader,
    );
  }
</script>

<FormShell title="Edit values" {onClose} onEnter={run}>
  <div class="row">
    <label class="row">Column
      <select bind:value={col}>
        {#each titles as t, c (c)}<option value={c}>{t}</option>{/each}
      </select>
    </label>
    <label class="row"><input type="checkbox" bind:checked={myHeader} /> First row is header</label>
  </div>
  <div class="row">
    <label class="row"><input type="radio" name="vmode" value="edit" bind:group={mode} /> Change the values listed</label>
    <label class="row"><input type="radio" name="vmode" value="list" bind:group={mode} /> Use a translation list (old → new)</label>
  </div>
  {#if mode === "edit"}
    {#if loading}
      <p class="note">Counting values…</p>
    {:else if tally}
      <p class="note">
        {tally.values.length.toLocaleString()} different values in {tally.rows.toLocaleString()} rows{tally.blanks ? `, ${tally.blanks.toLocaleString()} blank` : ""}.
        {#if !tally.complete}Only the {tally.values.length.toLocaleString()} most common are listed.{/if}
        Type a new value next to one to change every cell that holds it.
      </p>
      {#if tally.look_alikes.length}
        <fieldset>
          <legend>Look-alike spellings ({tally.look_alikes.length.toLocaleString()})</legend>
          <div class="groups">
            {#each tally.look_alikes.slice(0, 200) as g, k (k)}
              <div class="row">
                <span class="members">{g.members.map((i) => `${tally!.values[i]!.value} (${tally!.values[i]!.count.toLocaleString()})`).join(" · ")}</span>
                <select onchange={(e) => merge(g.members, (e.currentTarget as HTMLSelectElement).value)} aria-label="Merge into">
                  <option value="" selected disabled>Merge into…</option>
                  {#each g.members as i (i)}<option value={tally!.values[i]!.value}>{tally!.values[i]!.value}</option>{/each}
                </select>
              </div>
            {/each}
          </div>
        </fieldset>
      {/if}
      <input type="search" bind:value={search} placeholder="Find a value" aria-label="Find a value" />
      <div class="table-preview values" aria-label="Values and counts">
        <table>
          <tbody>
            <tr class="head"><td>Value</td><td>Rows</td><td>Change to</td></tr>
            {#each shown as v (v.i)}
              <tr class:edited={edits.has(v.value)}>
                <td>{v.value}</td>
                <td class="num">{v.count.toLocaleString()}</td>
                <td><input type="text" value={edits.get(v.value) ?? ""} placeholder="(keep)" oninput={(e) => setEdit(v.value, (e.currentTarget as HTMLInputElement).value)} aria-label={`New value for ${v.value}`} /></td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      {#if shown.length >= SHOWN}<p class="note">Showing {SHOWN.toLocaleString()} values; find a value to see others.</p>{/if}
      {#if pairs.length}<p class="good">{pairs.length.toLocaleString()} value{pairs.length === 1 ? "" : "s"} will change ({changing.toLocaleString()} cells).</p>{/if}
    {/if}
  {:else}
    <div class="row">
      <label class="row"><input type="radio" name="vfrom" value="paste" bind:group={from} /> Paste the list</label>
      <label class="row"><input type="radio" name="vfrom" value="tab" bind:group={from} /> Two columns of another tab</label>
    </div>
    {#if from === "paste"}
      <label class="col">One pair per line: old value, then the new one (separated by a tab, = or a comma)
        <textarea rows="6" bind:value={pasted} spellcheck="false" placeholder={"NY\tNew York\nCA\tCalifornia"}></textarea>
      </label>
      {#if pairs.length}<p class="good">{pairs.length.toLocaleString()} pair{pairs.length === 1 ? "" : "s"} read.</p>{/if}
    {:else}
      <label class="row">Tab
        <select bind:value={other}>
          {#each others as t (t.id)}<option value={t.id}>{t.name}{t.id === doc ? " (this one)" : ""}</option>{/each}
        </select>
      </label>
      <label class="row"><input type="checkbox" bind:checked={otherHeader} /> Its first row is its header</label>
      <div class="row">
        <label class="row">Old values in
          <select bind:value={oldCol}>{#each otherTitles as t, c (c)}<option value={c}>{t}</option>{/each}</select>
        </label>
        <label class="row">New values in
          <select bind:value={newCol}>{#each otherTitles as t, c (c)}<option value={c}>{t}</option>{/each}</select>
        </label>
      </div>
    {/if}
    <label class="row"><input type="checkbox" bind:checked={loose} /> Ignore case and spaces at the start and end</label>
    <p class="note">A whole cell must equal an old value; cells not on the list stay as they are.</p>
  {/if}
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" class="primary" onclick={run}>Change</button>
    <button type="button" onclick={onClose}>Cancel</button>
  {/snippet}
</FormShell>

<style>
  .values {
    max-height: 300px;
  }
  .values input {
    width: 14em;
  }
  td.num {
    text-align: right;
  }
  tr.edited td {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .groups {
    max-height: 140px;
    overflow: auto;
  }
  .members {
    font-size: 12px;
  }
  textarea {
    width: 100%;
    font-family: ui-monospace, monospace;
  }
  input[type="search"] {
    width: 100%;
    margin: 4px 0;
  }
</style>
