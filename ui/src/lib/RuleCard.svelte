<script lang="ts">
  // C10 "Show me the rule": the rule fill by example learned for a column, as a form the person
  // can change, with the first cells it would fill, how many it fills and the odd results among
  // them. Apply fills with exactly this rule (one undo step); Save as formula makes a computed
  // column instead. When the examples disagree, it names the one that breaks the rule.
  import { untrack } from "svelte";
  import { backend, isCmdError, type ReadOptions } from "./backend";
  import type { ExampleSpec } from "./clean";
  import FormShell from "./FormShell.svelte";
  import { CASES, CUTS, cutKey, kindWords, MAX_PIECES, splitBy, type Case, type Cut, type LearnReport, type Outliers, type Piece, type Program, type RulePreview } from "./rule";

  interface Props {
    doc: number;
    col: number;
    header: boolean;
    titles: string[];
    read: ReadOptions;
    /** Fill with this rule; true when it ran (the app closes the card). */
    onApply: (spec: ExampleSpec, leftRows: number[]) => Promise<boolean>;
    /** Add a computed column with this formula, to the right of the column. */
    onFormula: (formula: string) => void;
    onGoTo: (row: number) => void;
    onClose: () => void;
  }
  let { doc, col, header, titles, read, onApply, onFormula, onGoTo, onClose }: Props = $props();

  const target = untrack(() => col);
  const skip = untrack(() => header);
  let leaveOut = $state<number[]>([]);
  /** The shown rows of the examples left out (for the status line). */
  let leftRows = $state<number[]>([]);
  let report = $state<LearnReport | null>(null);
  let program = $state<Program | null>(null);
  /** The rule as the engine read it back (words, revision). */
  let words = $state("");
  let revision = $state<number | null>(null);
  let preview = $state<RulePreview | null>(null);
  let odd = $state<Outliers | null>(null);
  let counting = $state(false);
  let formula = $state<{ ok: string } | { why: string } | null>(null);
  let problem = $state("");
  let note = $state("");
  let working = $state(true);
  let edited = $state(false);
  let seq = 0;

  const spec = (p: Program | null): ExampleSpec => ({ col: target, skip_first: skip, program: p, leave_out: [...leaveOut] });
  const name = (c: number) => titles[c] ?? `column ${c + 1}`;
  const row = (r: number) => (r + 1).toLocaleString();
  const why = (e: unknown) => (isCmdError(e) ? e.message : String(e));

  /** Learn (or read back the edited rule), then the preview, the counts and the formula. */
  async function work(p: Program | null): Promise<void> {
    const my = ++seq;
    working = true;
    problem = "";
    try {
      const rep = await backend().exampleLearn(doc, spec(p));
      if (my !== seq) return;
      // An edited rule stays as the person has it (they may be typing still).
      if (!p) {
        report = rep;
        program = rep.program;
      }
      words = rep.words;
      revision = rep.revision;
      if (!rep.program) {
        preview = null;
        odd = null;
        formula = null;
        return;
      }
      const prog = rep.program;
      const [pv, f] = await Promise.all([
        backend().examplePreview(doc, spec(prog), prog),
        backend()
          .exampleFormula(doc, spec(prog), prog, read)
          .then((ok) => ({ ok }))
          .catch((e: unknown) => ({ why: why(e) })),
      ]);
      if (my !== seq) return;
      preview = pv;
      formula = f;
      void count(prog, my);
    } catch (e) {
      if (my === seq) problem = why(e);
    } finally {
      if (my === seq) working = false;
    }
  }

  async function count(p: Program, my: number): Promise<void> {
    counting = true;
    odd = null;
    try {
      const o = await backend().exampleOutliers(doc, spec(p), p, null);
      if (my === seq) odd = o;
    } catch (e) {
      if (my === seq && !(isCmdError(e) && e.kind === "cancelled")) note = `Could not count the results: ${why(e)}`;
    } finally {
      if (my === seq) counting = false;
    }
  }

  function stopCount(): void {
    void backend()
      .cancelJob(doc, "Looking for odd results")
      .catch(() => {});
  }

  let root: HTMLDivElement;
  $effect(() => {
    // The card itself takes the focus: Escape closes it, and Enter (off the boxes) applies.
    root.focus();
    untrack(() => void work(null));
    return () => {
      seq++;
      if (counting) stopCount();
    };
  });

  let timer = 0;
  /** A change on the form: read it back after a moment (typing stays smooth). */
  function changed(): void {
    edited = true;
    clearTimeout(timer);
    const p = program ? $state.snapshot(program) : null;
    timer = window.setTimeout(() => {
      if (p) void work(p);
    }, 250);
  }

  function setPiece(i: number, f: (x: Piece) => Piece): void {
    if (!program) return;
    program.pieces[i] = f($state.snapshot(program.pieces[i]!) as Piece);
    changed();
  }
  function setCut(i: number, key: string): void {
    setPiece(i, (x) => {
      if (x.kind !== "from") return x;
      const cut: Cut = key === "split" ? { split: { by: splitBy(x.cut) || "-" } } : (key as Cut);
      return { ...x, cut, index: cut === "whole" ? 0 : x.index };
    });
  }
  /** Piece number as people count (1 the first, -1 the last) to the engine's index. */
  function setWhich(i: number, v: string): void {
    const n = Math.trunc(Number.parseInt(v, 10));
    if (!Number.isFinite(n) || n === 0) return;
    setPiece(i, (x) => (x.kind === "from" ? { ...x, index: n > 0 ? n - 1 : n } : x));
  }
  function addText(): void {
    if (!program || program.pieces.length >= MAX_PIECES) return;
    program.pieces.push({ kind: "text", text: " " });
    changed();
  }
  function addFrom(): void {
    if (!program || program.pieces.length >= MAX_PIECES) return;
    const c = titles.findIndex((_, k) => k !== target);
    program.pieces.push({ kind: "from", col: Math.max(0, c), cut: "whole", index: 0, first: false, case: "keep" });
    changed();
  }
  function remove(i: number): void {
    if (!program || program.pieces.length <= 1) return;
    program.pieces.splice(i, 1);
    changed();
  }
  function move(i: number, by: number): void {
    if (!program) return;
    const j = i + by;
    if (j < 0 || j >= program.pieces.length) return;
    const p = $state.snapshot(program.pieces) as Piece[];
    [p[i], p[j]] = [p[j]!, p[i]!];
    program.pieces = p;
    changed();
  }

  function leaveItOut(): void {
    const c = report?.conflict;
    if (!c) return;
    leaveOut = [...leaveOut, c.id];
    leftRows = [...leftRows, c.row];
    note = `Row ${row(c.row)} is left out of learning and stays as you typed it.`;
    report = null;
    edited = false;
    void work(null);
  }

  function learnAgain(): void {
    edited = false;
    note = "";
    void work(null);
  }

  const ready = $derived(!!program && !working && !problem);

  let applying = false;
  async function apply(): Promise<void> {
    if (!ready || !program || applying) return;
    applying = true;
    const p = $state.snapshot(program) as Program;
    try {
      if (!(await onApply({ ...spec(p), revision }, [...leftRows]))) {
        // Refused (the table changed since, say): work it out again on the table as it is.
        note = "Not filled: the preview was worked out again on the table as it is now.";
        void work(edited ? p : null);
      }
    } finally {
      applying = false;
    }
  }

  function saveFormula(): void {
    if (!ready || !formula || !("ok" in formula)) return;
    onFormula(formula.ok);
  }

  /** Enter applies with the focus off the boxes; Ctrl+Enter from anywhere. */
  function onKey(e: KeyboardEvent): void {
    if (e.key !== "Enter" || e.isComposing) return;
    const t = e.target;
    const inBox = t instanceof HTMLInputElement || t instanceof HTMLSelectElement || t instanceof HTMLTextAreaElement || t instanceof HTMLButtonElement;
    if (e.ctrlKey || !inBox) {
      e.preventDefault();
      void apply();
    }
  }

  const which = (x: Piece) => (x.kind === "from" ? (x.index >= 0 ? x.index + 1 : x.index) : 0);
  const formulaWhy = $derived(!formula ? "Working out the rule…" : "why" in formula ? `No formula form: ${formula.why}.` : "");
</script>

<FormShell title={`The rule for ${name(target)}`} wide {onClose}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="card" tabindex="-1" bind:this={root} onkeydown={onKey}>
    {#if report?.conflict}
      {@const c = report.conflict}
      <div class="conflict" role="alert">
        <p>Row {row(c.row)} does not fit the rule the other examples share: you typed “{c.value}”.</p>
        <div class="acts">
          <button type="button" onclick={leaveItOut}>Leave it out</button>
          <button type="button" onclick={() => onGoTo(c.row)}>Go to row {row(c.row)}</button>
        </div>
      </div>
    {/if}
    {#if problem}
      <p class="problem" role="alert">
        {problem}
        {#if edited}<button type="button" class="small" onclick={learnAgain}>Learn it again</button>{/if}
      </p>
    {/if}
    {#if note}<p class="note">{note}</p>{/if}
    {#if program}
      <p class="words" aria-live="polite"><strong>Rule:</strong> {words || "…"}</p>
      <ol class="pieces" aria-label="The rule's pieces">
        {#each program.pieces as x, i (i)}
          <li>
            {#if x.kind === "text"}
              <label>Text <input type="text" value={x.text} maxlength="1000" oninput={(e) => setPiece(i, () => ({ kind: "text", text: e.currentTarget.value }))} /></label>
            {:else}
              <label
                >From
                <select value={x.col} onchange={(e) => setPiece(i, (p) => (p.kind === "from" ? { ...p, col: Number.parseInt(e.currentTarget.value, 10) } : p))}>
                  {#each titles as t, c (c)}{#if c !== target}<option value={c}>{t}</option>{/if}{/each}
                </select></label
              >
              <select aria-label="Which part" value={cutKey(x.cut)} onchange={(e) => setCut(i, e.currentTarget.value)}>
                {#each CUTS as [k, label] (k)}<option value={k}>{label}</option>{/each}
              </select>
              {#if typeof x.cut !== "string"}
                <input class="by" type="text" aria-label="Split at" maxlength="1" value={splitBy(x.cut)} oninput={(e) => { const by = e.currentTarget.value; if ([...by].length === 1) setPiece(i, (p) => (p.kind === "from" ? { ...p, cut: { split: { by } } } : p)); }} />
              {/if}
              {#if x.cut !== "whole"}
                <label title="1 the first, 2 the second; -1 the last, -2 the one before it">Number <input class="num" type="number" min="-64" max="64" value={which(x)} onchange={(e) => setWhich(i, e.currentTarget.value)} /></label>
              {/if}
              <label><input type="checkbox" checked={x.first} onchange={(e) => setPiece(i, (p) => (p.kind === "from" ? { ...p, first: e.currentTarget.checked } : p))} /> first letter</label>
              <select aria-label="Letters" value={x.case} onchange={(e) => setPiece(i, (p) => (p.kind === "from" ? { ...p, case: e.currentTarget.value as Case } : p))}>
                {#each CASES as [k, label] (k)}<option value={k}>{label}</option>{/each}
              </select>
            {/if}
            <span class="tools">
              <button type="button" class="small" aria-label="Move up" disabled={i === 0} onclick={() => move(i, -1)}>↑</button>
              <button type="button" class="small" aria-label="Move down" disabled={i === program.pieces.length - 1} onclick={() => move(i, 1)}>↓</button>
              <button type="button" class="small" aria-label="Remove this piece" disabled={program.pieces.length <= 1} title={program.pieces.length <= 1 ? "A rule needs at least one piece" : ""} onclick={() => remove(i)}>✕</button>
            </span>
          </li>
        {/each}
      </ol>
      <div class="acts">
        <button type="button" class="small" onclick={addFrom} disabled={program.pieces.length >= MAX_PIECES}>Add a piece of a column</button>
        <button type="button" class="small" onclick={addText} disabled={program.pieces.length >= MAX_PIECES}>Add text</button>
        {#if edited}<button type="button" class="small" onclick={learnAgain}>Learn it again from the examples</button>{/if}
      </div>
      <h3>What it would write</h3>
      {#if preview}
        {#if preview.rows.length}
          <table class="preview">
            <thead><tr><th>Row</th><th>Reads</th><th>Writes</th></tr></thead>
            <tbody>
              {#each preview.rows as r (r.row)}
                <tr>
                  <td>{row(r.row)}</td>
                  <td>{r.reads.map(([c, v]) => `${name(c)}: ${v}`).join(" · ")}</td>
                  <td>{#if r.result === null}<em>left empty (a piece is missing)</em>{:else}{r.result}{/if}</td>
                </tr>
              {/each}
            </tbody>
          </table>
          <p class="note">Showing the {preview.rows.length.toLocaleString()} found in the first {preview.looked.toLocaleString()} rows.</p>
        {:else}
          <p class="note">No empty cell to fill in the first {preview.looked.toLocaleString()} rows.</p>
        {/if}
      {/if}
      <div class="counts" aria-live="polite">
        {#if counting}
          <span>Counting the cells it fills…</span> <button type="button" class="small" onclick={stopCount}>Cancel</button>
        {:else if odd}
          <span>It fills {odd.fills.toLocaleString()} {odd.fills === 1 ? "cell" : "cells"}{odd.missed ? ` and leaves ${odd.missed.toLocaleString()} empty (a piece is missing)` : ""}. {odd.odd_rows ? `${odd.odd_rows.toLocaleString()} ${odd.odd_rows === 1 ? "result looks" : "results look"} odd:` : "None of the results look odd."}</span>
          {#if odd.rows.length}
            <ul class="odd">
              {#each odd.rows.slice(0, 20) as o (o.row)}
                <li><button type="button" class="link" onclick={() => onGoTo(o.row)}>Row {row(o.row)}</button> “{o.value}”: {o.kinds.map(kindWords).join(", ")}</li>
              {/each}
              {#if odd.odd_rows > 20}<li class="note">and {(odd.odd_rows - 20).toLocaleString()} more</li>{/if}
            </ul>
          {/if}
        {/if}
      </div>
    {:else if working}
      <p>Working out the rule…</p>
    {:else if !problem}
      <p class="problem">No single rule fits every example. Check them, or type one more.</p>
    {/if}
  </div>
  {#snippet buttons()}
    <button type="button" class="primary" onclick={() => void apply()} disabled={!ready} title={ready ? "Enter (off the boxes) or Ctrl+Enter" : "Wait for the rule to be worked out"}>Apply</button>
    <button type="button" onclick={saveFormula} disabled={!ready || !formula || !("ok" in formula)} title={formulaWhy}>Save as formula</button>
    <button type="button" onclick={onClose}>Cancel</button>
    {#if formulaWhy && ready}<span class="note why">{formulaWhy}</span>{/if}
  {/snippet}
</FormShell>

<style>
  .card {
    outline: none;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .pieces {
    margin: 0;
    padding-left: 1.4rem;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }
  .pieces li {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    align-items: center;
  }
  .tools {
    margin-left: auto;
    display: flex;
    gap: 0.2rem;
  }
  .by {
    width: 2.2rem;
  }
  .num {
    width: 4rem;
  }
  .acts {
    display: flex;
    gap: 0.4rem;
    flex-wrap: wrap;
  }
  .conflict {
    border: 1px solid var(--warn, #c80);
    border-radius: 6px;
    padding: 0.4rem 0.6rem;
  }
  .conflict p {
    margin: 0 0 0.3rem;
  }
  .preview {
    border-collapse: collapse;
    font-size: 0.85rem;
  }
  .preview th,
  .preview td {
    border-bottom: 1px solid var(--border);
    padding: 0.15rem 0.4rem;
    text-align: left;
  }
  h3 {
    font-size: 0.95rem;
    margin: 0.4rem 0 0;
  }
  .odd {
    margin: 0.2rem 0 0;
    padding-left: 1.2rem;
    font-size: 0.85rem;
  }
  .link {
    background: none;
    border: none;
    padding: 0;
    color: var(--accent);
    text-decoration: underline;
    cursor: pointer;
  }
  .note {
    color: var(--muted);
    font-size: 0.85rem;
    margin: 0;
  }
  .why {
    align-self: center;
  }
  .problem {
    color: var(--danger, #c00);
  }
</style>
