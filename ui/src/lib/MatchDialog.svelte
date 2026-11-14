<script lang="ts">
  // Match people (Milestone 15): pick the other list (or this table, for
  // near-duplicates) and the columns to compare; the app scores pairs that
  // look like the same person; accept or reject them, then add a "Match
  // row" and "Match score" column (and columns brought in) for the
  // accepted ones.
  import { untrack } from "svelte";
  import { backend, errorText } from "./backend";
  import FormShell from "./FormShell.svelte";
  import type { OtherTable } from "./transfer";
  import { KIND_LABELS, acceptedPairs, guessFields, type MatchField, type MatchReport, type MatchSpec } from "./match";

  interface Props {
    doc: number;
    titles: string[];
    header: boolean;
    others: OtherTable[];
    titlesOf: (id: number, header: boolean) => Promise<string[]>;
    onOpen: () => Promise<OtherTable | null>;
    /** Accepted pairs were added: the app's new table info. */
    onDone: (info: import("./backend").DocInfo, pairs: number) => void;
    onClose: () => void;
  }
  let { doc, titles, header, others, titlesOf, onOpen, onDone, onClose }: Props = $props();

  const SAME = -1;
  let list = $state<OtherTable[]>(untrack(() => others.filter((o) => o.id !== doc)));
  let other = $state<number>(untrack(() => list[0]?.id ?? SAME));
  let myHeader = $state(untrack(() => header));
  let otherHeader = $state(true);
  let otherTitles = $state<string[]>([]);
  let fields = $state<MatchField[]>([]);
  let bring = $state<number[]>([]);
  let minScore = $state(85);
  let dayFirst = $state(false);
  let report = $state<MatchReport | null>(null);
  let accepted = $state(new Set<number>());
  let running = $state(false);
  let problem = $state("");
  let page = $state(0);
  const PAGE = 200;

  const theirs = $derived(other === SAME ? titles : otherTitles);

  $effect(() => {
    const o = other;
    const h = otherHeader;
    report = null;
    if (o === SAME) {
      otherTitles = titles;
      if (!fields.length) fields = untrack(() => guessFields(titles, titles));
      return;
    }
    void titlesOf(o, h).then((t) => {
      otherTitles = t;
      if (!untrack(() => fields.length)) fields = guessFields(titles, t);
    });
  });

  function add(): void {
    if (fields.length < 8) fields = [...fields, { col: 0, other_col: 0, kind: "name", weight: 1 }];
  }
  function remove(i: number): void {
    fields = fields.filter((_, k) => k !== i);
  }
  function spec(): MatchSpec {
    return {
      fields: $state.snapshot(fields) as MatchField[],
      keep_first: myHeader,
      other_header: other === SAME ? myHeader : otherHeader,
      min_score: minScore,
      read: { decimal_comma: false, date_order: dayFirst ? "dmy" : "mdy" },
    };
  }
  async function find(): Promise<void> {
    if (!fields.length || running) return;
    running = true;
    problem = "";
    try {
      report = await backend().matchFind(doc, other === SAME ? null : other, spec(), other === SAME ? [] : $state.snapshot(bring) as number[]);
      accepted = new Set();
      page = 0;
    } catch (e) {
      report = null;
      problem = errorText(e);
    } finally {
      running = false;
    }
  }
  async function openFile(): Promise<void> {
    const o = await onOpen();
    if (o) {
      list = [...list.filter((x) => x.id !== o.id), o];
      other = o.id;
    }
  }
  function toggle(i: number): void {
    const s = new Set(accepted);
    if (s.has(i)) s.delete(i);
    else s.add(i);
    accepted = s;
  }
  let acceptAt = $state(95);
  function acceptAbove(): void {
    if (!report) return;
    const s = new Set(accepted);
    report.listed.forEach((c, i) => {
      if (c.score >= acceptAt) s.add(i);
    });
    accepted = s;
  }
  async function apply(): Promise<void> {
    if (!report || !accepted.size || running) return;
    running = true;
    problem = "";
    try {
      const info = await backend().matchAccept(doc, acceptedPairs(report.listed, accepted));
      onDone(info, accepted.size);
    } catch (e) {
      problem = errorText(e);
    } finally {
      running = false;
    }
  }
  const plural = (n: number, one: string, many = `${one}s`) => `${n.toLocaleString()} ${n === 1 ? one : many}`;
  const shown = $derived(report ? report.listed.slice(page * PAGE, page * PAGE + PAGE) : []);
  const pct = (x: number) => (x < 0 ? "blank" : `${Math.round(x * 100)}%`);
</script>

<FormShell title="Match people" wide {onClose}>
  {#if !report}
    <p class="note">Finds rows that look like the same person even when spelled differently (Bill and William, Smith and Smyth, a swapped day and month). Each pair gets a score out of 100; you accept or reject them before anything changes.</p>
    <label class="row">Match with
      <select bind:value={other}>
        {#each list as o (o.id)}<option value={o.id}>{o.name}</option>{/each}
        <option value={SAME}>this table itself (find near-duplicates)</option>
      </select>
      <button type="button" onclick={() => void openFile()}>Open a file…</button>
    </label>
    <label class="row"><input type="checkbox" bind:checked={myHeader} /> This table's first row is titles</label>
    {#if other !== SAME}<label class="row"><input type="checkbox" bind:checked={otherHeader} /> The other list's first row is titles</label>{/if}
    <div class="fields">
      <div class="head"><span>This table</span><span>{other === SAME ? "Same table" : "Other list"}</span><span>Compare as</span><span>Counts</span><span></span></div>
      {#each fields as f, i (i)}
        <div class="line">
          <select aria-label="Column in this table" bind:value={f.col}>{#each titles as t, c (c)}<option value={c}>{t}</option>{/each}</select>
          <select aria-label="Column in the other list" bind:value={f.other_col} disabled={other === SAME}>{#each theirs as t, c (c)}<option value={c}>{t}</option>{/each}</select>
          <select aria-label="How to compare" bind:value={f.kind}>{#each KIND_LABELS as [k, l] (k)}<option value={k}>{l}</option>{/each}</select>
          <select aria-label="How much it counts" bind:value={f.weight}><option value={0.5}>a little</option><option value={1}>normal</option><option value={2}>a lot</option></select>
          <button type="button" onclick={() => remove(i)} aria-label="Remove this field">✕</button>
        </div>
      {/each}
      <button type="button" onclick={add} disabled={fields.length >= 8}>Add a column to compare</button>
    </div>
    {#if other === SAME}<p class="note">For near-duplicates the same columns are compared with each other; each later row is paired with the earlier row it looks most like.</p>{/if}
    <label class="row">List pairs scoring at least <input type="number" min="50" max="100" step="1" bind:value={minScore} /></label>
    <label class="row"><input type="checkbox" bind:checked={dayFirst} /> Dates are day first (13/05/1980)</label>
    {#if other !== SAME && theirs.length}
      <details>
        <summary>Bring in columns from the other list for accepted pairs ({bring.length} chosen)</summary>
        <div class="bring">
          {#each theirs as t, c (c)}
            <label><input type="checkbox" checked={bring.includes(c)} onchange={(e) => { bring = (e.currentTarget as HTMLInputElement).checked ? [...bring, c].slice(0, 64) : bring.filter((x) => x !== c); }} /> {t}</label>
          {/each}
        </div>
      </details>
    {/if}
  {:else}
    <p>{report.total ? `${plural(report.total, "pair")} found (${plural(report.compared, "comparison")}).` : "No pairs at that score."}
      {#if report.total > report.listed.length}The best {report.listed.length.toLocaleString()} are listed.{/if}
      {#if report.windowed}<span class="warn">{plural(report.windowed, "row")} share a very common name, so each was compared only with the closest similar rows (by birth date or spelling), not with every row of that name. Add a column such as a birth date or address to compare them fully.</span>{/if}
    </p>
    {#if report.listed.length}
      <div class="tools">
        <label>Accept every pair scoring at least <input type="number" min="50" max="100" bind:value={acceptAt} /></label>
        <button type="button" onclick={acceptAbove}>Accept</button>
        <button type="button" onclick={() => (accepted = new Set())}>Reject all</button>
        <span>{plural(accepted.size, "pair")} accepted</span>
      </div>
      <div class="pairs" aria-label="Possible matches">
        <div class="phead"><span>Accept</span><span>Score</span><span>Row</span><span>Here</span><span>{other === SAME ? "Earlier row" : "Other row"}</span><span>There</span></div>
        {#each shown as c, k (page * PAGE + k)}
          {@const i = page * PAGE + k}
          <label class="pline">
            <span><input type="checkbox" checked={accepted.has(i)} onchange={() => toggle(i)} /></span>
            <span class="score">{c.score}</span>
            <span>{(c.row + 1).toLocaleString()}</span>
            <span title={c.parts.map(pct).join(", ")}>{c.here.join(" · ")}</span>
            <span>{(c.other_row + 1).toLocaleString()}</span>
            <span>{c.there.join(" · ")}</span>
          </label>
        {/each}
      </div>
      {#if report.listed.length > PAGE}
        <div class="tools">
          <button type="button" disabled={page === 0} onclick={() => page--}>Previous</button>
          <span>Pairs {(page * PAGE + 1).toLocaleString()} to {Math.min(report.listed.length, (page + 1) * PAGE).toLocaleString()}</span>
          <button type="button" disabled={(page + 1) * PAGE >= report.listed.length} onclick={() => page++}>Next</button>
        </div>
      {/if}
      <p class="note">Values are shown as compared (small letters, accents and titles off). Hover a "Here" cell to see how alike each field is.</p>
    {/if}
  {/if}
  {#if problem}<p class="warn">{problem}</p>{/if}
  {#snippet buttons()}
    {#if !report}
      <button type="button" class="primary" disabled={!fields.length || running} onclick={() => void find()}>{running ? "Matching…" : "Find matches"}</button>
    {:else}
      <button type="button" class="primary" disabled={!accepted.size || running} onclick={() => void apply()}>Add columns for {plural(accepted.size, "accepted pair")}</button>
      <button type="button" onclick={() => (report = null)}>Change the setup</button>
    {/if}
    <button type="button" onclick={onClose}>Close</button>
  {/snippet}
</FormShell>

<style>
  .fields .head,
  .fields .line {
    display: grid;
    grid-template-columns: 1fr 1fr 14em 7em 2.5em;
    gap: 6px;
    margin-bottom: 4px;
  }
  .fields .head {
    font-weight: 600;
  }
  .bring {
    max-height: 160px;
    overflow: auto;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(12em, 1fr));
  }
  .tools {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
    margin: 6px 0;
  }
  .tools input {
    width: 4em;
  }
  .pairs {
    max-height: 340px;
    overflow: auto;
    border: 1px solid var(--border, #8884);
    border-radius: 4px;
  }
  .phead,
  .pline {
    display: grid;
    grid-template-columns: 4em 4em 6em 1fr 6em 1fr;
    gap: 6px;
    padding: 2px 8px;
  }
  .phead {
    font-weight: 600;
    position: sticky;
    top: 0;
    background: var(--panel, #8881);
  }
  .pline span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pline:hover {
    background: var(--hover, #8882);
  }
  .score {
    font-variant-numeric: tabular-nums;
  }
  .warn {
    color: var(--warn, #b45309);
  }
</style>
