<script lang="ts">
  // Combine with a column map: each output column, and which column of each picked file feeds it,
  // changed by hand before anything is combined. Suggestions (two titles that are probably one
  // column) are only applied when the person merges them. The map can be saved (titles only) and
  // used again next month. The files' folders never reach this window, only their names.
  import { backend, errorText, type CombineExtras, type CombinePlan, type CombineStart, type CombineSuggestion, type DocInfo } from "./backend";
  import FormShell from "./FormShell.svelte";
  import { addAsColumn, landed, leftOut, mergePlan, samePlan, sameTitles, setSource, sourceOf } from "./combineMap";

  interface Props {
    start: CombineStart;
    /** Add a column with each row's file name. */
    source: boolean;
    onDone: (d: DocInfo) => void;
    onClose: () => void;
  }
  let { start, source, onDone, onClose }: Props = $props();

  // svelte-ignore state_referenced_locally
  let session = $state<CombineStart>(start);
  // svelte-ignore state_referenced_locally
  let plan = $state<CombinePlan>(start.plan);
  // svelte-ignore state_referenced_locally
  let suggestions = $state<CombineSuggestion[]>(start.suggestions);
  // svelte-ignore state_referenced_locally
  let withName = $state(source);
  let extras = $state<CombineExtras>("add");
  let shown = $state(0);
  let problem = $state("");
  let note = $state("");
  let busy = $state(false);
  const files = $derived(session.files);
  const auto = $derived(session.plan);

  async function use(p: CombinePlan): Promise<void> {
    problem = "";
    try {
      const r = await backend().combineCheck(session.token, p);
      plan = r.plan;
      suggestions = r.suggestions;
    } catch (e) {
      problem = errorText(e);
    }
  }

  function choose(f: number, out: number, value: string): void {
    void use(setSource(plan, f, out, value === "" ? null : Number(value)));
  }

  function merge(s: CombineSuggestion): void {
    const p = mergePlan(plan, s.keep, s.merge);
    if (p) void use(p);
    else problem = "A file has both columns, so they cannot be merged.";
  }

  function mergeAllTidied(): void {
    let p = plan;
    // Merge from the last pair so earlier numbers stay right.
    for (const s of [...suggestions].filter((s) => s.why.kind === "tidied").sort((a, b) => b.merge - a.merge)) {
      p = mergePlan(p, s.keep, s.merge) ?? p;
    }
    void use(p);
  }

  async function save(): Promise<void> {
    problem = "";
    try {
      if (await backend().combineSaveMapping(session.token, plan, extras)) note = "Column map saved (column titles only).";
    } catch (e) {
      problem = errorText(e);
    }
  }

  async function load(): Promise<void> {
    problem = "";
    try {
      const fit = await backend().combineLoadMapping(session.token);
      if (!fit) return;
      plan = fit.plan;
      suggestions = fit.suggestions;
      withName = fit.file_name_column;
      extras = fit.extras_mode;
      const said: string[] = [];
      if (fit.missing.length) said.push(`${fit.missing.length} column${fit.missing.length === 1 ? "" : "s"} of the map ${fit.missing.length === 1 ? "is" : "are"} not in some files (blank there)`);
      if (fit.extras.length) said.push(`${fit.extras.length} column${fit.extras.length === 1 ? "" : "s"} of the files ${fit.extras.length === 1 ? "is" : "are"} not in the map (${fit.extras_mode === "add" ? "added at the end" : "left out"})`);
      for (const p of fit.problems) said.push(`${files[p.file]?.name ?? "a file"}: ${p.what}; choose by hand`);
      note = said.length ? `Map used. ${said.join(". ")}.` : "Map used: every column fits.";
    } catch (e) {
      problem = errorText(e);
    }
  }

  async function run(): Promise<void> {
    busy = true;
    problem = "";
    try {
      const d = await backend().combineRun(session.token, plan, withName);
      onDone(d);
    } catch (e) {
      const kind = (e as { kind?: string }).kind;
      if (kind === "combine_stale") {
        // A file changed after it was picked: read the heads again. The map made by hand is kept when
        // every file's titles are still the same (it still means what it meant); otherwise it is made again.
        const said = errorText(e);
        try {
          const before = session.files;
          session = await backend().combineRefresh(session.token);
          let kept = false;
          if (sameTitles(before, session.files)) {
            try {
              const r = await backend().combineCheck(session.token, plan);
              plan = r.plan;
              suggestions = r.suggestions;
              kept = true;
            } catch {
              kept = false;
            }
          }
          if (!kept) {
            plan = session.plan;
            suggestions = session.suggestions;
          }
          problem = kept
            ? `${said} The files were read again and their titles are the same, so your map is kept: check it, then combine.`
            : `${said} The files were read again and their titles changed, so the map was made again: check it, then combine.`;
        } catch (e2) {
          problem = errorText(e2);
        }
      } else problem = errorText(e);
    } finally {
      busy = false;
    }
  }

  function close(): void {
    void backend().combineCancel(session.token);
    onClose();
  }

  const why = (s: CombineSuggestion): string => (s.why.kind === "tidied" ? "the same after tidying (case, spaces, _ - .)" : "a similar title in about the same place");
  const samples = (v: string[]): string => (v.length ? v.map((x) => `“${x}”`).join(", ") : "no values in the first rows");
  const preview = $derived(landed(plan, files, shown));
</script>

<FormShell title="Combine files: line up the columns" wide onClose={close} onEnter={() => void run()}>
  <p class="note">Each row below is a column of the combined table; each file's cell says which of its columns goes there. Nothing changes in the files. {files.length} files, about {files.reduce((n, f) => n + Math.max(0, f.rows_est - 1), 0).toLocaleString()} rows (not counting their title rows).</p>
  <div class="grid" role="region" aria-label="Column map">
    <table>
      <thead>
        <tr>
          <th scope="col">Combined column</th>
          {#each files as f, i (i)}<th scope="col">{f.name}<span class="sub"> ({f.cols} columns, {f.whole ? "" : "about "}{Math.max(0, f.rows_est - 1).toLocaleString()} rows)</span></th>{/each}
        </tr>
      </thead>
      <tbody>
        {#each plan.titles as t, out (out)}
          <tr>
            <th scope="row">{t}</th>
            {#each files as f, fi (fi)}
              {@const c = sourceOf(plan, fi, out)}
              {@const autoC = sourceOf(auto, fi, auto.titles.indexOf(t))}
              <td>
                <select aria-label={`${t}: column of ${f.name}`} value={c === null ? "" : String(c)} onchange={(e) => choose(fi, out, e.currentTarget.value)}>
                  <option value="">(none: blank)</option>
                  {#each f.titles as ft, k (k)}<option value={String(k)}>{ft || `Column ${k + 1}`}</option>{/each}
                </select>
                <span class="how">{c === null ? "missing" : autoC === c && f.titles[c]?.trim().toLowerCase() === t.trim().toLowerCase() ? "same title" : "chosen"}</span>
              </td>
            {/each}
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
  {#each files as f, fi (fi)}
    {@const out = leftOut(plan, fi)}
    {#if out.length}
      <p class="sub">{f.name}: {out.map((c) => f.titles[c] || `Column ${c + 1}`).join(", ")} {out.length === 1 ? "goes" : "go"} nowhere (left out).
        {#each out as c (c)}<button type="button" class="small" onclick={() => void use(addAsColumn(plan, fi, c, f.titles[c] ?? ""))}>Add {f.titles[c] || `Column ${c + 1}`}</button>{/each}
      </p>
    {/if}
    {#if f.cols !== files[0]?.cols}<p class="sub">{f.name} has {f.cols} columns ({files[0]?.name} has {files[0]?.cols}).</p>{/if}
  {/each}
  {#if suggestions.length}
    <fieldset>
      <legend>Suggestions (nothing is merged until you choose)</legend>
      {#each suggestions as s, i (i)}
        <div class="row">
          <span><strong>{plan.titles[s.keep]}</strong> and <strong>{plan.titles[s.merge]}</strong> may be one column: {why(s)}. Values: {samples(s.keep_samples)} / {samples(s.merge_samples)}.</span>
          <button type="button" onclick={() => merge(s)}>Merge</button>
        </div>
      {/each}
      {#if suggestions.some((s) => s.why.kind === "tidied")}<button type="button" onclick={mergeAllTidied}>Merge all that are the same after tidying</button>{/if}
    </fieldset>
  {/if}
  <fieldset>
    <legend>Preview: the first rows of</legend>
    <div class="row" role="tablist">
      {#each files as f, i (i)}<button type="button" role="tab" aria-selected={shown === i} class:on={shown === i} onclick={() => (shown = i)}>{f.name}</button>{/each}
    </div>
    <div class="grid small-grid">
      <table aria-label={`First rows of ${files[shown]?.name ?? ""} as they will land`}>
        <thead><tr>{#each plan.titles as t, i (i)}<th scope="col">{t}</th>{/each}</tr></thead>
        <tbody>{#each preview as r, i (i)}<tr>{#each r as v, k (k)}<td>{v}</td>{/each}</tr>{/each}</tbody>
      </table>
    </div>
  </fieldset>
  <label class="row"><input type="checkbox" bind:checked={withName} /> Add a column with each row's file name</label>
  <label class="row">A file's column the saved map does not name:
    <select bind:value={extras} aria-label="Columns a saved map does not name"><option value="add">add it at the end</option><option value="leave_out">leave it out</option></select>
  </label>
  {#if note}<p class="sub" role="status">{note}</p>{/if}
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" class="primary" onclick={() => void run()} disabled={busy}>{busy ? "Combining…" : "Combine"}</button>
    <button type="button" onclick={() => void use(auto)} disabled={samePlan(plan, auto)}>Reset to automatic</button>
    <button type="button" onclick={() => void save()}>Save map…</button>
    <button type="button" onclick={() => void load()}>Use a saved map…</button>
    <button type="button" onclick={close}>Cancel</button>
  {/snippet}
</FormShell>

<style>
  .grid {
    max-height: 40vh;
    overflow: auto;
    border: 1px solid var(--border, #ccc);
  }
  .small-grid {
    max-height: 22vh;
  }
  table {
    border-collapse: collapse;
    font-size: 0.92em;
  }
  th,
  td {
    padding: 2px 6px;
    border-bottom: 1px solid var(--border, #ddd);
    text-align: left;
    white-space: nowrap;
  }
  .how,
  .sub {
    color: var(--muted);
    font-size: 0.9em;
    margin-left: 4px;
  }
  .row {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
  }
  .small {
    font-size: 0.85em;
  }
  .on {
    font-weight: 600;
  }
</style>
