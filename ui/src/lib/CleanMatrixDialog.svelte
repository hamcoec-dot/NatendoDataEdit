<script lang="ts">
  // Clean > Clean matrix: columns down the side, cleaners across. Tick, look
  // at the counts, then clean everything in one pass and one undo step.
  import { untrack } from "svelte";
  import AllNone from "./AllNone.svelte";
  import FormShell from "./FormShell.svelte";
  import { buildMatrix, emptyPick, ORDER_TEXT, TICKS, type CasePick, type CleanMatrix, type ColumnPick, type MatrixPreview, type NamedCleaner, type TickId } from "./matrix";

  interface Props {
    titles: string[];
    /** Columns ticked to begin with. */
    picked: number[];
    hidden: number[];
    protectedCols: number[];
    /** Columns set to Text or Id: never read as dates. */
    textCols: number[];
    header: boolean;
    /** The table's state: a preview is only good for the one it read. */
    revision: string;
    readOnly: boolean;
    onPreview: (spec: CleanMatrix) => Promise<MatrixPreview | null>;
    onCancelRun: () => void;
    onOpenCleaner: () => Promise<NamedCleaner | null>;
    /** `counted`: the revision the preview shown was counted at. */
    onApply: (spec: CleanMatrix, counted: number) => void;
    onClose: () => void;
  }
  let { titles, picked, hidden, protectedCols, textCols, header, revision, readOnly, onPreview, onCancelRun, onOpenCleaner, onApply, onClose }: Props = $props();
  const init = untrack(() => ({ header, picked }));
  let cols = $state<number[]>([...init.picked]);
  let picks = $state<Record<number, ColumnPick>>({});
  let busy = $state(false);
  let preview = $state<MatrixPreview | null>(null);
  /** What the preview was made with. */
  let ran = $state("");
  let problem = $state("");

  const kept = $derived(new Set(protectedCols));
  const away = $derived(new Set(hidden));
  const words = $derived(new Set(textCols));
  /** Shown columns first, then the hidden ones. */
  const rows = $derived([...titles.map((_, c) => c).filter((c) => !away.has(c)), ...titles.map((_, c) => c).filter((c) => away.has(c))]);
  const firstHidden = $derived(rows.find((c) => away.has(c)));
  const spec = $derived(buildMatrix(cols, picks, titles, init.header, { decimal_comma: false, date_order: "mdy" }));
  const key = $derived(JSON.stringify([spec, revision]));
  const stale = $derived(!preview || ran !== key);
  const idle = $derived(cols.filter((c) => !spec.entries.some((e) => e.col === c)).length);

  function pickOf(c: number): ColumnPick {
    return picks[c] ?? emptyPick();
  }
  function setPick(c: number, change: Partial<ColumnPick>): void {
    picks[c] = { ...pickOf(c), ...change };
  }
  function tickColumn(c: number, on: boolean): void {
    cols = on ? [...cols, c] : cols.filter((x) => x !== c);
  }
  function tickCell(c: number, id: TickId, on: boolean): void {
    const ticks = pickOf(c).ticks;
    setPick(c, { ticks: on ? [...ticks, id] : ticks.filter((x) => x !== id) });
    if (on && !cols.includes(c)) tickColumn(c, true);
  }
  /** A header tick: the cleaner for every ticked column. */
  function tickAll(id: TickId, on: boolean): void {
    for (const c of cols) tickCell(c, id, on);
  }
  function datesAll(on: boolean): void {
    for (const c of cols) if (!words.has(c)) setPick(c, { dates: on });
  }
  function caseAll(v: CasePick): void {
    for (const c of cols) setPick(c, { case: v });
  }
  async function chooseCleaner(c: number): Promise<void> {
    problem = "";
    try {
      const saved = await onOpenCleaner();
      if (!saved) return;
      if (!saved.cleaner.steps.length) {
        problem = "That cleaner has no steps.";
        return;
      }
      if (saved.cleaner.steps.some((s) => s.tool === "scramble")) {
        problem = "A cleaner that scrambles needs its secret phrase: use Clean › Cleaning tools for it.";
        return;
      }
      setPick(c, { saved });
      if (!cols.includes(c)) tickColumn(c, true);
    } catch (e) {
      problem = (e as { message?: string }).message ?? String(e);
    }
  }
  async function look(): Promise<void> {
    if (busy || !spec.entries.length) return;
    const s = $state.snapshot(spec) as CleanMatrix;
    const at = key;
    busy = true;
    try {
      const p = await onPreview(s);
      if (!p) return;
      preview = p;
      ran = at;
    } finally {
      busy = false;
    }
  }
  const countOf = (c: number) => (stale ? undefined : preview?.columns.find((x) => x.col === c));
  function close(): void {
    if (busy) onCancelRun();
    onClose();
  }
</script>

<FormShell title="Clean matrix" wide onClose={close} onEnter={() => void look()}>
  <AllNone count={titles.length} picked={cols} skip={(c) => away.has(c) || kept.has(c)} onChange={(v) => (cols = v)} />
  <div class="scroll">
    <table>
      <thead>
        <tr>
          <th scope="col">Column</th>
          {#each TICKS as t (t.id)}
            <th scope="col">
              <label
                ><span>{t.label}</span>
                <input
                  type="checkbox"
                  aria-label={`${t.label} for every ticked column`}
                  disabled={busy || !cols.length}
                  checked={cols.length > 0 && cols.every((c) => pickOf(c).ticks.includes(t.id))}
                  onchange={(e) => tickAll(t.id, (e.currentTarget as HTMLInputElement).checked)}
                /></label
              >
            </th>
          {/each}
          <th scope="col">
            Case
            <select aria-label="Case for every ticked column" disabled={busy || !cols.length} onchange={(e) => caseAll((e.currentTarget as HTMLSelectElement).value as CasePick)}>
              <option value="">No change</option>
              <option value="upper">UPPER</option>
              <option value="lower">lower</option>
              <option value="title">Proper</option>
            </select>
          </th>
          <th scope="col">Saved cleaner</th>
          <th scope="col">
            <label
              ><span>Dates as yyyy-mm-dd</span>
              <input
                type="checkbox"
                aria-label="Dates as yyyy-mm-dd for every ticked column"
                disabled={busy || !cols.some((c) => !words.has(c))}
                checked={cols.some((c) => !words.has(c)) && cols.every((c) => words.has(c) || pickOf(c).dates)}
                onchange={(e) => datesAll((e.currentTarget as HTMLInputElement).checked)}
              /></label
            >
          </th>
          <th scope="col">Would change</th>
        </tr>
      </thead>
      <tbody>
        {#each rows as c (c)}
          {#if c === firstHidden}<tr><th colspan={TICKS.length + 5} scope="colgroup" class="divider">Hidden columns</th></tr>{/if}
          {@const off = busy || kept.has(c)}
          {@const p = pickOf(c)}
          {@const n = countOf(c)}
          <tr class:off={kept.has(c)}>
            <th scope="row">
              <label><input type="checkbox" checked={cols.includes(c)} disabled={off} onchange={(e) => tickColumn(c, (e.currentTarget as HTMLInputElement).checked)} /> {titles[c]}{kept.has(c) ? " (protected)" : ""}</label>
            </th>
            {#each TICKS as t (t.id)}
              <td><input type="checkbox" aria-label={`${t.label}: ${titles[c]}`} checked={p.ticks.includes(t.id)} disabled={off} onchange={(e) => tickCell(c, t.id, (e.currentTarget as HTMLInputElement).checked)} /></td>
            {/each}
            <td>
              <select
                aria-label={`Case: ${titles[c]}`}
                value={p.case}
                disabled={off}
                onchange={(e) => {
                  setPick(c, { case: (e.currentTarget as HTMLSelectElement).value as CasePick });
                  if (!cols.includes(c)) tickColumn(c, true);
                }}
              >
                <option value="">No change</option>
                <option value="upper">UPPER</option>
                <option value="lower">lower</option>
                <option value="title">Proper</option>
              </select>
            </td>
            <td class="saved">
              {#if p.saved}
                <span title={p.saved.file}>{p.saved.file}</span>
                <button type="button" disabled={off} aria-label={`Remove the saved cleaner from ${titles[c]}`} onclick={() => setPick(c, { saved: null })}>×</button>
              {:else}
                <button type="button" disabled={off} aria-label={`Choose a saved cleaner for ${titles[c]}`} onclick={() => void chooseCleaner(c)}>Choose…</button>
              {/if}
            </td>
            <td>
              <input
                type="checkbox"
                aria-label={`Dates as yyyy-mm-dd: ${titles[c]}`}
                title={words.has(c) ? "A Text or Id column is never read as dates." : undefined}
                checked={p.dates}
                disabled={off || words.has(c)}
                onchange={(e) => {
                  setPick(c, { dates: (e.currentTarget as HTMLInputElement).checked });
                  if (!cols.includes(c)) tickColumn(c, true);
                }}
              />
            </td>
            <td class="count">
              {#if n}
                {n.changed.toLocaleString()}
                {#if n.protected}<small>{n.protected.toLocaleString()} protected, left alone</small>{/if}
                {#if n.ambiguous_left}<small>{n.ambiguous_left.toLocaleString()} readable two ways, left as typed</small>{/if}
                {#if n.left > n.ambiguous_left}<small>{(n.left - n.ambiguous_left).toLocaleString()} not changed as dates or numbers, left alone</small>{/if}
                {#if n.homoglyph_cells}<small>{n.homoglyph_cells.toLocaleString()} with look-alike letters</small>{/if}
              {/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
  <p class="muted">{ORDER_TEXT} Dates readable two ways, two-digit years and eight bare digits stay as typed: Data › Check dates handles those. Shown rows only{init.header ? "; the title row is left out" : ""}.</p>
  {#if problem}<p class="warn" role="alert">{problem}</p>{/if}
  {#if idle}<p class="muted" role="note">{idle === 1 ? "1 ticked column has" : `${idle} ticked columns have`} nothing to do and will be left alone.</p>{/if}
  {#if busy}
    <p role="status">Counting… <button type="button" onclick={onCancelRun}>Stop</button></p>
  {:else if preview && stale}
    <p class="warn" role="status">Something changed: press Preview again.</p>
  {:else if preview}
    <section aria-label="Examples">
      {#if preview.examples.length}
        <table class="examples">
          <thead><tr><th>Column</th><th>Before</th><th>After</th></tr></thead>
          <tbody>
            {#each preview.examples as x, i (i)}<tr><td>{titles[x.col]}</td><td><pre>{x.before}</pre></td><td><pre>{x.after}</pre></td></tr>{/each}
          </tbody>
        </table>
      {:else}<p role="status">Nothing would change.</p>{/if}
    </section>
  {/if}
  {#snippet buttons()}
    <button type="button" onclick={close}>Cancel</button>
    <button type="button" onclick={() => void look()} disabled={busy || !spec.entries.length}>Preview</button>
    <button type="button" class="primary" disabled={busy || stale || readOnly || !preview?.columns.some((c) => c.changed)} onclick={() => preview && onApply($state.snapshot(spec) as CleanMatrix, preview.revision)}>Apply</button>
  {/snippet}
</FormShell>

<style>
  .scroll {
    max-height: 260px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 6px;
  }
  table {
    border-collapse: collapse;
    font-size: 12.5px;
    width: 100%;
  }
  th,
  td {
    padding: 3px 5px;
    text-align: center;
    border-bottom: 1px solid var(--border);
  }
  thead th {
    position: sticky;
    top: 0;
    background: var(--surface);
    vertical-align: bottom;
    font-weight: 600;
  }
  thead label {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
  }
  tbody th {
    text-align: left;
    font-weight: normal;
    white-space: nowrap;
  }
  .divider {
    color: var(--muted);
    font-weight: 600;
  }
  .off {
    color: var(--muted);
  }
  .saved span {
    display: inline-block;
    max-width: 110px;
    overflow: hidden;
    text-overflow: ellipsis;
    vertical-align: bottom;
    white-space: nowrap;
  }
  .count small {
    display: block;
    color: var(--muted);
  }
  .examples {
    margin-top: 4px;
  }
  .examples td {
    text-align: left;
  }
  .examples pre {
    margin: 0;
    max-width: 260px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .muted {
    color: var(--muted);
    font-size: 12px;
  }
  .warn {
    color: var(--danger, #c0392b);
  }
</style>
