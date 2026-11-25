<script lang="ts">
  // Print and Save as PDF: page setup with a preview of the first page.
  // Print hands the pages to the Windows print dialog (its own preview,
  // printer choice and "Save as PDF"); Save as PDF writes the file in the
  // engine, any size, with the same setup.
  import { untrack } from "svelte";
  import type { PageSetup } from "./backend";
  import FormShell from "./FormShell.svelte";
  import { PAPERS, PRINT_MAX_CELLS, PRINT_MAX_ROWS, printHtml } from "./m8";

  interface Props {
    setup: PageSetup;
    /** Column titles shown (null: the file has no header row). */
    titles: string[] | null;
    /** The first rows of what would print, for the preview. */
    sample: string[][];
    /** Rows that would print with "all rows" and with "selected rows". */
    allRows: number;
    selRows: number;
    cols: number;
    fontFamily: string;
    /** Start with Save as PDF as the main button. */
    pdfFirst: boolean;
    /** By printed column: typed Text or Id (never set as numbers). */
    textCols?: boolean[];
    onPrint: (s: PageSetup, selection: boolean) => void;
    onPdf: (s: PageSetup, selection: boolean) => void;
    onClose: () => void;
  }

  let { setup, titles, sample, allRows, selRows, cols, fontFamily, pdfFirst, textCols = [], onPrint, onPdf, onClose }: Props = $props();

  let s = $state<PageSetup>(untrack(() => structuredClone($state.snapshot(setup))));
  let selection = $state(false);
  let margins = $state<"normal" | "narrow" | "wide" | "custom">(untrack(() => (setup.margin_mm === 12.7 ? "normal" : setup.margin_mm === 6.35 ? "narrow" : setup.margin_mm === 25.4 ? "wide" : "custom")));
  $effect(() => {
    if (margins === "normal") s.margin_mm = 12.7;
    else if (margins === "narrow") s.margin_mm = 6.35;
    else if (margins === "wide") s.margin_mm = 25.4;
  });

  const rows = $derived(selection ? selRows : allRows);
  const tooBig = $derived(rows > PRINT_MAX_ROWS || rows * cols > PRINT_MAX_CELLS);
  const paper = $derived(PAPERS.find((p) => p.value === s.paper) ?? PAPERS[0]!);
  const pageW = $derived(s.landscape ? paper.size[1] : paper.size[0]);
  const pageH = $derived(s.landscape ? paper.size[0] : paper.size[1]);
  // The preview page is drawn at full size and scaled to fit the box.
  const boxW = 300;
  const scale = $derived(boxW / (pageW * 3.7795));
  const html = $derived(printHtml(titles, sample, $state.snapshot(s) as PageSetup, fontFamily, true, textCols));
  const fontOk = $derived(Number.isFinite(s.font_size) && s.font_size >= 4 && s.font_size <= 72);
</script>

<FormShell title={pdfFirst ? "Save as PDF" : "Print"} wide {onClose} onEnter={() => (pdfFirst ? onPdf($state.snapshot(s) as PageSetup, selection) : !tooBig && onPrint($state.snapshot(s) as PageSetup, selection))}>
  <div class="layout">
    <div class="opts">
      <label class="row">Paper
        <select bind:value={s.paper}>{#each PAPERS as p (p.value)}<option value={p.value}>{p.label}</option>{/each}</select>
      </label>
      <label class="row">Orientation
        <select bind:value={s.landscape}><option value={false}>Portrait</option><option value={true}>Landscape</option></select>
      </label>
      <label class="row">Margins
        <select bind:value={margins}>
          <option value="normal">Normal (12.7 mm)</option>
          <option value="narrow">Narrow (6.35 mm)</option>
          <option value="wide">Wide (25.4 mm)</option>
          <option value="custom">Custom…</option>
        </select>
        {#if margins === "custom"}<input type="number" min="0" max="100" step="0.5" bind:value={s.margin_mm} aria-label="Margin in millimetres" /> mm{/if}
      </label>
      <label class="row">Text size <input type="number" min="4" max="72" step="0.5" bind:value={s.font_size} /> pt</label>
      <label class="row"><input type="checkbox" bind:checked={s.fit_width} /> Fit all columns to the page width</label>
      <label class="row"><input type="checkbox" bind:checked={s.grid} /> Grid lines</label>
      {#if titles}<label class="row"><input type="checkbox" bind:checked={s.repeat_header} /> Column titles on every page</label>{/if}
      <label class="row"><input type="checkbox" bind:checked={s.page_numbers} /> Page numbers</label>
      <label class="row">Title <input type="text" bind:value={s.title} /></label>
      <fieldset>
        <legend>Print</legend>
        <label class="row"><input type="radio" bind:group={selection} value={false} /> all rows shown ({allRows.toLocaleString()})</label>
        <label class="row"><input type="radio" bind:group={selection} value={true} disabled={selRows < 1} /> selected rows ({selRows.toLocaleString()})</label>
      </fieldset>
      <p class="note">Hidden columns and rows a filter hides are left out.</p>
    </div>
    <div class="preview" style:width={`${boxW}px`} style:height={`${(boxW * pageH) / pageW}px`} aria-label="Preview of the first page">
      <iframe title="Preview" srcdoc={html} sandbox="" style:width={`${pageW}mm`} style:height={`${pageH}mm`} style:transform={`scale(${scale})`}></iframe>
    </div>
  </div>
  {#if tooBig && !pdfFirst}
    <p class="problem">That is more than Print handles at once ({PRINT_MAX_ROWS.toLocaleString()} rows or {PRINT_MAX_CELLS.toLocaleString()} cells). Save as PDF has no limit, or print selected rows.</p>
  {/if}
  {#if !fontOk}<p class="problem">The text size must be from 4 to 72 points.</p>{/if}
  {#snippet buttons()}
    {#if pdfFirst}
      <button type="button" class="primary" disabled={!fontOk} onclick={() => onPdf($state.snapshot(s) as PageSetup, selection)}>Save as PDF…</button>
      <button type="button" disabled={tooBig || !fontOk} onclick={() => onPrint($state.snapshot(s) as PageSetup, selection)}>Print…</button>
    {:else}
      <button type="button" class="primary" disabled={tooBig || !fontOk} onclick={() => onPrint($state.snapshot(s) as PageSetup, selection)}>Print…</button>
      <button type="button" disabled={!fontOk} onclick={() => onPdf($state.snapshot(s) as PageSetup, selection)}>Save as PDF…</button>
    {/if}
    <button type="button" onclick={onClose}>Cancel</button>
  {/snippet}
</FormShell>

<style>
  .layout {
    display: flex;
    gap: 16px;
    align-items: flex-start;
    flex-wrap: wrap;
  }
  .opts {
    flex: 1;
    min-width: 260px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .opts input[type="number"] {
    width: 5em;
  }
  .preview {
    position: relative;
    overflow: hidden;
    background: #fff;
    box-shadow: 0 1px 6px rgb(0 0 0 / 25%);
    flex: none;
  }
  .preview iframe {
    position: absolute;
    top: 0;
    left: 0;
    border: 0;
    transform-origin: 0 0;
    pointer-events: none;
    background: #fff;
  }
</style>
