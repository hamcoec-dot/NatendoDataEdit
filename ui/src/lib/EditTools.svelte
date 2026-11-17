<script lang="ts">
  // Everyday editing tools (Milestone 12): group figures in a new column,
  // choosing and ordering columns, pulling a piece of text into its own
  // column, and building a column without typing a formula (a template
  // like "{First} {Last}", or if-then rules). Each is one undo step.
  import AllNone from "./AllNone.svelte";
  import { untrack } from "svelte";
  import { backend, errorText, type ComputePreview, type ComputeSpec, type DataOp, type ExtractWhat, type GroupFunc } from "./backend";
  import { EXTRACTS, GROUP_FUNCS, RULE_OPS, rulesExpr, templateExpr, type Rule } from "./editing";
  import FormShell from "./FormShell.svelte";
  import { localDays } from "./m8";

  export type ToolKind = "group" | "arrange" | "extract" | "build";

  interface Props {
    kind: ToolKind;
    doc: number;
    /** Column titles (may be cut short). */
    titles: string[];
    /** Every column in the document. */
    cols: number;
    col: number;
    header: boolean;
    onRun: (op: DataOp, header: boolean) => void;
    onClose: () => void;
  }
  let { kind, doc, titles, cols, col, header, onRun, onClose }: Props = $props();

  let myHeader = $state(untrack(() => header));
  let problem = $state("");

  // Group figures.
  let keys = $state<number[]>([]);
  let func = $state<GroupFunc>("count");
  let valueCol = $state(untrack(() => Math.min(col + 1, Math.max(0, titles.length - 1))));
  let groupTitle = $state("");
  const needsValue = $derived(func !== "count" && func !== "running");

  // Choose and order columns.
  let order = $state<{ c: number; keep: boolean; name: string }[]>(untrack(() => titles.map((_, c) => ({ c, keep: true, name: "" }))));
  const tooMany = $derived(titles.length < cols);
  function move(i: number, by: number): void {
    const j = i + by;
    if (j < 0 || j >= order.length) return;
    const next = [...order];
    [next[i], next[j]] = [next[j]!, next[i]!];
    order = next;
  }

  // Extract.
  let exCol = $state(untrack(() => col));
  let what = $state<ExtractWhat>("number");
  let remove = $state(false);
  let exTitle = $state("");

  // Build a column.
  let buildMode = $state<"template" | "rules">("template");
  let template = $state("");
  let rules = $state<Rule[]>(untrack(() => [{ col, op: "is", value: "", result: "" }]));
  let otherwise = $state("");
  let buildTitle = $state("");
  let preview = $state<ComputePreview | null>(null);
  const built = $derived(buildMode === "template" ? templateExpr(template, titles) : rulesExpr(rules, otherwise, titles));

  function computeSpec(): ComputeSpec {
    return {
      expr: built.expr,
      title: buildTitle.trim() || "New column",
      target: null,
      at: null,
      keep_first: myHeader,
      read: { decimal_comma: false, date_order: "mdy" },
      now: localDays(),
    };
  }

  let seq = 0;
  $effect(() => {
    if (kind !== "build") return;
    const s = computeSpec();
    const empty = buildMode === "template" ? !template.trim() : rules.length === 0;
    const mine = ++seq;
    if (empty || built.unknown.length) {
      preview = null;
      return;
    }
    const t = setTimeout(() => {
      backend()
        .computePreview(doc, s)
        .then((p) => {
          if (mine === seq) {
            preview = p;
            problem = "";
          }
        })
        .catch((e: unknown) => {
          if (mine === seq) {
            preview = null;
            problem = errorText(e);
          }
        });
    }, 200);
    return () => clearTimeout(t);
  });

  function insertName(c: number): void {
    template += `{${titles[c] ?? `#${c + 1}`}}`;
  }

  function run(): void {
    problem = "";
    switch (kind) {
      case "group":
        if (needsValue && valueCol === null) {
          problem = "Choose the column to work out.";
          return;
        }
        onRun(
          {
            op: "group_calc",
            spec: {
              keys: [...keys],
              value: needsValue ? valueCol : null,
              func,
              title: groupTitle.trim() || (GROUP_FUNCS.find((f) => f[0] === func)?.[1] ?? "Group figure").replace(/ \(.*\)$/, ""),
              at: keys.length ? Math.max(...keys) + 1 : null,
              match_case: false,
              trim: true,
              keep_first: myHeader,
            },
          },
          myHeader,
        );
        return;
      case "arrange": {
        if (tooMany) {
          problem = "This file has too many columns to list here.";
          return;
        }
        const kept = order.filter((o) => o.keep);
        if (!kept.length) {
          problem = "Keep at least one column.";
          return;
        }
        onRun({ op: "arrange", spec: { order: kept.map((o) => o.c), names: kept.map((o) => o.name.trim()), keep_first: myHeader } }, myHeader);
        return;
      }
      case "extract":
        onRun({ op: "extract", spec: { col: exCol, what, title: exTitle.trim(), remove, keep_first: myHeader } }, myHeader);
        return;
      case "build":
        if (built.unknown.length) {
          problem = `No column is called ${built.unknown.map((u) => `“${u}”`).join(", ")}.`;
          return;
        }
        if (buildMode === "template" && !template.trim()) {
          problem = "Write the template, for example {First} {Last}.";
          return;
        }
        onRun({ op: "compute", spec: computeSpec() }, myHeader);
        return;
    }
  }

  const TITLES: Record<ToolKind, string> = {
    group: "Group figures",
    arrange: "Choose and order columns",
    extract: "Extract into a column",
    build: "Build a column",
  };
</script>

<FormShell title={TITLES[kind]} {onClose} onEnter={run}>
  <label class="row"><input type="checkbox" bind:checked={myHeader} /> First row is header</label>
  {#if kind === "group"}
    <fieldset>
      <legend>Rows are in the same group when these match</legend>
      <AllNone count={titles.length} picked={keys} onChange={(v) => (keys = v)} />
      <div class="list">
        {#each titles as t, c (c)}
          <label><input type="checkbox" checked={keys.includes(c)} onchange={() => (keys = keys.includes(c) ? keys.filter((k) => k !== c) : [...keys, c].sort((a, b) => a - b))} /> {t}</label>
        {/each}
      </div>
      <p class="note">Tick none for one group of all rows.</p>
    </fieldset>
    <label class="row">Work out
      <select bind:value={func}>
        {#each GROUP_FUNCS as [v, label] (v)}<option value={v}>{label}</option>{/each}
      </select>
    </label>
    {#if needsValue}
      <label class="row">Of the column
        <select bind:value={valueCol}>{#each titles as t, c (c)}<option value={c}>{t}</option>{/each}</select>
      </label>
    {/if}
    <label class="row">New column's name <input type="text" bind:value={groupTitle} placeholder="(the figure's name)" /></label>
    <p class="note">The new column goes right after the group's columns. With a filter on, only shown rows are counted and filled.</p>
  {:else if kind === "arrange"}
    {#if tooMany}
      <p class="problem">This file has {cols.toLocaleString()} columns; only {titles.length.toLocaleString()} can be listed here.</p>
    {:else}
      <div class="row">
        <button type="button" onclick={() => (order = order.map((o) => ({ ...o, keep: true })))}>Keep all</button>
        <button type="button" onclick={() => (order = order.map((o) => ({ ...o, keep: false })))}>Keep none</button>
      </div>
      <div class="arrange" role="list">
        {#each order as o, i (o.c)}
          <div class="row" role="listitem" class:gone={!o.keep}>
            <input type="checkbox" bind:checked={o.keep} aria-label={`Keep ${titles[o.c]}`} />
            <span class="name">{titles[o.c]}</span>
            <input type="text" bind:value={o.name} placeholder="(same name)" aria-label={`New name for ${titles[o.c]}`} />
            <button type="button" onclick={() => move(i, -1)} disabled={i === 0} aria-label="Move up">▲</button>
            <button type="button" onclick={() => move(i, 1)} disabled={i === order.length - 1} aria-label="Move down">▼</button>
          </div>
        {/each}
      </div>
      <p class="note">Unticked columns are deleted. New names change the first row (with "First row is header"). One undo step.</p>
    {/if}
  {:else if kind === "extract"}
    <label class="row">From the column
      <select bind:value={exCol}>{#each titles as t, c (c)}<option value={c}>{t}</option>{/each}</select>
    </label>
    <label class="row">Take out
      <select bind:value={what}>
        {#each EXTRACTS as [v, label] (v)}<option value={v}>{label}</option>{/each}
      </select>
    </label>
    <label class="row"><input type="checkbox" bind:checked={remove} /> Also remove it from the original cell</label>
    <label class="row">New column's name <input type="text" bind:value={exTitle} placeholder="(column name + part)" /></label>
    <p class="note">The new column goes right after the original. Every row is looked at, hidden ones too.</p>
  {:else}
    <div class="row">
      <label class="row"><input type="radio" name="bmode" value="template" bind:group={buildMode} /> From a template</label>
      <label class="row"><input type="radio" name="bmode" value="rules" bind:group={buildMode} /> From if-then rules</label>
    </div>
    {#if buildMode === "template"}
      <label class="col">Template (column names in braces)
        <input type="text" bind:value={template} placeholder={"{First} {Last}"} spellcheck="false" />
      </label>
      <div class="chips">
        {#each titles.slice(0, 60) as t, c (c)}<button type="button" onclick={() => insertName(c)}>{t}</button>{/each}
      </div>
    {:else}
      {#each rules as r, i (i)}
        <div class="row rule">
          If
          <select bind:value={r.col} aria-label="Column">{#each titles as t, c (c)}<option value={c}>{t}</option>{/each}</select>
          <select bind:value={r.op} aria-label="Test">{#each RULE_OPS as [v, label] (v)}<option value={v}>{label}</option>{/each}</select>
          {#if r.op !== "blank" && r.op !== "not_blank"}<input type="text" bind:value={r.value} class="short" aria-label="Value" />{/if}
          then
          <input type="text" bind:value={r.result} placeholder={"text or {Column}"} aria-label="Result" />
          {#if rules.length > 1}<button type="button" onclick={() => (rules = rules.filter((_, k) => k !== i))} aria-label="Remove this rule">✕</button>{/if}
        </div>
      {/each}
      <button type="button" onclick={() => (rules = [...rules, { col, op: "is", value: "", result: "" }])}>Add a rule</button>
      <label class="row">Otherwise <input type="text" bind:value={otherwise} placeholder="(blank)" /></label>
      <p class="note">The first rule that holds decides the value.</p>
    {/if}
    <label class="row">New column's name <input type="text" bind:value={buildTitle} placeholder="New column" /></label>
    {#if built.unknown.length}<p class="problem">No column is called {built.unknown.map((u) => `“${u}”`).join(", ")}.</p>{/if}
    <details>
      <summary>As a computed-column expression</summary>
      <pre class="preview">{built.expr}</pre>
    </details>
    {#if preview}
      <div class="table-preview" aria-label="First results">
        <table><tbody>{#each preview.rows as [row, v] (row)}<tr><td>{row}</td><td>{v}</td></tr>{/each}</tbody></table>
      </div>
    {/if}
    <p class="note">The new column is added at the end; values are written as text.</p>
  {/if}
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" class="primary" onclick={run}>OK</button>
    <button type="button" onclick={onClose}>Cancel</button>
  {/snippet}
</FormShell>

<style>
  .arrange {
    max-height: 320px;
    overflow: auto;
  }
  .arrange .name {
    min-width: 12em;
  }
  .arrange .gone .name {
    text-decoration: line-through;
    color: var(--muted);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    max-height: 90px;
    overflow: auto;
  }
  .chips button {
    font-size: 12px;
  }
  .short {
    width: 8em;
  }
  .rule input:not(.short) {
    width: 12em;
  }
</style>
