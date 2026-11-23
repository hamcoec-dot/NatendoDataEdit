<script lang="ts">
  // Export: Excel, JSON, XML, HTML table, Markdown table or a template of
  // the user's own, to a file or the clipboard, with a live preview of the
  // first rows. Filtered-out rows are left out.
  import { untrack } from "svelte";
  import { backend, errorText, type ExportFormat, type ExportOptions } from "./backend";
  import FormShell from "./FormShell.svelte";
  import { exportDefaults, templateFor } from "./transfer";

  interface Props {
    doc: number;
    name: string;
    header: boolean;
    filtered: boolean;
    /** Column titles (first row, or "Column N"). */
    titles: string[];
    /** Rows shown (the title row too). */
    rows?: number;
    /** Open tabs (several: they can go to one workbook). A private,
     *  encrypted or audit-record tab is marked and starts unticked. */
    tabList?: { id: number; name: string; why: string }[];
    /** Column widths set on screen (pixels; 0 for the default). */
    widths?: number[];
    /** Rows with a height of their own, as [shown row, pixels] (none while a filter is on). */
    rowHeights?: [number, number][];
    onExport: (o: ExportOptions, to: "file" | "clipboard", tabIds?: number[]) => void;
    onClose: () => void;
  }
  let { doc, name, header, filtered, titles, rows = 0, tabList = [], widths = [], rowHeights = [], onExport, onClose }: Props = $props();

  /** Data rows a sheet holds under the title row. */
  const SHEET_ROWS = 1_048_575;
  /** Remembered: big tables go to separate files or to sheets. */
  const SPLIT_KEY = "nde.excelSplit";
  function lastSplit(): boolean {
    try {
      return localStorage.getItem(SPLIT_KEY) !== "sheets";
    } catch {
      return true;
    }
  }
  let allTabs = $state(false);
  let picked = $state<Record<number, boolean>>(untrack(() => Object.fromEntries(tabList.map((t) => [t.id, !t.why]))));
  const pickedIds = $derived(tabList.filter((t) => picked[t.id]).map((t) => t.id));
  let partRows = $state("");

  let o = $state<ExportOptions>(untrack(() => ({ ...exportDefaults(header, name.replace(/\.[^.]*$/, "").slice(0, 31) || "Sheet1"), split_files: lastSplit() })));
  const dataRows = $derived(Math.max(0, rows - (o.header ? 1 : 0)));
  const perPart = $derived(Math.min(SHEET_ROWS, Math.max(1, Math.floor(Number(partRows.replace(/,/g, "")) || SHEET_ROWS))));
  const parts = $derived(Math.max(1, Math.ceil(dataRows / perPart)));
  let preview = $state("");
  let problem = $state("");

  const FORMATS: [ExportFormat, string][] = [
    ["xlsx", "Excel workbook (.xlsx)"],
    ["json", "JSON"],
    ["json_lines", "JSON Lines (.jsonl, one record per line)"],
    ["xml", "XML"],
    ["html", "Web page table (HTML)"],
    ["markdown", "Markdown table"],
    ["template", "Template (your own layout)"],
    ["fixed_width", "Fixed width (columns lined up with spaces)"],
    ["sql", "SQL statements (CREATE TABLE and INSERT)"],
  ];

  $effect(() => {
    if (o.format === "template" && o.row === "") o.row = untrack(() => templateFor(titles.slice(0, 10)));
  });

  // Preview: after a short pause, so typing in a template stays smooth.
  let timer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const snap = $state.snapshot(o);
    clearTimeout(timer);
    timer = setTimeout(() => {
      if (snap.format === "xlsx") {
        preview = "";
        problem = "";
        return;
      }
      backend()
        .exportPreview(doc, snap)
        .then((p) => {
          preview = p;
          problem = "";
        })
        .catch((e: unknown) => {
          preview = "";
          problem = errorText(e);
        });
    }, 200);
    return () => clearTimeout(timer);
  });

  function run(to: "file" | "clipboard"): void {
    if (problem) return;
    const snap = $state.snapshot(o);
    if (snap.format === "xlsx") {
      snap.part_rows = partRows.trim() ? perPart : 0;
      snap.widths = widths;
      snap.row_heights = rowHeights;
      if (parts > 1)
        try {
          localStorage.setItem(SPLIT_KEY, snap.split_files ? "files" : "sheets");
        } catch {
          // Asked again next time.
        }
    }
    onExport(snap, to, snap.format === "xlsx" && allTabs ? pickedIds : undefined);
  }
</script>

<FormShell title="Export" wide {onClose} onEnter={() => run("file")}>
  <div class="grid2">
    <label class="row">Format
      <select bind:value={o.format}>
        {#each FORMATS as [v, label] (v)}<option value={v}>{label}</option>{/each}
      </select>
    </label>
    <label class="row"><input type="checkbox" bind:checked={o.header} /> First row holds the column names</label>
  </div>
  {#if o.format === "xlsx"}
    <div class="grid2">
      <label class="row">Sheet name <input type="text" bind:value={o.sheet} maxlength="31" disabled={allTabs} /></label>
      <label class="row"><input type="checkbox" bind:checked={o.numbers} /> Store numbers as numbers</label>
    </div>
    <label class="row"><input type="checkbox" bind:checked={o.look} /> Made for Excel users</label>
    <p class="note">
      {#if o.look}
        A bold title row that stays in view, with filter buttons; columns as wide as their values (or as you set them here); dates, money and percents as real
        Excel values with their formats; IDs, ZIP codes and phone numbers kept as text, so leading zeros stay and Excel shows no warnings.
      {:else}
        Values exactly as written: numbers are numbers only when Excel shows them the same ("007" and "1.50" stay text); dates stay text.
      {/if}
    </p>
    {#if tabList.length > 1}
      <label class="row"><input type="checkbox" bind:checked={allTabs} /> Several open tabs as one workbook, each on a sheet named after it</label>
      {#if allTabs}
        <fieldset class="split tabs" aria-label="Tabs">
          <legend>{pickedIds.length} of {tabList.length} tabs</legend>
          {#each tabList as t (t.id)}
            <label class="row"><input type="checkbox" bind:checked={picked[t.id]} /> {t.name}{#if t.why}<span class="why"> ({t.why})</span>{/if}</label>
          {/each}
          {#if tabList.some((t) => t.why && picked[t.id])}<p class="note warn">The workbook is a plain file anyone can read, private tabs too.</p>{/if}
        </fieldset>
      {/if}
    {/if}
    <label class="row">Rows per part <input type="text" inputmode="numeric" bind:value={partRows} placeholder="as many as fit (1,048,575)" /></label>
    {#if parts > 1 && !allTabs}
      <fieldset class="split" aria-label="Parts">
        <legend>{dataRows.toLocaleString()} rows{partRows.trim() ? "" : ", more than an Excel sheet holds"}: {parts.toLocaleString()} parts, each with the title row</legend>
        <label class="row"><input type="radio" name="split" checked={o.split_files} onchange={() => (o.split_files = true)} /> Separate files ("{o.sheet} (1 of {parts}).xlsx" and on)</label>
        <label class="row"><input type="radio" name="split" checked={!o.split_files} onchange={() => (o.split_files = false)} /> Sheets of one workbook</label>
        {#if !o.split_files && dataRows > 2_000_000}
          <p class="note warn">A workbook of several full sheets can be too much for Excel to open comfortably; separate files are easier to open and share.</p>
        {/if}
      </fieldset>
    {:else if allTabs}
      <p class="note">A tab longer than a sheet goes on over more sheets, each with the title row.</p>
    {/if}
  {:else if o.format === "json"}
    <label class="row"><input type="checkbox" bind:checked={o.arrays} /> Rows as lists of values (instead of name: value)</label>
    <label class="row"><input type="checkbox" bind:checked={o.pretty} /> Indented</label>
  {:else if o.format === "json_lines"}
    <label class="row"><input type="checkbox" bind:checked={o.arrays} /> Rows as lists of values (instead of name: value)</label>
    <p class="note">One record per line, as databases and web tools read and write logs and records. Open such a file with File › Open.</p>
  {:else if o.format === "xml"}
    <div class="grid2">
      <label class="row">Outer element <input type="text" bind:value={o.root} maxlength="100" /></label>
      <label class="row">Row element <input type="text" bind:value={o.record} maxlength="100" /></label>
    </div>
    <label class="row"><input type="checkbox" bind:checked={o.pretty} /> Indented</label>
  {:else if o.format === "sql"}
    <div class="grid2">
      <label class="row">Table name <input type="text" bind:value={o.table} maxlength="128" /></label>
      <label class="row">For
        <select bind:value={o.dialect}>
          <option value="standard">SQLite, PostgreSQL, Oracle ("names")</option>
          <option value="mysql">MySQL, MariaDB (`names`)</option>
          <option value="sql_server">SQL Server, Access ([names])</option>
        </select>
      </label>
      <label class="row">Rows per INSERT <input type="number" min="1" max="10000" bind:value={o.batch} /></label>
      <label class="row"><input type="checkbox" bind:checked={o.create} /> Start with CREATE TABLE</label>
    </div>
    <label class="row"><input type="checkbox" bind:checked={o.numbers} /> Write plain numbers without quotes (values with leading zeros, like ZIP codes, stay quoted)</label>
    <p class="note">Every column is created as text; empty cells are written as NULL.</p>
  {:else if o.format === "template"}
    <p class="note">Write how each row looks. <code>{"{Name}"}</code> puts in the value of the column named Name, <code>{"{#2}"}</code> the second column; <code>{"{{"}</code> and <code>{"}}"}</code> write braces.</p>
    <label class="block">Before the rows <textarea rows="2" bind:value={o.before} spellcheck="false"></textarea></label>
    <label class="block">Each row <textarea rows="3" bind:value={o.row} spellcheck="false"></textarea></label>
    <label class="block">After the rows <textarea rows="2" bind:value={o.after} spellcheck="false"></textarea></label>
  {/if}
  {#if filtered}<p class="note">A filter is on: only the rows it shows are exported.</p>{/if}
  {#if o.format !== "xlsx"}
    <p class="note">Preview (first rows):</p>
    <pre class="preview">{preview}</pre>
  {/if}
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    {#if o.format !== "xlsx"}<button type="button" onclick={() => run("clipboard")} disabled={!!problem}>Copy to clipboard</button>{/if}
    <button type="button" class="primary" onclick={() => run("file")} disabled={!!problem}>Export to file…</button>
    <button type="button" onclick={onClose}>Cancel</button>
  {/snippet}
</FormShell>

<style>
  .tabs {
    max-height: 180px;
    overflow: auto;
  }
  .why {
    color: var(--muted, #666);
  }
</style>
