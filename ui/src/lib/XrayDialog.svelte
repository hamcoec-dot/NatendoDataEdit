<script lang="ts">
  // X-ray report: look-alike characters, odd spaces, smart quotes, tabs and breaks over the shown rows of the
  // chosen columns (or the whole file). One job with progress and Cancel; only counts and row numbers come
  // back, never a value. A click shows only the rows that have one, or opens Sanitize.
  import { backend, errorText, type DocInfo, type XrayReport } from "./backend";
  import FormShell from "./FormShell.svelte";

  interface Props {
    doc: number;
    titles: string[];
    /** The columns to look at (empty: every column). */
    cols: number[];
    /** The first row holds titles (left out). */
    header: boolean;
    onShow: (info: DocInfo) => void;
    onSanitize: () => void;
    onClose: () => void;
  }
  let { doc, titles, cols, header, onShow, onSanitize, onClose }: Props = $props();

  const KIND_NAME: Record<string, string> = {
    zero_width: "zero-width characters",
    odd_space: "odd spaces",
    control: "control characters",
    smart_quote: "smart quotes",
    tab: "tabs",
    line_break: "line breaks inside a cell",
    homoglyph: "look-alike letters",
  };

  let report = $state<XrayReport | null>(null);
  let problem = $state("");
  let busy = $state(true);

  $effect(() => {
    let live = true;
    busy = true;
    backend()
      .xrayReport(doc, cols, header)
      .then((r) => {
        if (live) report = r;
      })
      .catch((e: unknown) => {
        if (live) problem = errorText(e);
      })
      .finally(() => {
        if (live) busy = false;
      });
    return () => {
      live = false;
    };
  });

  async function show(): Promise<void> {
    busy = true;
    try {
      onShow(await backend().xrayShow(doc, cols, header));
    } catch (e) {
      problem = errorText(e);
      busy = false;
    }
  }

  const name = (c: number) => titles[c] || `Column ${c + 1}`;
  const words = (n: number, one: string, many: string) => `${n.toLocaleString()} ${n === 1 ? one : many}`;
</script>

<FormShell title="X-ray report" {onClose}>
  <div class="body" aria-live="polite">
    {#if problem}
      <p class="problem" role="alert">{problem}</p>
    {:else if busy && !report}
      <p>Looking at the cells…</p>
    {:else if report}
      {#if report.cells === 0}
        <p data-testid="xray-none">Nothing found in {report.looked.toLocaleString()} cells: no look-alike letters, odd spaces, smart quotes, tabs or line breaks.</p>
      {:else}
        <p data-testid="xray-summary">
          {words(report.cells, "cell", "cells")} in {words(report.rows, "row", "rows")} hold something that looks right but is not (of {report.looked.toLocaleString()} cells looked at).
        </p>
        <ul class="kinds">
          {#each report.kinds as [k, n] (k)}<li>{n.toLocaleString()} {KIND_NAME[k] ?? k}</li>{/each}
        </ul>
        <table>
          <thead><tr><th>Column</th><th>Cells</th><th>What</th></tr></thead>
          <tbody>
            {#each report.columns as c (c.col)}
              <tr><td>{name(c.col)}</td><td>{c.cells.toLocaleString()}</td><td>{c.kinds.map(([k, n]) => `${n.toLocaleString()} ${KIND_NAME[k] ?? k}`).join(", ")}</td></tr>
            {/each}
          </tbody>
        </table>
      {/if}
    {/if}
  </div>
  {#snippet buttons()}
    {#if report && report.cells > 0}
      <button type="button" class="primary" disabled={busy} onclick={() => void show()}>Show only these rows</button>
      <button type="button" disabled={busy} onclick={onSanitize}>Sanitize…</button>
    {/if}
    <button type="button" onclick={onClose}>{busy && !report ? "Cancel" : "Close"}</button>
  {/snippet}
</FormShell>

<style>
  .body {
    min-width: 420px;
    max-height: 60vh;
    overflow: auto;
  }
  .kinds {
    margin: 0 0 0.6rem;
    padding-left: 1.2rem;
  }
  table {
    border-collapse: collapse;
    width: 100%;
  }
  th,
  td {
    text-align: left;
    padding: 2px 8px;
    border-bottom: 1px solid var(--border, #ddd);
  }
  .problem {
    color: var(--danger, #b00020);
  }
</style>
