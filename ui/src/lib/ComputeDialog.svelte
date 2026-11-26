<script lang="ts">
  // Computed column: an expression over each row's cells, with the first
  // results shown as you type. The values are written as text (a delimited
  // file cannot keep formulas), in a new column or over an existing one.
  import HiddenNote from "./HiddenNote.svelte";
  import { hiddenRows, hiddenWords, includeWhile } from "./shown.svelte";
  import { untrack } from "svelte";
  import { backend, errorText, type ComputePreview, type ComputeSpec } from "./backend";
  import FormShell from "./FormShell.svelte";
  import { columnRef, FUNCTIONS, localDays } from "./m8";
  import { formulaHelp } from "./cellformula";

  interface Props {
    doc: number;
    titles: string[];
    /** The column the cursor is in. */
    col: number;
    header: boolean;
    /** compute: a new column (or over one); change: over the cursor's
     *  column; filter: show only the rows where it is TRUE. */
    mode?: "compute" | "change" | "filter";
    onRun: (spec: ComputeSpec) => void;
    onClose: () => void;
  }
  let { doc, titles, col, header, mode = "compute", onRun, onClose }: Props = $props();

  let expr = $state(untrack(() => (mode === "change" ? columnRef(titles[col] ?? "", col) : "")));
  let title = $state("");
  let place = $state<"end" | "after" | "over">(untrack(() => (mode === "change" ? "over" : "end")));
  let target = $state(untrack(() => col));
  let preview = $state<ComputePreview | null>(null);
  let problem = $state("");
  let box = $state<HTMLTextAreaElement | null>(null);
  let fn = $state("");
  /** Fill a new column in hidden rows too (offered only while some are hidden). */
  let fillHidden = $state(false);

  function spec(): ComputeSpec {
    return {
      expr,
      title: title.trim() || "Computed",
      target: place === "over" ? target : null,
      at: place === "after" ? col + 1 : null,
      keep_first: header,
      read: { decimal_comma: false, date_order: "mdy" },
      now: localDays(),
      hidden: !hiddenRows.hidden || fillHidden,
    };
  }

  // LOOKUP tables are kept while this screen is open, let go when it closes.
  $effect(() => () => void backend().lookupClose().catch(() => {}));

  let seq = 0;
  $effect(() => {
    const s = spec();
    const mine = ++seq;
    if (!s.expr.trim()) {
      preview = null;
      problem = "";
      return;
    }
    const t = setTimeout(() => {
      backend()
        .computePreview(doc, s)
        .then((p) => {
          if (mine !== seq) return;
          preview = p;
          problem = "";
        })
        .catch((e: unknown) => {
          if (mine !== seq) return;
          preview = null;
          problem = errorText(e);
        });
    }, 150);
    return () => clearTimeout(t);
  });

  /** Put text at the cursor in the expression box. */
  function insert(text: string): void {
    const el = box;
    if (!el) {
      expr += text;
      return;
    }
    const a = el.selectionStart ?? expr.length;
    const b = el.selectionEnd ?? expr.length;
    expr = expr.slice(0, a) + text + expr.slice(b);
    queueMicrotask(() => {
      el.focus();
      el.selectionStart = el.selectionEnd = a + text.length;
    });
  }

  function run(): void {
    if (!expr.trim() || problem) return;
    onRun(spec());
  }

  // The formula helper: what fits the word being typed, and how the
  // function around the cursor is used.
  let caret = $state(0);
  let picked = $state(0);
  const help = $derived(formulaHelp(expr, caret, FUNCTIONS, titles));
  function noteCaret(): void {
    caret = box?.selectionStart ?? expr.length;
    picked = 0;
  }
  function accept(i: number): void {
    const s = help.suggest[i];
    const el = box;
    if (!s || !el) return;
    const end = caret;
    expr = expr.slice(0, help.from) + s.insert + expr.slice(end);
    const at = help.from + s.insert.length;
    caret = at;
    queueMicrotask(() => {
      el.focus();
      el.selectionStart = el.selectionEnd = at;
    });
  }
  function helperKey(e: KeyboardEvent): void {
    if (!help.suggest.length) return;
    if (e.key === "Tab") {
      e.preventDefault();
      accept(picked);
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const n = help.suggest.length;
      picked = (picked + (e.key === "ArrowDown" ? 1 : n - 1)) % n;
    }
  }
  const hint = $derived(help.hint || (FUNCTIONS.find((f) => f.name === fn)?.hint ?? ""));
  const heading = $derived(mode === "filter" ? "Filter by formula" : mode === "change" ? "Change a column with a formula" : "Computed column");

  // The status pill turns amber while this includes hidden rows.
  $effect(() => includeWhile(fillHidden && place !== "over"));
</script>

<FormShell title={heading} wide {onClose} onEnter={run}>
  <label class="block">{mode === "filter" ? "Show the rows where this is TRUE" : "Expression"}
    <textarea bind:this={box} bind:value={expr} rows="3" spellcheck="false" class="mono"
      oninput={noteCaret} onclick={noteCaret} onkeyup={(e) => { if (!["ArrowDown", "ArrowUp", "Tab"].includes(e.key)) noteCaret(); }} onkeydown={helperKey}
      placeholder={mode === "filter" ? `for example  AND(${columnRef(titles[0] ?? "", 0)} > 100, ISBLANK(${columnRef(titles[1] ?? "", 1)}))` : `for example  ${columnRef(titles[0] ?? "", 0)} & " " & UPPER(${columnRef(titles[1] ?? "", 1)})`}></textarea>
  </label>
  {#if help.suggest.length}
    <div class="suggest" role="listbox" aria-label="Suggestions">
      {#each help.suggest as s, i (s.label)}
        <button type="button" role="option" aria-selected={i === picked} class:on={i === picked} title={s.hint} onmousedown={(e) => { e.preventDefault(); accept(i); }}>{s.label}</button>
      {/each}
      <span class="note">Tab puts in the {picked === 0 ? "first" : "lit"} one</span>
    </div>
  {/if}
  <div class="grid2">
    <label class="row">Insert a column
      <select value="" onchange={(e) => { const v = (e.currentTarget as HTMLSelectElement).value; if (v !== "") insert(columnRef(titles[Number(v)] ?? "", Number(v))); (e.currentTarget as HTMLSelectElement).value = ""; }}>
        <option value="">Choose…</option>
        {#each titles as t, i (i)}<option value={i}>{columnRef(t, i)}</option>{/each}
      </select>
    </label>
    <label class="row">Insert a function
      <select bind:value={fn} onchange={() => { if (fn) insert(`${fn}(`); }}>
        <option value="">Choose…</option>
        {#each FUNCTIONS as f (f.name)}<option value={f.name}>{f.name}</option>{/each}
      </select>
    </label>
  </div>
  {#if hint}<p class="note mono">{hint}</p>{/if}
  <p class="note">Columns are written in brackets, like <code>[Price]</code> or <code>[#3]</code> for the third column. Text goes in double quotes. Join text with <code>&amp;</code>; compare with <code>= &lt;&gt; &lt; &gt; &lt;= &gt;=</code>.</p>
  {#if mode !== "filter"}
  <fieldset>
    <legend>Put the results</legend>
    <label class="row"><input type="radio" bind:group={place} value="end" /> in a new column at the end</label>
    <label class="row"><input type="radio" bind:group={place} value="after" /> in a new column after {titles[col] || `column ${col + 1}`}</label>
    <label class="row"><input type="radio" bind:group={place} value="over" /> over the values in
      <select bind:value={target} disabled={place !== "over"}>
        {#each titles as t, i (i)}<option value={i}>{t || `Column ${i + 1}`}</option>{/each}
      </select>
    </label>
    {#if place !== "over" && header}
      <label class="row">New column's title <input type="text" bind:value={title} placeholder="Computed" /></label>
    {/if}
    {#if place !== "over" && hiddenRows.hidden}
      <label class="row"><input type="checkbox" bind:checked={fillHidden} /> Also fill the hidden rows</label>
      <HiddenNote text={fillHidden ? "Including hidden rows: they are filled too." : hiddenWords("empty", hiddenRows.hidden, hiddenRows.total)} amber={fillHidden} />
    {/if}
  </fieldset>
  {/if}
  {#if preview}
    <div class="table-preview" aria-label="First results">
      <table>
        <tbody>
          {#each preview.rows as [r, v] (r)}
            <tr><td class="num">{r}</td><td class:err={v.startsWith("#")}>{v}</td></tr>
          {/each}
        </tbody>
      </table>
    </div>
    {#if mode === "filter"}
      <p class="note">The first {preview.rows.length} rows: TRUE rows stay shown.{preview.errors ? ` ${preview.errors} came out as errors and are hidden.` : ""} Data › Clear filter shows every row again.</p>
    {:else if place === "over"}
      <p class="note">The first {preview.rows.length} shown rows: {preview.rows.length - preview.errors} get the new value{preview.errors ? `, ${preview.errors} give an error and keep their value` : ""}. Hidden rows are left as they are.</p>
    {:else}
      <p class="note">The first {preview.rows.length} rows.{preview.errors ? ` ${preview.errors} came out as errors (like #VALUE!: text where a number is needed).` : ""} Values are saved as text; the expression is not kept in the file.</p>
    {/if}
  {/if}
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" class="primary" onclick={run} disabled={!expr.trim() || !!problem}>{mode === "filter" ? "Filter" : mode === "change" ? "Change" : "Compute"}</button>
    <button type="button" onclick={onClose}>Cancel</button>
  {/snippet}
</FormShell>

<style>
  .mono {
    font-family: ui-monospace, Consolas, monospace;
  }
  td.num {
    color: var(--muted);
    text-align: right;
  }
  .suggest {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    align-items: center;
    margin: 2px 0 6px;
  }
  .suggest button {
    font-family: ui-monospace, Consolas, monospace;
    padding: 1px 6px;
  }
  .suggest button.on {
    outline: 2px solid var(--accent, #3b82f6);
  }
  td.err {
    color: var(--danger, #c0392b);
  }
</style>
