<script lang="ts">
  // Check fields (Milestone 15): a pattern box next to each column. Check
  // now counts the cells that do not fit, per column, and marks them; then
  // show the rows that fail (any column or one), the rows that pass, put
  // failing rows first, or add a "Check result" column. The patterns are
  // the Data checks' pattern checks, so they are kept with views and the
  // other kinds of checks are left as they are.
  import { untrack } from "svelte";
  import { backend, errorText, type Check, type CheckReport, type CheckSpec } from "./backend";
  import FormShell from "./FormShell.svelte";
  import { CHECK_PATTERNS, localDays } from "./m8";
  import { patternChecks, withPatterns } from "./fieldcheck";

  interface Props {
    doc: number;
    titles: string[];
    header: boolean;
    checks: Check[];
    /** Keep these checks on the tab and mark failing cells. */
    onApply: (checks: Check[]) => void;
    /** Show failing (or passing) rows; `only`: one column's checks. */
    onShow: (spec: CheckSpec, passing: boolean) => void;
    onSortFailing: (spec: CheckSpec) => void;
    onResultColumn: (spec: CheckSpec) => void;
    onClose: () => void;
  }
  let { doc, titles, header, checks, onApply, onShow, onSortFailing, onResultColumn, onClose }: Props = $props();

  const MAX_COLS = 500;
  const shown = $derived(titles.slice(0, MAX_COLS));
  let boxes = $state<string[]>(untrack(() => patternChecks(checks, titles.length)));
  let whole = $state(true);
  let report = $state<CheckReport | null>(null);
  let problem = $state("");
  let running = $state(false);
  /** Which column the show/sort buttons use: -1 any column. */
  let which = $state(-1);

  const filled = $derived(boxes.some((b) => b.trim() !== ""));

  function all(): Check[] {
    return withPatterns($state.snapshot(checks) as Check[], $state.snapshot(boxes) as string[], whole);
  }
  function spec(only = which): CheckSpec {
    let list = all();
    if (only >= 0) list = list.filter((c) => c.col === only && c.kind.type === "pattern");
    return { checks: list, keep_first: header, read: { decimal_comma: false, date_order: "mdy" }, now: localDays() };
  }
  /** Failing cells per column, from the last check (pattern checks only). */
  const perCol = $derived.by(() => {
    const out = new Map<number, number>();
    if (!report) return out;
    const list = all();
    list.forEach((c, i) => {
      if (c.kind.type === "pattern" && c.col !== null && c.col !== undefined) out.set(c.col, (out.get(c.col) ?? 0) + (report!.per_check[i] ?? 0));
    });
    return out;
  });

  async function check(): Promise<void> {
    if (!filled || running) return;
    running = true;
    problem = "";
    try {
      const s = spec(-1);
      report = await backend().runChecks(doc, s);
      onApply(s.checks);
    } catch (e) {
      report = null;
      problem = errorText(e);
    } finally {
      running = false;
    }
  }
  const plural = (n: number, one: string, many = `${one}s`) => `${n.toLocaleString()} ${n === 1 ? one : many}`;
  const usable = $derived(filled && (which < 0 || (boxes[which] ?? "").trim() !== ""));
</script>

<FormShell title="Check fields" wide {onClose}>
  <p class="note">Type a pattern (a regular expression) next to any column. Leave a box empty to skip that column. <strong>Check now</strong> counts the cells that do not fit and marks them in the table.</p>
  <label class="row"><input type="checkbox" bind:checked={whole} /> The whole cell must fit the pattern (not just part of it)</label>
  <div class="cols" role="table" aria-label="Patterns per column">
    {#each shown as t, c (c)}
      <div class="line" role="row">
        <span class="title" title={t}>{t}</span>
        <input type="text" aria-label={`Pattern for ${t}`} bind:value={boxes[c]} oninput={() => (report = null)} placeholder="no check" spellcheck="false" maxlength="10000" />
        <select aria-label={`Common patterns for ${t}`} onchange={(e) => { const v = (e.currentTarget as HTMLSelectElement).value; if (v) { boxes[c] = v; report = null; } (e.currentTarget as HTMLSelectElement).value = ""; }}>
          <option value="">Common…</option>
          {#each CHECK_PATTERNS as p (p.name)}<option value={p.text}>{p.name}</option>{/each}
        </select>
        <span class="count">
          {#if report && (boxes[c] ?? "").trim()}
            {#if perCol.get(c)}<span class="bad">{plural(perCol.get(c)!, "fails", "fail")}</span>{:else}<span class="ok">all fit</span>{/if}
          {/if}
        </span>
      </div>
    {/each}
  </div>
  {#if titles.length > MAX_COLS}<p class="note">The first {MAX_COLS} columns are listed; use Data checks for the others.</p>{/if}
  {#if report}
    <p>{report.rows ? `${plural(report.rows, "row")} of ${report.checked.toLocaleString()} fail${report.rows === 1 ? "s" : ""} at least one check.` : `Every one of the ${report.checked.toLocaleString()} rows passes.`}</p>
  {/if}
  {#if problem}<p class="warn">{problem}</p>{/if}
  <label class="row">Use
    <select bind:value={which}>
      <option value={-1}>any column</option>
      {#each shown as t, c (c)}{#if (boxes[c] ?? "").trim()}<option value={c}>{t}</option>{/if}{/each}
    </select>
    for the buttons below.
  </label>
  <div class="actions">
    <button type="button" disabled={!usable} onclick={() => onShow(spec(), false)}>Show rows that fail</button>
    <button type="button" disabled={!usable} onclick={() => onShow(spec(), true)}>Show rows that pass</button>
    <button type="button" disabled={!usable} onclick={() => onSortFailing(spec())}>Put failing rows first</button>
    <button type="button" disabled={!usable} onclick={() => onResultColumn(spec())}>Add a "Check result" column</button>
  </div>
  {#snippet buttons()}
    <button type="button" class="primary" data-enter disabled={!filled || running} onclick={() => void check()}>{running ? "Checking…" : "Check now"}</button>
    <button type="button" onclick={onClose}>Close</button>
  {/snippet}
</FormShell>

<style>
  .cols {
    max-height: 320px;
    overflow: auto;
    border: 1px solid var(--border, #8884);
    border-radius: 4px;
    padding: 4px 8px;
  }
  .line {
    display: grid;
    grid-template-columns: 12em 1fr 9em 7em;
    gap: 8px;
    align-items: center;
    padding: 2px 0;
  }
  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .line input {
    font-family: var(--mono, monospace);
  }
  .bad {
    color: var(--warn, #b45309);
  }
  .ok {
    opacity: 0.7;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 6px;
  }
  .warn {
    color: var(--warn, #b45309);
  }
</style>
