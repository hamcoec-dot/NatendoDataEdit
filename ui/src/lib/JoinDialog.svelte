<script lang="ts">
  // Join (bring values in from another table where key values match, or
  // keep or drop the rows on a list) and Append (add another table's rows),
  // from another open tab or, for a join, a pasted list of keys. "Open a
  // file…" opens one first. Either is one undo step. A join shows the
  // first rows as they would look, with counts for the whole table.
  import OtherHidden from "./OtherHidden.svelte";
  import AllNone from "./AllNone.svelte";
  import { untrack } from "svelte";
  import { backend, errorText, type DataOp, type JoinPreview, type JoinSpec } from "./backend";
  import FormShell from "./FormShell.svelte";
  import {
    COMPARES,
    compareOf,
    defaultSpec,
    guessTargets,
    joinCounts,
    joinWarnings,
    KEEPS,
    REPEATS,
    valueCount,
    withCompare,
    WRITES,
    type Compare,
  } from "./joins";
  import type { OtherTable } from "./transfer";

  interface Props {
    kind: "join" | "append";
    /** This document. */
    doc: number;
    titles: string[];
    header: boolean;
    /** Shown rows of this document. */
    rows?: number;
    others: OtherTable[];
    titlesOf: (id: number, header: boolean) => Promise<string[]>;
    onOpen: () => Promise<OtherTable | null>;
    onRun: (op: DataOp, header: boolean) => void;
    onClose: () => void;
  }
  let { kind, doc, titles, header, rows = 0, others, titlesOf, onOpen, onRun, onClose }: Props = $props();

  let list = $state<OtherTable[]>(untrack(() => others));
  let other = $state<number>(untrack(() => others.find((o) => o.id !== doc)?.id ?? others[0]?.id ?? -1));
  let myHeader = $state(untrack(() => header));
  let otherHeader = $state(untrack(() => others.find((o) => o.id === other)?.header ?? true));
  let otherTitles = $state<string[]>([]);
  // Join.
  let fromList = $state(false);
  let pasted = $state("");
  let pairs = $state<[number, number][]>([[0, 0]]);
  // Columns to bring in: none ticked to start (null: every non-key column).
  let bring = $state<number[] | null>([]);
  let matchCase = $state(false);
  let trim = $state(true);
  let compare = $state<Compare>("exact");
  let addUnmatched = $state(false);
  let write = $state<JoinSpec["write"]>("new");
  let keepRows = $state<JoinSpec["keep_rows"]>("all");
  let repeats = $state<JoinSpec["repeats"]>("first");
  let separator = $state("; ");
  let notFound = $state("");
  let place = $state<JoinSpec["place"]>("end");
  let prefix = $state("");
  /** Update: this table's column for each brought-in column (by its number). */
  let into = $state<Record<number, number>>({});
  // Append.
  let byName = $state(true);
  let addColumns = $state(true);
  let problem = $state("");
  let preview = $state<JoinPreview | null>(null);
  let previewProblem = $state("");
  let previewing = $state(false);

  $effect(() => {
    const id = other;
    const h = otherHeader;
    if (id < 0 || fromList) return;
    void titlesOf(id, h).then((t) => {
      otherTitles = t;
      // Match the first key by name when both tables have one.
      untrack(() => {
        if (kind === "join" && pairs.length === 1) {
          const mine = titles.map((x) => x.trim().toLowerCase());
          const hit = t.findIndex((x) => mine.includes(x.trim().toLowerCase()));
          if (hit >= 0) pairs = [[mine.indexOf(t[hit]!.trim().toLowerCase()), hit]];
        }
      });
    });
  });

  async function open(): Promise<void> {
    const t = await onOpen();
    if (!t) return;
    list = [...list.filter((x) => x.id !== t.id), t];
    other = t.id;
    otherHeader = t.header;
    fromList = false;
  }

  function choose(id: number): void {
    other = id;
    otherHeader = list.find((o) => o.id === id)?.header ?? true;
    bring = null;
    into = {};
  }

  const keyCols = $derived(pairs.map((p) => p[1]));
  const bringList = $derived(fromList ? [] : (bring ?? otherTitles.map((_, i) => i).filter((i) => !keyCols.includes(i))));
  const update = $derived(write === "overwrite" || write === "fill_blanks");
  const writes = $derived(write !== "nothing" && keepRows !== "unmatched");
  const range = $derived(compare === "below" || compare === "above");
  /** Update: where each value goes ("count" brings one value). */
  const targets = $derived.by(() => {
    if (!update) return [];
    if (repeats === "count") return [into[-1] ?? -1];
    const guess = guessTargets(titles, otherTitles, bringList, pairs.map((p) => p[0]));
    return bringList.map((c, k) => into[c] ?? guess[k] ?? -1);
  });

  /** A list brings in no values: start with keeping the rows on it. */
  function toList(): void {
    write = "nothing";
    if (keepRows === "all") keepRows = "matched";
  }

  function toggle(c: number): void {
    const cur = bringList;
    bring = cur.includes(c) ? cur.filter((x) => x !== c) : [...cur, c].sort((a, b) => a - b);
  }

  function spec(): JoinSpec {
    return {
      ...defaultSpec(),
      keys: pairs.map((p) => p[0]),
      other_keys: fromList ? pairs.map((_, i) => i) : pairs.map((p) => p[1]),
      columns: bring === null || fromList ? [] : $state.snapshot(bring),
      keep_first: myHeader,
      other_header: otherHeader,
      match_case: matchCase,
      trim,
      add_unmatched: addUnmatched && !fromList && !range,
      write: fromList && write !== "nothing" ? "new" : write,
      targets: [...targets],
      keep_rows: keepRows,
      repeats: fromList ? "count" : repeats,
      separator,
      not_found: notFound,
      place,
      prefix,
      ...withCompare(compare),
    };
  }

  /** What is wrong before anything is asked of the engine. */
  function check(s: JoinSpec): string {
    if (!fromList && other < 0) return "Open the other table first.";
    if (fromList && !pasted.trim()) return "Paste the list of keys, one per line.";
    if (fromList) return s.write === "nothing" && s.keep_rows === "all" ? "Choose which rows to keep, or add the count." : "";
    if (writes && s.write !== "nothing" && valueCount(s, bringList.length) === 0) return "Tick at least one column to bring in.";
    if (update && writes && targets.some((t) => t < 0)) return "Choose a column of this table for each value brought in.";
    if (update && writes && new Set(targets).size !== targets.length) return "Each column can be updated only once.";
    if (s.write === "nothing" && s.keep_rows === "all") return "Choose which rows to keep, or bring in some values.";
    return "";
  }

  // A pasted list is handed to the app once; previews name it by number.
  let listId: number | null = null;
  let listFor = "";
  async function keptList(text: string): Promise<number> {
    if (listId !== null && listFor === text) return listId;
    listId = await backend().joinList(doc, text);
    listFor = text;
    return listId;
  }

  // One preview at a time: a change while one runs waits for it, and only
  // the newest waiting change is previewed next (the app also stops an
  // older preview when a newer one starts).
  let busy = false;
  let waiting: (() => Promise<void>) | null = null;
  function request(job: () => Promise<void>): void {
    if (busy) {
      waiting = job;
      return;
    }
    busy = true;
    void job().finally(() => {
      busy = false;
      const next = waiting;
      waiting = null;
      if (next) request(next);
    });
  }

  let seq = 0;
  $effect(() => {
    if (kind !== "join") return;
    const s = spec();
    const text = fromList ? pasted : null;
    const id = other;
    const mine = ++seq;
    const bad = check(s);
    if (bad) {
      preview = null;
      previewProblem = bad;
      return;
    }
    previewing = true;
    const t = setTimeout(() => {
      request(async () => {
        if (mine !== seq) return;
        try {
          const lid = text !== null ? await keptList(text) : null;
          const p = await backend().joinPreview(doc, text !== null ? null : id, null, s, lid);
          if (mine !== seq) return;
          preview = p;
          previewProblem = "";
        } catch (e: unknown) {
          if (mine !== seq) return;
          preview = null;
          previewProblem = errorText(e);
        } finally {
          if (mine === seq) previewing = false;
        }
      });
    }, 300);
    return () => clearTimeout(t);
  });

  // The app keeps the indexed other table while this screen is open.
  $effect(() => {
    const d = doc;
    return () => void backend().joinClose(d).catch(() => {});
  });

  const warnings = $derived(preview ? joinWarnings(preview.report, rows) : []);

  function run(): void {
    if (kind === "join") {
      const s = spec();
      const bad = check(s);
      if (bad) {
        problem = bad;
        return;
      }
      onRun({ op: "join", other: fromList ? null : other, list: fromList ? pasted : null, spec: s }, myHeader);
    } else {
      if (other < 0) {
        problem = "Open the other table first.";
        return;
      }
      onRun({ op: "append", other, spec: { by_name: byName, keep_first: myHeader, other_header: otherHeader, add_columns: addColumns } }, myHeader);
    }
  }

  const KIND_LABEL: Record<string, string> = {
    matched: "Matched",
    unmatched: "No match",
    blank: "Blank key",
    removed: "Taken out",
    extra: "Extra match",
    added: "Added",
  };
</script>

<FormShell title={kind === "join" ? "Join with another table" : "Append another table"} {onClose} onEnter={run}>
  {#if kind === "join"}
    <div class="row">
      <label class="row"><input type="radio" name="from" value={false} bind:group={fromList} /> Another table</label>
      <label class="row"><input type="radio" name="from" value={true} bind:group={fromList} onchange={toList} /> A pasted list of keys</label>
    </div>
  {/if}
  {#if !fromList || kind === "append"}
    <div class="row">
      <label class="row">Other table
        <select value={other} onchange={(e) => choose(Number((e.currentTarget as HTMLSelectElement).value))}>
          {#each list as t (t.id)}<option value={t.id}>{t.name}{t.id === doc ? " (this one)" : ""}</option>{/each}
        </select>
      </label>
      <button type="button" onclick={() => void open()}>Open a file…</button>
    </div>
  {:else}
    <label class="col">Keys, one per line (several key columns: separate them with tabs)
      <textarea rows="4" bind:value={pasted} spellcheck="false"></textarea>
    </label>
  {/if}
  <div class="grid2">
    <label class="row"><input type="checkbox" bind:checked={myHeader} /> This table's first row is its header</label>
    {#if !fromList || kind === "append"}<label class="row"><input type="checkbox" bind:checked={otherHeader} /> The other table's first row is its header</label>{/if}
  </div>
  {#if kind === "join"}
    <fieldset>
      <legend>Match rows where</legend>
      {#each pairs as p, i (i)}
        <div class="row">
          <select value={p[0]} onchange={(e) => (pairs[i] = [Number((e.currentTarget as HTMLSelectElement).value), p[1]])} aria-label="Column in this table">
            {#each titles as t, c (c)}<option value={c}>{t}</option>{/each}
          </select>
          {#if fromList}
            is on the list{#if pairs.length > 1} (part {i + 1}){/if}
          {:else}
            equals
            <select value={p[1]} onchange={(e) => (pairs[i] = [p[0], Number((e.currentTarget as HTMLSelectElement).value)])} aria-label="Column in the other table">
              {#each otherTitles as t, c (c)}<option value={c}>{t}</option>{/each}
            </select>
          {/if}
          {#if pairs.length > 1}<button type="button" onclick={() => (pairs = pairs.filter((_, k) => k !== i))} aria-label="Remove this key">✕</button>{/if}
        </div>
      {/each}
      {#if pairs.length < 8 && !range}<button type="button" onclick={() => (pairs = [...pairs, [0, 0]])}>And another column…</button>{/if}
      <label class="row">Compare keys
        <select bind:value={compare} onchange={() => { if (compare === "below" || compare === "above") pairs = pairs.slice(0, 1); }}>
          {#each COMPARES as [v, label] (v)}<option value={v}>{label}</option>{/each}
        </select>
      </label>
      {#if compare === "exact"}
        <div class="grid2">
          <label class="row"><input type="checkbox" bind:checked={matchCase} /> Match case</label>
          <label class="row"><input type="checkbox" bind:checked={trim} /> Ignore spaces at the start and end</label>
        </div>
      {:else if compare === "loose"}
        <label class="row"><input type="checkbox" bind:checked={matchCase} /> Match case</label>
      {/if}
    </fieldset>
    <fieldset>
      <legend>What to do</legend>
      <label class="row">Rows
        <select bind:value={keepRows}>
          {#each KEEPS as [v, label] (v)}<option value={v}>{label}</option>{/each}
        </select>
      </label>
      {#if keepRows !== "unmatched"}
        <label class="row">Values
          <select bind:value={write}>
            {#if fromList}
              <option value="new">Add a column counting how often each key is on the list</option>
              <option value="nothing">Don't add anything (only keep or drop rows)</option>
            {:else}
              {#each WRITES as [v, label] (v)}<option value={v}>{label}</option>{/each}
            {/if}
          </select>
        </label>
      {/if}
      {#if writes && !fromList}
        <label class="row">When the other table has a key more than once
          <select bind:value={repeats}>
            {#each REPEATS as [v, label] (v)}<option value={v}>{label}</option>{/each}
          </select>
        </label>
        {#if repeats === "joined"}
          <label class="row">Separate them with <input class="short" bind:value={separator} aria-label="Separator" /></label>
        {/if}
      {/if}
    </fieldset>
    {#if writes && !fromList && repeats !== "count"}
      <fieldset>
        <legend>{update ? "Columns to bring in, and where they go" : "Columns to bring in"}</legend>
        <AllNone count={otherTitles.length} picked={bringList} onChange={(v) => (bring = v)} skip={(c) => keyCols.includes(c)} />
        <div class="list" class:single={update}>
          {#each otherTitles as t, c (c)}
            <div class="row">
              <label><input type="checkbox" checked={bringList.includes(c)} onchange={() => toggle(c)} /> {t}</label>
              {#if update && bringList.includes(c)}
                into
                <select value={targets[bringList.indexOf(c)] ?? -1} onchange={(e) => (into = { ...into, [c]: Number((e.currentTarget as HTMLSelectElement).value) })} aria-label={`Column of this table for ${t}`}>
                  <option value={-1} disabled>choose…</option>
                  {#each titles as m, k (k)}{#if !pairs.some((p) => p[0] === k)}<option value={k}>{m}</option>{/if}{/each}
                </select>
              {/if}
            </div>
          {/each}
        </div>
      </fieldset>
    {:else if writes && update && repeats === "count"}
      <label class="row">Write the count into
        <select value={targets[0] ?? -1} onchange={(e) => (into = { ...into, [-1]: Number((e.currentTarget as HTMLSelectElement).value) })}>
          <option value={-1} disabled>choose…</option>
          {#each titles as m, k (k)}{#if !pairs.some((p) => p[0] === k)}<option value={k}>{m}</option>{/if}{/each}
        </select>
      </label>
    {/if}
    {#if writes && write === "new"}
      <div class="stack">
        <label class="row">Put new columns
          <select bind:value={place}>
            <option value="end">at the end</option>
            <option value="after_key">right after the key</option>
          </select>
        </label>
        <label class="row">Name them starting with <input class="short" bind:value={prefix} placeholder="(nothing)" aria-label="Start of new column names" /></label>
        <label class="row">Rows with no match get <input class="short" bind:value={notFound} placeholder="(blank)" aria-label="Text for rows with no match" /></label>
      </div>
    {/if}
    {#if !fromList && !range && writes}
      <label class="row"><input type="checkbox" bind:checked={addUnmatched} /> Also add rows only the other table has (at the end)</label>
    {/if}
    <div class="preview-head">
      <strong>Preview</strong>
      {#if previewing}<span class="note">Working it out…</span>{/if}
    </div>
    {#if preview}
      <p class="good">{joinCounts(preview.report, { repeats, keep_rows: keepRows, key_match: compare === "below" || compare === "above" ? compare : "exact" })}</p>
      {#each warnings as w (w)}<p class="problem">{w}</p>{/each}
      <div class="table-preview" aria-label="The first rows after the join">
        <table>
          <tbody>
            <tr class="head"><td></td>{#each preview.titles as t, c (c)}<td class:new={preview.columns.includes(c)}>{t}</td>{/each}</tr>
            {#each preview.rows.slice(0, 50) as r, i (i)}
              <tr class={r.kind}>
                <td class="kind">{r.row ?? ""} {KIND_LABEL[r.kind] ?? r.kind}</td>
                {#each r.values as v, c (c)}<td class:changed={r.changed.includes(c)}>{v}</td>{/each}
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <p class="note">Showing the first rows; the counts are for the whole table. Values are written as plain text. With a filter on, only shown rows change.{#if repeats === "every" && addUnmatched} Rows only the other table has are added grouped by key, in the order each key first appears.{/if}{#if range && preview.report.matched + preview.report.unmatched > 0} Dates are read month first (03/04/2024 is March 4); keys are compared as numbers or as dates, whichever most of the other table's keys are.{/if}</p>
    {:else if previewProblem}
      <p class="note">{previewProblem}</p>
    {/if}
  {:else}
    <fieldset>
      <legend>Line up columns</legend>
      <label class="row"><input type="radio" name="by" value={true} bind:group={byName} /> By column name</label>
      <label class="row"><input type="radio" name="by" value={false} bind:group={byName} /> By position (first with first, ...)</label>
      {#if byName}
        <label class="row"><input type="checkbox" bind:checked={addColumns} /> Add columns for names this table does not have</label>
      {/if}
    </fieldset>
    <p class="note">The other table's rows are added at the end.</p>
  {/if}
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#if !fromList || kind === "append"}<OtherHidden table={list.find((o) => o.id === other)} matching={kind === "join" && keepRows !== "all"} />{/if}
  {#snippet buttons()}
    <button type="button" class="primary" onclick={run}>{kind === "join" ? "Join" : "Append"}</button>
    <button type="button" onclick={onClose}>Cancel</button>
  {/snippet}
</FormShell>

<style>
  .short {
    width: 9em;
  }
  .list.single {
    grid-template-columns: 1fr;
  }
  .stack {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  textarea {
    width: 100%;
    font-family: ui-monospace, monospace;
  }
  .preview-head {
    display: flex;
    gap: 8px;
    align-items: baseline;
    margin-top: 8px;
  }
  .table-preview td.new {
    color: var(--accent);
  }
  .table-preview td.changed {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .table-preview tr.removed td {
    text-decoration: line-through;
    color: var(--muted);
  }
  .table-preview tr.extra td,
  .table-preview tr.added td {
    font-style: italic;
  }
  .table-preview td.kind {
    color: var(--muted);
    white-space: nowrap;
  }
</style>
