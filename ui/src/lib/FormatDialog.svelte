<script lang="ts">
  // How the chosen columns show their values: decimals, thousands, percent,
  // currency, date style or tick boxes. Display only: the file never
  // changes. Kept with saved views.
  import { untrack } from "svelte";
  import FormShell from "./FormShell.svelte";
  import { cleanFormat, DATE_STYLES, defaultFormat, formatCell, type DisplayFormat, type FormatKind } from "./m9";

  interface Props {
    /** What the columns are called, for the title. */
    what: string;
    /** The first column's format now, if any. */
    current: DisplayFormat | null;
    /** A few of the first column's values, for the preview. */
    samples: string[];
    onDone: (f: DisplayFormat | null) => void;
    onClose: () => void;
  }
  let { what, current, samples, onDone, onClose }: Props = $props();
  let f = $state<DisplayFormat>(untrack(() => current ?? defaultFormat("number")));

  const KINDS: { id: FormatKind; label: string }[] = [
    { id: "number", label: "Number" },
    { id: "percent", label: "Percent" },
    { id: "currency", label: "Currency" },
    { id: "date", label: "Date" },
    { id: "checkbox", label: "Tick boxes (yes/no, true/false)" },
  ];

  const preview = $derived.by(() => {
    const g = cleanFormat($state.snapshot(f));
    const list = samples.filter((s) => s.trim() !== "").slice(0, 5);
    if (!g) return [];
    return list.map((s) => [s, g.kind === "checkbox" ? s : formatCell(s, g)] as const);
  });

  function done(): void {
    onDone(cleanFormat($state.snapshot(f)));
  }
</script>

<FormShell title={`Display format: ${what}`} {onClose} onEnter={done}>
  <p class="note">Changes only how values look here. The file is saved exactly as it is.</p>
  <label class="row">Show as
    <select bind:value={f.kind}>
      {#each KINDS as k (k.id)}<option value={k.id}>{k.label}</option>{/each}
    </select>
  </label>
  {#if f.kind === "number" || f.kind === "percent" || f.kind === "currency"}
    <label class="row">Decimal places <input type="number" min="0" max="10" step="1" bind:value={f.decimals} /></label>
    <label class="row"><input type="checkbox" bind:checked={f.thousands} /> Thousands separators (1,000)</label>
  {/if}
  {#if f.kind === "currency"}
    <label class="row">Currency sign <input type="text" maxlength="4" bind:value={f.symbol} /></label>
  {/if}
  {#if f.kind === "date"}
    <label class="row">Date style
      <select bind:value={f.date}>
        {#each DATE_STYLES as d (d.id)}<option value={d.id}>{d.label}</option>{/each}
      </select>
    </label>
  {/if}
  {#if f.kind === "checkbox"}
    <p class="note">Cells holding yes/no, true/false, Y/N, 1/0 or x/blank show as tick boxes. Clicking a box writes the other value in the same style.</p>
  {:else if preview.length}
    <table class="preview" aria-label="Preview">
      <thead><tr><th>In the file</th><th>Shows as</th></tr></thead>
      <tbody>
        {#each preview as [a, b], i (i)}<tr><td>{a}</td><td>{b}</td></tr>{/each}
      </tbody>
    </table>
  {/if}
  {#snippet buttons()}
    <button type="button" class="primary" onclick={done}>OK</button>
    {#if current}<button type="button" onclick={() => onDone(null)}>Remove format</button>{/if}
    <button type="button" onclick={onClose}>Cancel</button>
  {/snippet}
</FormShell>

<style>
  .row {
    display: flex;
    gap: 8px;
    align-items: center;
    margin: 6px 0;
  }
  .row input[type="number"],
  .row input[type="text"] {
    width: 6em;
  }
  .note {
    color: var(--muted);
    font-size: 0.85rem;
    max-width: 36em;
  }
  .preview {
    border-collapse: collapse;
    margin-top: 8px;
    font-variant-numeric: tabular-nums;
  }
  .preview th,
  .preview td {
    border: 1px solid var(--border);
    padding: 2px 8px;
    text-align: left;
    max-width: 16em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
