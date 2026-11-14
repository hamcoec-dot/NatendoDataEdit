<script lang="ts">
  // Search a folder of files (Milestone 14): every table file in a folder
  // is searched without opening it in a tab; click a result to open the
  // file at that cell.
  import { onDestroy } from "svelte";
  import { backend, type PatternKind } from "./backend";
  import FormShell from "./FormShell.svelte";

  export interface FolderHit {
    path: string;
    row: number;
    col: number;
    text: string;
    start: number;
    len: number;
  }
  export interface FolderReport {
    hits: FolderHit[];
    cells: number;
    files: number;
    files_with_hits: number;
    skipped: [string, string][];
    too_many_files: boolean;
  }

  interface Props {
    /** The open tab, for where the folder picker starts. */
    doc: number | null;
    onOpen: (hit: FolderHit) => void;
    onClose: () => void;
  }
  let { doc, onOpen, onClose }: Props = $props();

  let folder = $state("");
  let text = $state("");
  let kind = $state<PatternKind>("plain");
  let matchCase = $state(false);
  let wholeCell = $state(false);
  let subfolders = $state(true);
  let types = $state("");
  let running = $state(false);
  let fraction = $state(0);
  let report = $state<FolderReport | null>(null);
  let problem = $state("");
  let poll: ReturnType<typeof setInterval> | undefined;
  onDestroy(() => {
    clearInterval(poll);
    if (running) void backend().folderSearchCancel();
  });

  async function pick(): Promise<void> {
    try {
      const f = await backend().pickSearchFolder(doc);
      if (f) folder = f;
    } catch (e) {
      problem = String((e as { message?: string })?.message ?? e);
    }
  }

  async function run(): Promise<void> {
    problem = "";
    if (!folder) {
      problem = "Choose the folder to search first.";
      return;
    }
    if (!text) {
      problem = "Type what to look for.";
      return;
    }
    running = true;
    fraction = 0;
    report = null;
    poll = setInterval(() => {
      void backend()
        .folderSearchProgress()
        .then((f) => {
          if (f !== null) fraction = f;
        })
        .catch(() => {});
    }, 300);
    try {
      report = await backend().searchFolder(folder, {
        pattern: { text, kind, match_case: matchCase, whole_cell: wholeCell },
        subfolders,
        types: types.split(/[\s,;]+/).filter(Boolean),
      });
    } catch (e) {
      const m = String((e as { message?: string })?.message ?? e);
      problem = /cancel/i.test(m) ? "Search stopped." : m;
    } finally {
      clearInterval(poll);
      running = false;
    }
  }

  const name = (p: string) => p.split(/[\\/]/).pop() ?? p;
  const rel = (p: string) => (folder && p.startsWith(folder) ? p.slice(folder.length).replace(/^[\\/]/, "") : p);
  const plural = (n: number, one: string) => `${n.toLocaleString()} ${n === 1 ? one : `${one}s`}`;
</script>

<FormShell title="Search a folder of files" wide {onClose} onEnter={() => void run()}>
  <div class="row">
    <button type="button" onclick={() => void pick()} disabled={running}>Choose folder…</button>
    <span class="folder" title={folder}>{folder || "No folder chosen"}</span>
  </div>
  <label class="row">Find <input type="text" bind:value={text} disabled={running} aria-label="Find" /></label>
  <div class="row">
    <label><input type="radio" bind:group={kind} value="plain" /> Text</label>
    <label><input type="radio" bind:group={kind} value="wildcard" /> Wildcards (* ?)</label>
    <label><input type="radio" bind:group={kind} value="regex" /> Pattern (regular expression)</label>
  </div>
  <div class="row">
    <label><input type="checkbox" bind:checked={matchCase} /> Match case</label>
    <label><input type="checkbox" bind:checked={wholeCell} /> Whole cell</label>
    <label><input type="checkbox" bind:checked={subfolders} /> Subfolders too</label>
  </div>
  <label class="row">File types <input type="text" bind:value={types} placeholder="csv tsv txt (the usual table files)" /></label>
  {#if running}
    <p class="note">Searching… {Math.round(fraction * 100)}% of the files</p>
  {/if}
  {#if report}
    <p>
      {report.cells ? `${plural(report.cells, "cell")} found in ${plural(report.files_with_hits, "file")}` : "Nothing found"} ({plural(report.files, "file")} searched).
      {#if report.cells > report.hits.length}The first {report.hits.length.toLocaleString()} are listed.{/if}
      {#if report.too_many_files}<strong> The folder has more files than are searched at once; choose a smaller folder.</strong>{/if}
    </p>
    {#if report.hits.length}
      <div class="hits" aria-label="Cells found">
        {#each report.hits as h, i (i)}
          <button type="button" class="hit" onclick={() => onOpen(h)} title={`Open ${h.path} at row ${h.row + 1}`}>
            <span class="file">{rel(h.path) || name(h.path)}</span>
            <span class="at">row {(h.row + 1).toLocaleString()}, col {h.col + 1}</span>
            <span class="text">{[...h.text].slice(0, h.start).join("")}<mark>{[...h.text].slice(h.start, h.start + h.len).join("")}</mark>{[...h.text].slice(h.start + h.len).join("")}</span>
          </button>
        {/each}
      </div>
    {/if}
    {#if report.skipped.length}
      <details>
        <summary>{plural(report.skipped.length, "file")} could not be read</summary>
        <ul>{#each report.skipped as [p, why] (p)}<li>{rel(p)}: {why}</li>{/each}</ul>
      </details>
    {/if}
  {/if}
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    {#if running}
      <button type="button" onclick={() => void backend().folderSearchCancel()}>Stop</button>
    {:else}
      <button type="button" class="primary" onclick={() => void run()}>Search</button>
    {/if}
    <button type="button" onclick={onClose}>Close</button>
  {/snippet}
</FormShell>

<style>
  .folder {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 40em;
    opacity: 0.85;
  }
  .hits {
    max-height: 320px;
    overflow: auto;
    border: 1px solid var(--border, #8884);
    border-radius: 4px;
  }
  .hit {
    display: grid;
    grid-template-columns: 14em 9em 1fr;
    gap: 8px;
    width: 100%;
    text-align: left;
    padding: 3px 8px;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    cursor: pointer;
  }
  .hit:hover {
    background: var(--hover, #8882);
  }
  .hit span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .at {
    opacity: 0.75;
  }
</style>
