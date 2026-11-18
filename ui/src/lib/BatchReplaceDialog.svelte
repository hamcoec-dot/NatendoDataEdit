<script lang="ts">
  // Replace from a list: many find-and-replace pairs at once, from a pasted
  // two-column list or from two columns of another open tab. All pairs are
  // applied in one pass (the longest match wins), shown first, and undone
  // in one step.
  import { untrack } from "svelte";
  import { backend, errorText, type BatchReplace, type ReplaceReport, type Scope } from "./backend";
  import { cellDiff } from "./textDiff";
  import FormShell from "./FormShell.svelte";
  import type { Tab } from "./tab.svelte";

  interface Props {
    tab: Tab;
    tabs: Tab[];
    /** Column titles of the shown tab. */
    titles: string[];
    /** The selected columns (first, count), for "Selected columns". */
    sel: { col: number; cols: number };
    /** Runs a long engine call with progress shown and Cancel offered. */
    run: <T>(label: string, f: () => Promise<T>) => Promise<T | null>;
    onDone: (r: ReplaceReport) => void;
    onClose: () => void;
  }
  let { tab, tabs, titles, sel, run, onDone, onClose }: Props = $props();

  let from = $state<"paste" | "tab">("paste");
  let text = $state("");
  const others = $derived(tabs.filter((t) => t !== tab));
  let srcId = $state(untrack(() => tabs.find((t) => t !== tab)?.id ?? -1));
  const src = $derived(others.find((t) => t.id === srcId) ?? null);
  let findCol = $state(0);
  let withCol = $state(1);
  let wholeCell = $state(true);
  let matchCase = $state(false);
  let where = $state<"all" | "cols">(untrack(() => (sel.cols > 0 && sel.cols < tab.info.cols ? "cols" : "all")));
  let problem = $state("");
  let preview = $state<ReplaceReport | null>(null);
  let previewed: BatchReplace | null = null;
  let pairCount = $state(0);

  const srcTitles = $derived(src ? Array.from({ length: Math.min(src.info.cols, 2000) }, (_, c) => (src.header ? src.cache.get(0, c) || `Column ${c + 1}` : `Column ${c + 1}`)) : []);

  /** Pasted lines: "find<TAB>replace" (or a comma when there is no tab). */
  function parsePasted(): [string, string][] {
    const out: [string, string][] = [];
    for (const line of text.split(/\r?\n/)) {
      if (!line.trim()) continue;
      const sep = line.includes("\t") ? "\t" : ",";
      const i = line.indexOf(sep);
      const a = i < 0 ? line : line.slice(0, i);
      const b = i < 0 ? "" : line.slice(i + 1);
      if (a) out.push([a, b]);
    }
    return out;
  }

  async function pairs(): Promise<[string, string][] | null> {
    if (from === "paste") return parsePasted();
    if (!src) return null;
    return backend().columnPairs(src.id, findCol, withCol, src.header);
  }

  function scope(): Scope {
    return where === "cols" ? { kind: "columns", cols: Array.from({ length: sel.cols }, (_, i) => sel.col + i) } : { kind: "all" };
  }

  // Changing anything closes the preview.
  $effect(() => {
    void [from, text, srcId, findCol, withCol, wholeCell, matchCase, where, tab.version];
    untrack(() => {
      preview = null;
      previewed = null;
    });
  });

  async function look(): Promise<void> {
    problem = "";
    try {
      const p = await pairs();
      if (!p) {
        problem = "Open the tab that holds the list first.";
        return;
      }
      if (!p.length) {
        problem = from === "paste" ? "Paste a list: one pair per line, what to find, a tab (or comma), then what to put instead." : "That column has nothing to find.";
        return;
      }
      if (from === "tab" && findCol === withCol) {
        problem = "Pick two different columns.";
        return;
      }
      pairCount = p.length;
      const spec: BatchReplace = { pairs: p, whole_cell: wholeCell, match_case: matchCase, scope: scope(), skip_first: tab.header, skip_cols: tab.columns.hiddenList() };
      const r = await run("Looking for matches", () => backend().batchPreview(tab.id, spec));
      if (!r) return;
      if (r.cells === 0) {
        problem = "Nothing in this tab matches the list.";
        return;
      }
      preview = r;
      previewed = spec;
    } catch (e) {
      problem = errorText(e);
    }
  }

  async function apply(): Promise<void> {
    const spec = previewed;
    if (!spec) return;
    try {
      const r = await run("Replacing", () => backend().batchConfirm(tab.id, spec));
      if (r) onDone(r);
    } catch (e) {
      problem = errorText(e);
      preview = null;
      previewed = null;
    }
  }
</script>

<FormShell title="Replace from a list" {onClose} onEnter={() => void (preview ? apply() : look())}>
  <fieldset>
    <legend>The list</legend>
    <label class="row"><input type="radio" bind:group={from} value="paste" /> Pasted here</label>
    <label class="row"><input type="radio" bind:group={from} value="tab" disabled={!others.length} /> Two columns of another open tab{others.length ? "" : " (open one first)"}</label>
    {#if from === "paste"}
      <textarea aria-label="Pairs to replace" rows="7" placeholder={"One pair per line: what to find, a tab (or comma), what to put instead\nTN\tTennessee\nGA\tGeorgia"} bind:value={text}></textarea>
      <p class="note">{parsePasted().length.toLocaleString()} pairs. Copy two columns from a spreadsheet and paste them here.</p>
    {:else}
      <label class="row">Tab <select bind:value={srcId}>{#each others as t (t.id)}<option value={t.id}>{t.info.name}</option>{/each}</select></label>
      <label class="row">Find what is in <select bind:value={findCol}>{#each srcTitles as t, c (c)}<option value={c}>{t}</option>{/each}</select></label>
      <label class="row">Put instead what is in <select bind:value={withCol}>{#each srcTitles as t, c (c)}<option value={c}>{t}</option>{/each}</select></label>
    {/if}
  </fieldset>
  <fieldset>
    <legend>How</legend>
    <label class="row"><input type="radio" bind:group={wholeCell} value={true} /> Whole cells only (a cell that is exactly "TN" becomes "Tennessee")</label>
    <label class="row"><input type="radio" bind:group={wholeCell} value={false} /> Text inside cells too (the longest match wins)</label>
    <label class="row"><input type="checkbox" bind:checked={matchCase} /> Match case</label>
    <label class="row"
      >In
      <select bind:value={where}>
        <option value="all">the whole tab</option>
        <option value="cols" disabled={sel.cols < 1}>the selected columns{sel.cols ? ` (${titles.slice(sel.col, sel.col + sel.cols).slice(0, 3).join(", ")}${sel.cols > 3 ? "…" : ""})` : ""}</option>
      </select>
    </label>
  </fieldset>
  <p class="note">Every pair is applied at once, so "A to B" and "B to C" never chain. Hidden columns are left alone. One undo step.</p>
  {#if preview}
    <div class="pv" role="region" aria-label="What changes">
      <p><b>{preview.cells.toLocaleString()} cell{preview.cells === 1 ? "" : "s"}</b> change ({pairCount.toLocaleString()} pairs). The first ones:</p>
      <table>
        <thead><tr><th>Row</th><th>Column</th><th>Before</th><th>After</th></tr></thead>
        <tbody>
          {#each preview.preview.slice(0, 50) as c, i (i)}
            {@const d = cellDiff(c.before, c.after)}
            <tr><td>{c.number + 1}</td><td>{titles[c.col] ?? c.col + 1}</td><td class="cell">{d.head}<del>{d.del}</del>{d.tail}</td><td class="cell">{d.head}<ins>{d.ins}</ins>{d.tail}</td></tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    {#if preview}
      <button type="button" class="primary" onclick={() => void apply()}>Replace {preview.cells.toLocaleString()} cell{preview.cells === 1 ? "" : "s"}</button>
    {:else}
      <button type="button" class="primary" onclick={() => void look()}>Show what changes</button>
    {/if}
    <button type="button" onclick={onClose}>Cancel</button>
  {/snippet}
</FormShell>

<style>
  .row {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  textarea {
    width: 100%;
    box-sizing: border-box;
    font-family: ui-monospace, Consolas, monospace;
    font-size: 12px;
  }
  .note {
    color: var(--muted);
    font-size: 0.85rem;
  }
  .problem {
    color: var(--danger, #c00);
  }
  .pv {
    max-height: 220px;
    overflow: auto;
    font-size: 12px;
  }
  table {
    border-collapse: collapse;
  }
  td,
  th {
    padding: 1px 6px;
    text-align: left;
    white-space: nowrap;
    max-width: 260px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  del {
    background: color-mix(in srgb, #e5484d 30%, transparent);
  }
  ins {
    background: color-mix(in srgb, #30a46c 30%, transparent);
    text-decoration: none;
  }
</style>
