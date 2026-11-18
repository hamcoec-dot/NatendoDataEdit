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
    onExport: (o: ExportOptions, to: "file" | "clipboard") => void;
    onClose: () => void;
  }
  let { doc, name, header, filtered, titles, onExport, onClose }: Props = $props();

  let o = $state<ExportOptions>(untrack(() => ({ ...exportDefaults(header, name.replace(/\.[^.]*$/, "").slice(0, 31) || "Sheet1") })));
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
    onExport($state.snapshot(o), to);
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
      <label class="row">Sheet name <input type="text" bind:value={o.sheet} maxlength="31" /></label>
      <label class="row"><input type="checkbox" bind:checked={o.numbers} /> Store numbers as numbers</label>
    </div>
    <p class="note">
      Numbers are stored as numbers only when Excel shows them exactly as written ("007" and "1.50" stay text). Dates stay as written. A sheet holds at most
      1,048,576 rows; more rows continue on further sheets.
    </p>
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
