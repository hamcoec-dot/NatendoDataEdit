<script lang="ts">
  // Data checks: rules a column's cells (or a whole row) must meet. Run
  // lists the problems (click one to go there); problems can be marked on
  // the grid and the table can show only rows with problems. Checks are
  // kept with views and can be saved to a file the command line reads.
  import { untrack } from "svelte";
  import { backend, errorText, type Check, type CheckKind, type CheckReport, type CheckSpec, type RuleSuggestion } from "./backend";
  import FormShell from "./FormShell.svelte";
  import { CHECK_KINDS, CHECK_PATTERNS, describeCheck, localDays, newCheck } from "./m8";

  interface Props {
    doc: number;
    titles: string[];
    col: number;
    header: boolean;
    checks: Check[];
    marked: boolean;
    /** Keep these checks on the tab (and mark problems when `mark`). */
    onApply: (checks: Check[], mark: boolean) => void;
    onGoto: (row: number, col: number | null) => void;
    onShowProblems: (spec: CheckSpec) => void;
    onClose: () => void;
    /** Set when the rules came from a file beside the table (its name shows, and the file can be removed). */
    rulesFile?: string;
  }
  let { doc, titles, col, header, checks, marked, onApply, onGoto, onShowProblems, onClose, rulesFile = "" }: Props = $props();

  let list = $state<Check[]>(untrack(() => structuredClone($state.snapshot(checks))));
  let mark = $state(untrack(() => marked || checks.length === 0));
  let report = $state<CheckReport | null>(null);
  let problem = $state("");
  let running = $state(false);
  let kept = $state("");
  let suggestions = $state<(RuleSuggestion & { on: boolean })[] | null>(null);
  let suggesting = $state(false);

  /** Ready-made rules for the column this dialog was opened on. */
  const PRESETS: { id: string; label: string; make: () => Check | null }[] = [
    { id: "digits5", label: "5 digits (like a ZIP code)", make: () => ({ col, name: "5 digits", kind: { type: "pattern", pattern: { text: "[0-9]{5}", kind: "regex", match_case: false, whole_cell: true } } }) },
    { id: "trim", label: "No spaces at the start or end", make: () => (titles[col] ? { col: null, name: "No spaces at the start or end", kind: { type: "expression", expr: `TRIM([${titles[col]}]) = [${titles[col]}]` } } : null) },
    { id: "iso", label: "A real date written YYYY-MM-DD", make: () => ({ col, kind: { type: "date_iso" } }) },
    { id: "whole", label: "A whole number", make: () => ({ col, kind: { type: "number", min: null, max: null, whole: true } }) },
    { id: "list", label: "One of a list of values", make: () => ({ col, kind: { type: "list", values: [], match_case: false } }) },
    { id: "unique", label: "No repeats", make: () => ({ col, kind: { type: "unique", match_case: false } }) },
    { id: "required", label: "Must not be blank", make: () => ({ col, kind: { type: "required" } }) },
  ];
  function addPreset(id: string): void {
    const c = PRESETS.find((p) => p.id === id)?.make();
    if (!c) {
      problem = "That rule needs the column to have a title.";
      return;
    }
    if (list.length < MAX) list = [...list, c];
  }

  async function suggest(): Promise<void> {
    suggesting = true;
    problem = "";
    try {
      const s = await backend().suggestRules(doc, col, header);
      suggestions = s.map((x) => ({ ...x, on: true }));
      if (!s.length) problem = "Nothing to suggest: this column's values do not share one clear form.";
    } catch (e) {
      problem = errorText(e);
    } finally {
      suggesting = false;
    }
  }
  function addSuggested(): void {
    const on = (suggestions ?? []).filter((s) => s.on).map((s) => s.check);
    list = [...list, ...on].slice(0, MAX);
    suggestions = null;
    report = null;
  }

  async function keep(): Promise<void> {
    try {
      const name = await backend().checksSidecarKeep(doc, spec().checks, header);
      kept = list.length ? `Kept in ${name}.` : "No rules, so the file beside the table was removed.";
      problem = "";
    } catch (e) {
      problem = errorText(e);
    }
  }
  function forget(): void {
    list = [];
    report = null;
    suggestions = null;
    onApply([], false);
    kept = "Rules forgotten. A rules file beside the table is untouched: Keep with no rules removes it.";
  }

  const MAX = 200;
  function add(): void {
    if (list.length < MAX) list = [...list, newCheck("required", col)];
  }
  function remove(i: number): void {
    list = list.filter((_, k) => k !== i);
    report = null;
  }
  function setKind(i: number, type: CheckKind["type"]): void {
    const c = list[i]!;
    list[i] = newCheck(type, c.col ?? col);
    report = null;
  }
  const numOrNull = (s: string): number | null => (s.trim() === "" || Number.isNaN(Number(s)) ? null : Number(s));
  const strOrNull = (s: string): string | null => (s.trim() === "" ? null : s.trim());

  function spec(): CheckSpec {
    return { checks: $state.snapshot(list) as Check[], keep_first: header, read: { decimal_comma: false, date_order: "mdy" }, now: localDays() };
  }

  async function run(): Promise<void> {
    if (!list.length || running) return;
    running = true;
    problem = "";
    try {
      report = await backend().runChecks(doc, spec());
      onApply(spec().checks, mark);
    } catch (e) {
      report = null;
      problem = errorText(e);
    } finally {
      running = false;
    }
  }

  async function save(): Promise<void> {
    try {
      const name = await backend().checksSave(spec().checks);
      if (name) problem = "";
    } catch (e) {
      problem = errorText(e);
    }
  }
  async function open(): Promise<void> {
    try {
      const c = await backend().checksOpen();
      if (c) {
        list = c.filter((x) => x.col === null || x.col < titles.length);
        report = null;
      }
    } catch (e) {
      problem = errorText(e);
    }
  }
  const title = (c: number | null): string => (c === null ? "row" : titles[c] || `Column ${c + 1}`);
</script>

<FormShell title="Data checks" wide {onClose} onEnter={() => void run()}>
  <div class="checks">
    {#each list as c, i (i)}
      <div class="check">
        {#if c.kind.type !== "expression"}
          <select aria-label="Column" bind:value={c.col}>
            {#each titles as t, k (k)}<option value={k}>{t || `Column ${k + 1}`}</option>{/each}
          </select>
        {/if}
        <input type="text" class="name" aria-label="Rule name" placeholder="name (optional)" maxlength="100" value={c.name ?? ""} oninput={(e) => (c.name = e.currentTarget.value.trim() ? e.currentTarget.value : null)} />
        <select aria-label="Check" value={c.kind.type} onchange={(e) => setKind(i, (e.currentTarget as HTMLSelectElement).value as CheckKind["type"])}>
          {#each CHECK_KINDS as k (k.type)}<option value={k.type}>{k.label}</option>{/each}
        </select>
        {#if c.kind.type === "number"}
          {@const k = c.kind}
          <input type="text" aria-label="Smallest" placeholder="smallest" value={k.min ?? ""} oninput={(e) => (k.min = numOrNull(e.currentTarget.value))} />
          <input type="text" aria-label="Largest" placeholder="largest" value={k.max ?? ""} oninput={(e) => (k.max = numOrNull(e.currentTarget.value))} />
          <label class="row"><input type="checkbox" bind:checked={k.whole} /> whole</label>
        {:else if c.kind.type === "date"}
          {@const k = c.kind}
          <input type="text" aria-label="Earliest" placeholder="earliest (2024-01-01)" value={k.min ?? ""} oninput={(e) => (k.min = strOrNull(e.currentTarget.value))} />
          <input type="text" aria-label="Latest" placeholder="latest" value={k.max ?? ""} oninput={(e) => (k.max = strOrNull(e.currentTarget.value))} />
        {:else if c.kind.type === "length"}
          {@const k = c.kind}
          <input type="text" aria-label="Shortest" placeholder="shortest" value={k.min ?? ""} oninput={(e) => (k.min = numOrNull(e.currentTarget.value))} />
          <input type="text" aria-label="Longest" placeholder="longest" value={k.max ?? ""} oninput={(e) => (k.max = numOrNull(e.currentTarget.value))} />
        {:else if c.kind.type === "list"}
          {@const k = c.kind}
          <input class="grow" type="text" aria-label="Allowed values" placeholder="allowed values, separated by commas" value={k.values.join(", ")}
            oninput={(e) => (k.values = e.currentTarget.value.split(",").map((v) => v.trim()).filter(Boolean))} />
          <label class="row"><input type="checkbox" bind:checked={k.match_case} /> case</label>
        {:else if c.kind.type === "pattern"}
          {@const k = c.kind}
          <select aria-label="Ready-made pattern" value="" onchange={(e) => { const v = (e.currentTarget as HTMLSelectElement).value; if (v) k.pattern.text = v; (e.currentTarget as HTMLSelectElement).value = ""; }}>
            <option value="">Ready-made…</option>
            {#each CHECK_PATTERNS as p (p.name)}<option value={p.text}>{p.name}</option>{/each}
          </select>
          <input class="grow mono" type="text" aria-label="Pattern" placeholder="regular expression for the whole cell" bind:value={k.pattern.text} spellcheck="false" />
        {:else if c.kind.type === "unique"}
          {@const k = c.kind}
          <label class="row"><input type="checkbox" bind:checked={k.match_case} /> case matters</label>
        {:else if c.kind.type === "expression"}
          {@const k = c.kind}
          <input class="grow mono" type="text" aria-label="Rule" placeholder="true when the row is right, like [Start] <= [End]" bind:value={k.expr} spellcheck="false" />
        {/if}
        {#if report}<span class="count" class:bad={(report.per_check[i] ?? 0) > 0}>{(report.per_check[i] ?? 0).toLocaleString()}</span>{/if}
        <button type="button" class="x" aria-label="Remove this check" onclick={() => remove(i)}>×</button>
      </div>
    {/each}
    <div class="check">
      <button type="button" onclick={add} disabled={list.length >= MAX}>Add a check</button>
      <select aria-label="Ready-made rule" value="" onchange={(e) => { const v = (e.currentTarget as HTMLSelectElement).value; if (v) addPreset(v); (e.currentTarget as HTMLSelectElement).value = ""; }}>
        <option value="">Ready-made rule for “{titles[col] || `Column ${col + 1}`}”…</option>
        {#each PRESETS as p (p.id)}<option value={p.id}>{p.label}</option>{/each}
      </select>
      <button type="button" onclick={() => void suggest()} disabled={suggesting}>{suggesting ? "Looking…" : "Suggest rules from this data"}</button>
    </div>
  </div>
  {#if suggestions?.length}
    <div class="suggest" aria-label="Suggested rules">
      {#each suggestions as s, i (i)}
        <label class="row"><input type="checkbox" bind:checked={s.on} /> <strong>{s.check.name ?? describeCheck(s.check, titles)}</strong> <span class="sub">{s.why}</span></label>
      {/each}
      <div class="check"><button type="button" onclick={addSuggested}>Add the ticked rules</button><button type="button" onclick={() => (suggestions = null)}>Not now</button></div>
    </div>
  {/if}
  {#if rulesFile}<p class="note">These rules were read from {rulesFile} beside the table.</p>{/if}
  <p class="note">Blank cells pass every check except "Must not be blank". Checks are kept with saved views.</p>
  <label class="row"><input type="checkbox" bind:checked={mark} /> Mark problem cells on the grid</label>
  {#if report}
    <p class={report.problems ? "problem" : "good"}>
      {report.checked.toLocaleString()} rows checked: {report.problems ? `${report.rows.toLocaleString()} with problems (${report.problems.toLocaleString()} in all)` : "no problems"}.
    </p>
    {#if report.listed.length}
      <div class="table-preview" aria-label="Problems">
        <table>
          <tbody>
            {#each report.listed.slice(0, 500) as p, k (k)}
              <tr class="hit" onclick={() => onGoto(p.row, p.col)}>
                <td class="num">{p.row + 1}</td><td>{title(p.col)}</td><td>{p.value}</td><td class="sub">{describeCheck(list[p.check] ?? newCheck("required", 0), titles)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <p class="note">Click a problem to go to it.{report.listed.length < report.problems ? ` Listing the first ${report.listed.length.toLocaleString()}.` : ""}</p>
    {/if}
  {/if}
  {#if kept}<p class="good">{kept}</p>{/if}
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" class="primary" onclick={() => void run()} disabled={!list.length || running}>{running ? "Checking…" : "Run checks"}</button>
    <button type="button" onclick={() => onShowProblems(spec())} disabled={!list.length}>Show only rows with problems</button>
    <button type="button" onclick={() => { onApply(spec().checks, mark); onClose(); }}>Keep and close</button>
    <button type="button" onclick={() => void open()}>Open…</button>
    <button type="button" onclick={() => void save()} disabled={!list.length}>Save…</button>
    <button type="button" title="Writes the rules to a small file beside this file (never the table's cells: only the names, patterns and lists you typed). Not for private or encrypted files." onclick={() => void keep()}>Keep these rules with the file</button>
    <button type="button" onclick={forget} disabled={!list.length}>Forget these rules</button>
    <button type="button" onclick={onClose}>Cancel</button>
  {/snippet}
</FormShell>

<style>
  .checks {
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 40vh;
    overflow: auto;
  }
  .check {
    display: flex;
    gap: 6px;
    align-items: center;
    flex-wrap: wrap;
  }
  .check input[type="text"] {
    width: 9em;
  }
  .check input.name {
    width: 11em;
  }
  .suggest {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 6px;
    border: 1px solid var(--border);
    border-radius: 4px;
  }
  .sub {
    color: var(--muted);
  }
  .check input.grow {
    flex: 1;
    min-width: 14em;
  }
  .mono {
    font-family: ui-monospace, Consolas, monospace;
  }
  .count {
    min-width: 4em;
    text-align: right;
    color: var(--muted);
  }
  .count.bad {
    color: var(--danger, #c0392b);
    font-weight: 600;
  }
  .x {
    width: 2em;
  }
  tr.hit {
    cursor: pointer;
  }
  tr.hit:hover td {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  td.num {
    color: var(--muted);
    text-align: right;
  }
</style>
