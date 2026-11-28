<script lang="ts">
  // How the chosen columns show their values: decimals, thousands, percent,
  // currency, date style, time, date and time, a custom code or tick boxes.
  // Display only: the file never changes. Kept with saved views.
  import { untrack } from "svelte";
  import FormShell from "./FormShell.svelte";
  import { cleanFormat, codeColorCss, DATE_STYLES, defaultFormat, formatCellColored, isDarkColor, STARTER_CODES, TIME_STYLES, type DisplayFormat, type FormatKind } from "./m9";
  import { codeProblem } from "./numfmt";

  interface Props {
    /** What the columns are called, for the title. */
    what: string;
    /** The first column's format now, if any. */
    current: DisplayFormat | null;
    /** Open on this kind (Custom code… opens on custom). */
    start?: FormatKind;
    /** A few of the first column's values, for the preview. */
    samples: string[];
    onDone: (f: DisplayFormat | null) => void;
    onClose: () => void;
  }
  let { what, current, start, samples, onDone, onClose }: Props = $props();
  let f = $state<DisplayFormat>(
    untrack(() => (start && current?.kind !== start ? defaultFormat(start) : { ...defaultFormat(current?.kind ?? "number"), ...(current ?? {}) })),
  );

  const KINDS: { id: FormatKind; label: string }[] = [
    { id: "number", label: "Number" },
    { id: "percent", label: "Percent" },
    { id: "currency", label: "Currency" },
    { id: "date", label: "Date" },
    { id: "time", label: "Time" },
    { id: "datetime", label: "Date and time" },
    { id: "custom", label: "Custom code" },
    { id: "checkbox", label: "Tick boxes (yes/no, true/false)" },
  ];

  /** Why the custom code cannot be used ("Not supported at character 4: ..."), or "". */
  const problem = $derived(f.kind === "custom" ? (codeProblem(f.code) ?? "") : "");
  const dark = isDarkColor(getComputedStyle(document.documentElement).getPropertyValue("--surface").trim() || "#fff");
  const forced = matchMedia("(forced-colors: active)").matches;

  const preview = $derived.by(() => {
    const g = cleanFormat($state.snapshot(f));
    const list = samples.filter((s) => s.trim() !== "").slice(0, 8);
    if (!g) return [];
    return list.map((s) => {
      const shown = g.kind === "checkbox" ? { text: s, color: null } : formatCellColored(s, g);
      return [s, shown.text, codeColorCss(shown.color, dark, forced)] as const;
    });
  });

  function kindChanged(): void {
    if (f.kind === "custom" && !f.code) f.code = defaultFormat("custom").code;
    if (f.kind === "datetime" && f.date === "iso" && !current) f.date = "medium";
  }

  function starter(e: Event): void {
    const el = e.currentTarget as HTMLSelectElement;
    if (el.value) f.code = el.value;
    el.value = "";
  }

  function done(): void {
    if (problem) return;
    onDone(cleanFormat($state.snapshot(f)));
  }
</script>

<FormShell title={`Display format: ${what}`} {onClose} onEnter={done}>
  <p class="note">Changes only how values look here. The file is saved exactly as it is.</p>
  <label class="row">Show as
    <select bind:value={f.kind} onchange={kindChanged}>
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
  {#if f.kind === "date" || f.kind === "datetime"}
    <label class="row">Date style
      <select bind:value={f.date}>
        {#each DATE_STYLES as d (d.id)}<option value={d.id}>{d.label}</option>{/each}
      </select>
    </label>
  {/if}
  {#if f.kind === "time" || f.kind === "datetime"}
    <label class="row">Time style
      <select bind:value={f.time}>
        {#each TIME_STYLES as t (t.id)}<option value={t.id}>{t.label}</option>{/each}
      </select>
    </label>
    <p class="note">The clock time shows as written: a time zone (Z, +05:30) is not shown and not converted. Seconds are cut, not rounded.</p>
  {/if}
  {#if f.kind === "custom"}
    <label class="row">Code
      <input class="code" type="text" maxlength="200" spellcheck="false" autocomplete="off" aria-describedby="code-help" aria-invalid={problem ? "true" : "false"} bind:value={f.code} />
    </label>
    <label class="row">Start from
      <select aria-label="Start from a code" onchange={starter}>
        <option value="">Choose a starter code…</option>
        {#each STARTER_CODES as s (s.code)}<option value={s.code}>{s.label}: {s.code}</option>{/each}
      </select>
    </label>
    {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
    <p class="note" id="code-help">0 is a digit (leading zeros kept), # a digit if there is one, ? a digit or a space; . and , as in 1,234.50; % is times 100; "text" or \x is written as it is; [Red], [Blue], [Green] and other colours; [&gt;100] conditions; ; between positive;negative;zero;text, with @ for the text; y m d h s AM/PM for dates and times.</p>
  {/if}
  {#if f.kind === "checkbox"}
    <p class="note">Cells holding yes/no, true/false, Y/N, 1/0 or x/blank show as tick boxes. Clicking a box writes the other value in the same style.</p>
  {:else if preview.length}
    <table class="preview" aria-label="Preview">
      <thead><tr><th>In the file</th><th>Shows as</th></tr></thead>
      <tbody>
        {#each preview as [a, b, color], i (i)}<tr><td>{a}</td><td style:color>{b}</td></tr>{/each}
      </tbody>
    </table>
  {/if}
  {#snippet buttons()}
    <button type="button" class="primary" onclick={done} disabled={!!problem}>OK</button>
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
  .row input.code {
    width: 22em;
    font-family: var(--mono, ui-monospace, monospace);
  }
  .problem {
    color: var(--danger, #b3261e);
    font-size: 0.85rem;
    margin: 2px 0 6px;
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
