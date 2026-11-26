<script lang="ts">
  // Golden records: how each column's value is chosen, a preview of every
  // cluster (disagreements shown, doubts highlighted), then a new tab. The
  // matched table is never changed.
  import { untrack } from "svelte";
  import { backend, errorText } from "./backend";
  import FormShell from "./FormShell.svelte";
  import {
    GOLDEN_PAGE,
    REVIEW_SHOWN,
    RULE_LABELS,
    guessIdColumn,
    makeRule,
    notes,
    summaryText,
    type GoldenCluster,
    type GoldenMade,
    type GoldenRuleName,
    type GoldenSpec,
    type GoldenSummary,
  } from "./golden";

  interface Props {
    doc: number;
    titles: string[];
    header: boolean;
    dayFirst: boolean;
    pairs: [number, number, number][];
    onCreated: (made: GoldenMade, summary: GoldenSummary, away: boolean) => void;
    /** Back to the pairs. */
    onBack: () => void;
  }
  let { doc, titles, header, dayFirst, pairs, onCreated, onBack }: Props = $props();

  let rules = $state<GoldenRuleName[]>(untrack(() => titles.map(() => "keep" as GoldenRuleName)));
  let separator = $state("; ");
  let blankCounts = $state(false);
  let idCol = $state<number | null>(untrack(() => guessIdColumn(titles)));
  let dateCol = $state<number | null>(null);
  let maxCluster = $state(10);
  let requireAll = $state(false);
  let away = $state(true);
  let skip = $state(new Set<number>());
  let summary = $state<GoldenSummary | null>(null);
  let clusters = $state<GoldenCluster[]>([]);
  let page = $state(0);
  /** The setup and the skips the preview was made with. */
  let planned = $state("");
  let plannedSkips = $state("");
  let busy = $state(false);
  let problem = $state("");

  const needsDate = $derived(rules.some((r) => r === "newest" || r === "oldest"));
  const setup = $derived(JSON.stringify([rules, separator, blankCounts, idCol, dateCol, maxCluster, requireAll]));
  const stale = $derived(!!summary && planned !== setup);
  const skips = $derived(JSON.stringify([...skip].sort((a, b) => a - b)));

  function spec(): GoldenSpec {
    return {
      rules: rules.flatMap((r, col) => (r === "keep" ? [] : [{ col, rule: makeRule(r, separator, blankCounts) }])),
      id_col: idCol,
      date_col: dateCol,
      max_cluster: maxCluster,
      require_all: requireAll,
      skip: [...skip],
      keep_first: header,
      read: { decimal_comma: false, date_order: dayFirst ? "dmy" : "mdy" },
    };
  }

  async function work(f: () => Promise<void>): Promise<void> {
    if (busy) return;
    busy = true;
    problem = "";
    try {
      await f();
    } catch (e) {
      problem = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function plan(): Promise<void> {
    const made = setup;
    const left = skips;
    summary = await backend().goldenPlan(doc, pairs, spec());
    planned = made;
    plannedSkips = left;
  }

  function preview(): void {
    // The skips belong to clusters that may now be different ones.
    if (summary && planned !== setup) skip = new Set();
    void work(async () => {
      summary = null;
      await plan();
      page = 0;
      clusters = await backend().goldenPage(doc, 0);
    });
  }

  function turn(to: number): void {
    void work(async () => {
      clusters = await backend().goldenPage(doc, to);
      page = to;
    });
  }

  function merge(hub: number, on: boolean): void {
    const s = new Set(skip);
    if (on) s.delete(hub);
    else s.add(hub);
    skip = s;
  }

  function create(): void {
    if (!summary || stale) return;
    void work(async () => {
      if (plannedSkips !== skips) await plan();
      const s = summary!;
      onCreated(await backend().goldenCreate(doc, away), s, away);
    });
  }

  const merging = $derived(summary ? summary.listed - skip.size : 0);
  const title = (c: number) => titles[c] ?? `Column ${c + 1}`;
</script>

<FormShell title="Merge into golden records" wide onClose={onBack}>
  <p class="note">Makes a new table with one row for each person in the {pairs.length.toLocaleString()} accepted {pairs.length === 1 ? "pair" : "pairs"}. Your table is not changed. Rows merge only with a row they were accepted with; doubtful ones are left for you to review.</p>
  <details open={!summary}>
    <summary>How each column's value is chosen</summary>
    <p class="note">A blank never replaces a value. Values stay as typed; only sums are rewritten. "All different values" treats Tag and tag as different.</p>
    <div class="rules">
      {#each titles as t, c (c)}
        <label><span>{t}</span><select aria-label={`Rule for ${t}`} bind:value={rules[c]}>{#each RULE_LABELS as [k, l] (k)}<option value={k}>{l}</option>{/each}</select></label>
      {/each}
    </div>
    {#if rules.includes("union")}<label class="row">Join with <input type="text" bind:value={separator} maxlength="20" /></label>{/if}
    {#if needsDate}
      <label class="row">Date column <select bind:value={dateCol}><option value={null}>choose…</option>{#each titles as t, c (c)}<option value={c}>{t}</option>{/each}</select></label>
      {#if rules.includes("newest")}<label class="row"><input type="checkbox" bind:checked={blankCounts} /> A newer blank replaces an older value</label>{/if}
      <p class="note">On equal dates the first row in the table wins. If any date in a cluster can be read two ways (03/04/2026), has a two-digit year or cannot be read, the first value is kept and the cell is highlighted.</p>
    {/if}
  </details>
  <label class="row">Name merged rows by <select bind:value={idCol}><option value={null}>row number in the file</option>{#each titles as t, c (c)}<option value={c}>{t}</option>{/each}</select></label>
  <p class="note">The new columns Merged_IDs (the main row first, then in the table's order) and Merged_Count say which rows became which. "row 5" is the fifth line of the file, counting the title row. Merged_IDs is only as sensitive as the column you name the rows by.</p>
  <label class="row">Largest cluster to merge <input type="number" min="2" max="100" bind:value={maxCluster} /> rows</label>
  <label class="row"><input type="checkbox" bind:checked={requireAll} /> Only merge clusters where every pair was accepted</label>
  <p class="note">Match people lists one pair for each later row, so with this ticked only clusters of two rows merge; larger ones are left for review.</p>
  <label class="row"><input type="checkbox" bind:checked={away} /> Also open the folded rows in a second tab ("Merged away")</label>

  {#if stale}<p class="warn" role="status">The choices changed: preview again.</p>{/if}
  {#if summary && !stale}
    <p><b>{summaryText(summary.counts)}</b></p>
    {#each notes(summary) as line (line)}<p class="note">{line}</p>{/each}
    {#if summary.review.length}
      <details>
        <summary>Rows to review</summary>
        <ul class="review">
          {#each summary.review.slice(0, REVIEW_SHOWN) as r, i (i)}<li><b>{r.name}</b>: {r.reason}</li>{/each}
        </ul>
        {#if summary.counts.review > REVIEW_SHOWN}<p class="note">The first {REVIEW_SHOWN} of {summary.counts.review.toLocaleString()} are listed.</p>{/if}
      </details>
    {/if}
    <div class="clusters" aria-label="Clusters">
      {#each clusters as c, k (c.hub)}
        <fieldset class:off={skip.has(c.hub)}>
          <legend>Cluster {page * GOLDEN_PAGE + k + 1}: {c.members.join("; ")}</legend>
          <label><input type="checkbox" checked={!skip.has(c.hub)} onchange={(e) => merge(c.hub, (e.currentTarget as HTMLInputElement).checked)} /> Merge this cluster</label>
          {#if c.hub_only}<p class="only">Joined through {c.members[0]} only: not every pair here was accepted.</p>{/if}
          {#if c.cells.length}
            <table>
              <thead><tr><th>Column</th><th>Result</th><th>Other values</th></tr></thead>
              <tbody>
                {#each c.cells as cell (cell.col)}
                  <tr class:doubt={!!cell.flag}>
                    <th scope="row">{title(cell.col)}</th>
                    <td>{cell.value || "(blank)"}{#if cell.flag}<span class="why">{cell.flag}</span>{/if}</td>
                    <td>{cell.differ.map((d) => d || "(blank)").join(" · ")}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {:else}
            <p class="note">Every column agrees.</p>
          {/if}
        </fieldset>
      {/each}
    </div>
    {#if summary.listed > GOLDEN_PAGE}
      <div class="row">
        <button type="button" disabled={page === 0 || busy} onclick={() => turn(page - 1)}>Previous</button>
        <span>Page {page + 1} of {Math.ceil(summary.listed / GOLDEN_PAGE)}</span>
        <button type="button" disabled={(page + 1) * GOLDEN_PAGE >= summary.listed || busy} onclick={() => turn(page + 1)}>Next</button>
      </div>
    {/if}
  {/if}
  {#if problem}<p class="warn" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" class={summary && !stale ? "" : "primary"} data-enter={summary && !stale ? undefined : true} disabled={busy || (needsDate && dateCol === null)} onclick={preview}>{busy ? "Working…" : "Preview"}</button>
    <button type="button" class={summary && !stale ? "primary" : ""} disabled={busy || !summary || stale || merging < 1} onclick={create}>Create the new table</button>
    <button type="button" onclick={onBack}>Back to the pairs</button>
  {/snippet}
</FormShell>

<style>
  .rules {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(22em, 1fr));
    gap: 4px 12px;
    max-height: 180px;
    overflow: auto;
  }
  .rules label {
    display: flex;
    justify-content: space-between;
    gap: 8px;
  }
  .rules span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .clusters {
    max-height: 300px;
    overflow: auto;
  }
  fieldset {
    margin-bottom: 8px;
  }
  fieldset.off table {
    opacity: 0.5;
  }
  .only {
    color: var(--warn, #b45309);
    font-weight: 600;
    margin: 4px 0;
  }
  table {
    border-collapse: collapse;
    width: 100%;
  }
  th,
  td {
    border-bottom: 1px solid var(--border, #8884);
    padding: 2px 8px;
    text-align: left;
    vertical-align: top;
  }
  tr.doubt {
    background: #f59e0b33;
  }
  .why {
    display: block;
    color: var(--warn, #b45309);
  }
  .review {
    max-height: 140px;
    overflow: auto;
  }
  .warn {
    color: var(--warn, #b45309);
  }
</style>
