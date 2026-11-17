<script lang="ts">
  // Mailing labels and letters (Milestone 14): a text with {Column} fields,
  // filled once per shown row and saved as a PDF to print.
  import { untrack } from "svelte";
  import FormShell from "./FormShell.svelte";
  import { DEFAULT_SHEET, suggestTemplate, unknownFields, type LabelSheet, type MailKind, type MailSpec } from "./mail";
  import { PAPERS } from "./m8";

  interface Props {
    titles: string[];
    header: boolean;
    /** Shown data rows (each gets a label or a letter). */
    rows: number;
    sheets: [string, LabelSheet][];
    /** The first few labels or letters as lines. */
    preview: (spec: MailSpec) => Promise<string[][]>;
    problem?: string;
    onSave: (spec: MailSpec) => void;
    onClose: () => void;
  }
  let { titles, header, rows, sheets, preview, problem = "", onSave, onClose }: Props = $props();

  let kind = $state<MailKind>("labels");
  let sheetIndex = $state(0);
  let template = $state(untrack(() => suggestTemplate(titles)));
  let letter = $state(untrack(() => `{${titles[0] ?? "1"}}\n\nDear {${titles[0] ?? "1"}},\n\n`));
  let skip = $state(0);
  let fontSize = $state(10);
  let paper = $state<MailSpec["paper"]>("letter");
  let marginMm = $state(25.4);
  let skipBlank = $state(true);
  let shown = $state<string[][]>([]);
  let previewProblem = $state("");
  let area = $state<HTMLTextAreaElement | null>(null);

  const sheet = $derived(sheets[sheetIndex]?.[1] ?? DEFAULT_SHEET);
  const perSheet = $derived(sheet.across * sheet.down);
  const text = $derived(kind === "labels" ? template : letter);
  const unknown = $derived(unknownFields(text, titles));
  const pages = $derived(kind === "labels" ? Math.ceil((rows + Math.min(skip, perSheet - 1)) / perSheet) : rows);

  function spec(): MailSpec {
    return {
      kind,
      template: text,
      header,
      sheet,
      skip: Math.max(0, Math.min(Math.floor(skip) || 0, perSheet - 1)),
      paper,
      margin_mm: marginMm,
      font_size: fontSize,
      skip_blank_lines: skipBlank,
      rows: null,
    };
  }

  // Preview after a short pause in typing.
  let timer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const s = spec();
    clearTimeout(timer);
    if (unknown.length || !s.template.trim()) {
      shown = [];
      return;
    }
    timer = setTimeout(() => {
      preview(s)
        .then((p) => {
          shown = p;
          previewProblem = "";
        })
        .catch((e: unknown) => {
          shown = [];
          previewProblem = e instanceof Error ? e.message : String((e as { message?: string })?.message ?? e);
        });
    }, 250);
  });

  function insertField(t: string): void {
    const field = `{${t.replace(/[{}]/g, "")}}`;
    const el = area;
    const value = kind === "labels" ? template : letter;
    const at = el ? el.selectionStart : value.length;
    const end = el ? el.selectionEnd : value.length;
    const next = value.slice(0, at) + field + value.slice(end);
    if (kind === "labels") template = next;
    else letter = next;
    queueMicrotask(() => {
      el?.focus();
      el?.setSelectionRange(at + field.length, at + field.length);
    });
  }

  function save(): void {
    if (unknown.length) return;
    if (!text.trim()) return;
    onSave(spec());
  }
</script>

<FormShell title="Mailing labels and letters" wide {onClose}>
  <div class="row">
    <label><input type="radio" bind:group={kind} value="labels" /> Labels</label>
    <label><input type="radio" bind:group={kind} value="letters" /> Letters (one page per row)</label>
  </div>
  {#if kind === "labels"}
    <label class="row">Label sheet
      <select bind:value={sheetIndex}>
        {#each sheets as [name], i (i)}<option value={i}>{name}</option>{/each}
      </select>
    </label>
    <label class="row">Labels already used on the first sheet <input type="number" min="0" max={perSheet - 1} bind:value={skip} /></label>
  {:else}
    <label class="row">Paper
      <select bind:value={paper}>
        {#each PAPERS as p (p.value)}<option value={p.value}>{p.label}</option>{/each}
      </select>
    </label>
    <label class="row">Margins (mm) <input type="number" min="0" max="80" step="0.1" bind:value={marginMm} /></label>
  {/if}
  <label class="row">Text size <input type="number" min="4" max="72" step="0.5" bind:value={fontSize} /> points{kind === "labels" ? " (smaller when a label is full)" : ""}</label>
  <label class="row">Insert a column
    <select value="" onchange={(e) => { const v = (e.currentTarget as HTMLSelectElement).value; if (v) insertField(v); (e.currentTarget as HTMLSelectElement).value = ""; }}>
      <option value="">Choose…</option>
      {#each titles as t, i (i)}<option value={t}>{t}</option>{/each}
    </select>
  </label>
  {#if kind === "labels"}
    <textarea bind:this={area} bind:value={template} rows="5" spellcheck="false" aria-label="Text of each label"></textarea>
  {:else}
    <textarea bind:this={area} bind:value={letter} rows="10" aria-label="Text of each letter"></textarea>
  {/if}
  <label class="row"><input type="checkbox" bind:checked={skipBlank} /> Leave out lines whose columns are all blank (a second address line)</label>
  <p class="note">Write a column's title in braces, like {"{City}"}, where its value goes. Each shown row gets one {kind === "labels" ? "label" : "letter"}: {rows.toLocaleString()} in all, {pages.toLocaleString()} {pages === 1 ? "page" : "pages"}.</p>
  {#if unknown.length}<p class="problem" role="alert">No column is called {unknown.map((u) => `"${u}"`).join(", ")}.</p>{/if}
  {#if previewProblem}<p class="problem" role="alert">{previewProblem}</p>{/if}
  {#if shown.length}
    <div class="preview" class:letters={kind === "letters"} aria-label="The first few">
      {#each shown as lines, i (i)}
        <div class="item">{#each lines as l, k (k)}<div>{l || " "}</div>{/each}</div>
      {/each}
    </div>
  {/if}
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" class="primary" data-enter onclick={save} disabled={!!unknown.length || !text.trim() || rows === 0}>Save as PDF…</button>
    <button type="button" onclick={onClose}>Close</button>
  {/snippet}
</FormShell>

<style>
  textarea {
    width: 100%;
    font-family: inherit;
    box-sizing: border-box;
  }
  .preview {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 6px;
    max-height: 220px;
    overflow: auto;
    margin: 6px 0;
  }
  .preview.letters {
    grid-template-columns: 1fr;
  }
  .item {
    border: 1px dashed var(--border, #888);
    border-radius: 4px;
    padding: 4px 6px;
    font-size: 12px;
    white-space: pre-wrap;
    overflow: hidden;
  }
</style>
