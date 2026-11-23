<script lang="ts">
  // The value editor: every distinct value of a column with how many rows
  // hold it. Type a new value next to one to change every cell holding it,
  // merge look-alike spellings in one click, or apply a translation list
  // (old → new) pasted in or read from two columns of another tab. One undo
  // step; with a filter on, only shown rows change.
  import { untrack } from "svelte";
  import { backend, errorText, isCmdError, type DataOp, type LookAlikeMethod, type LookAlikes, type ValueTally } from "./backend";
  import { editPairs, parsePairs } from "./editing";
  import FormShell from "./FormShell.svelte";
  import type { OtherTable } from "./transfer";

  interface Props {
    doc: number;
    titles: string[];
    col: number;
    header: boolean;
    /** How look-alikes are grouped when it opens. */
    startMethod?: "letters" | LookAlikeMethod;
    others: OtherTable[];
    titlesOf: (id: number, header: boolean) => Promise<string[]>;
    onRun: (op: DataOp, header: boolean) => void;
    onClose: () => void;
  }
  let { doc, titles, col: startCol, header, startMethod = "letters", others, titlesOf, onRun, onClose }: Props = $props();

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

  // Group look-alikes by sound or spelling (the first choice is the groups
  // the list already shows: the same letters in other case, spacing or accents).
  let method = $state<"letters" | LookAlikeMethod>(untrack(() => startMethod));
  let found = $state<LookAlikes | null>(null);
  let grouping = $state(false);
  let groupProgress = $state<number | null>(null);
  let groupProblem = $state("");
  /** Values ticked out of their group, by group number then value. */
  let dropped = $state(new Map<string, string>());
  /** Group number → the value the group becomes (default: the first). */
  let winners = $state(new Map<number, string>());
  let gen = 0;

  $effect(() => {
    const m = method;
    const c = col;
    const h = myHeader;
    found = null;
    groupProblem = "";
    dropped = new Map();
    winners = new Map();
    if (m === "letters") return;
    const mine = ++gen;
    grouping = true;
    groupProgress = null;
    void backend()
      .lookalikeGroups(doc, { col: c, keep_first: h, method: m })
      .then((r) => {
        if (mine === gen) found = r;
      })
      .catch((e: unknown) => {
        if (mine !== gen) return;
        groupProblem = isCmdError(e) && e.kind === "cancelled" ? "Stopped." : errorText(e);
      })
      .finally(() => {
        if (mine === gen) grouping = false;
      });
    return () => {
      gen++;
    };
  });

  $effect(() => {
    if (!grouping) return;
    const poll = window.setInterval(async () => {
      try {
        const j = await backend().jobProgress(doc);
        groupProgress = j ? j.fraction : null;
      } catch {
        // Closed meanwhile.
      }
    }, 200);
    return () => window.clearInterval(poll);
  });

  function stopGrouping(): void {
    void backend().cancelJob(doc);
  }

  /** The value a group becomes. */
  function winnerOf(k: number, g: { value: string }[]): string {
    return winners.get(k) ?? g[0]!.value;
  }
  function isDropped(k: number, v: string): boolean {
    return dropped.get(`${k}\u0000${v}`) !== undefined;
  }
  function setDropped(k: number, v: string, out: boolean): void {
    const next = new Map(dropped);
    if (out) next.set(`${k}\u0000${v}`, v);
    else next.delete(`${k}\u0000${v}`);
    dropped = next;
  }
  /** Put the group's ticked values into the list of changes, as the winner. */
  function mergeGroup(k: number, g: { value: string }[]): void {
    const to = winnerOf(k, g);
    const next = new Map(edits);
    for (const v of g) {
      if (isDropped(k, v.value) || v.value === to) {
        if (v.value === to) next.delete(v.value);
        continue;
      }
      next.set(v.value, to);
    }
    edits = next;
  }
  /** Every group at once, with each group's suggested value. */
  function mergeAll(): void {
    if (!found) return;
    found.groups.forEach((g, k) => mergeGroup(k, g));
  }

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
      <div class="row">
        <label class="row">Look-alikes:
          <select bind:value={method} aria-label="Group look-alikes by">
            <option value="letters">Same letters (case, spaces, accents)</option>
            <option value="sounds">Sound alike (Smith, Smyth)</option>
            <option value="spelling">Close spelling (Jonson, Johnson)</option>
            <option value="both">Sound or spelling</option>
          </select>
        </label>
      </div>
      {#if method !== "letters"}
        {#if grouping}
          <p class="note">
            Looking for look-alikes…{groupProgress !== null ? ` ${Math.round(groupProgress * 100)}%` : ""}
            <button type="button" class="small" onclick={stopGrouping}>Cancel</button>
          </p>
        {:else if groupProblem}
          <p class="problem" role="alert">{groupProblem}</p>
        {:else if found}
          <fieldset>
            <legend>{method === "sounds" ? "Sound alike" : method === "spelling" ? "Close spelling" : "Sound or spelling"} ({found.groups.length.toLocaleString()} {found.groups.length === 1 ? "group" : "groups"})</legend>
            {#if !found.complete}<p class="note">Only the {found.values.toLocaleString()} most common values were looked at.</p>{/if}
            {#if found.too_long}<p class="note">{found.too_long.toLocaleString()} very long {found.too_long === 1 ? "value was" : "values were"} left out of spelling groups.</p>{/if}
            {#if found.groups.length === 0}
              <p class="note">No look-alikes found.</p>
            {:else}
              <p class="note">
                Each group becomes the value chosen next to it (the most common one to begin with). Untick a value to leave it alone, then put the group in the list of changes.
                <button type="button" class="small" onclick={mergeAll}>Put all groups in the list</button>
              </p>
              <div class="groups" aria-label="Look-alike groups">
                {#each found.groups.slice(0, 200) as g, k (k)}
                  <div class="row group">
                    {#each g as v (v.value)}
                      <label class="row"><input type="checkbox" checked={!isDropped(k, v.value)} onchange={(e) => setDropped(k, v.value, !(e.currentTarget as HTMLInputElement).checked)} aria-label={`Include ${v.value}`} /> {v.value} ({v.count.toLocaleString()})</label>
                    {/each}
                    <select value={winnerOf(k, g)} onchange={(e) => { const next = new Map(winners); next.set(k, (e.currentTarget as HTMLSelectElement).value); winners = next; }} aria-label="Becomes">
                      {#each g as v (v.value)}<option value={v.value}>{v.value}</option>{/each}
                    </select>
                    <button type="button" class="small" onclick={() => mergeGroup(k, g)}>Put in list</button>
                  </div>
                {/each}
              </div>
              {#if found.groups.length > 200}<p class="note">Showing the first 200 groups.</p>{/if}
            {/if}
          </fieldset>
        {/if}
      {:else if tally.look_alikes.length}
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
  .group {
    flex-wrap: wrap;
    gap: 4px 12px;
    padding: 2px 0;
    border-bottom: 1px solid var(--border);
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
