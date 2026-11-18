<script lang="ts">
  // Column profile: the shapes of a column's values, a small histogram, the
  // shortest and longest values, and odd values. Every line can show its
  // rows (as Filter by formula does, on the rows shown now).
  import { onMount } from "svelte";
  import { backend, errorText, type Profile } from "./backend";
  import FormShell from "./FormShell.svelte";
  import { binExpr, binLabel, outlierExpr, shapeExpr } from "./profile";

  interface Props {
    doc: number;
    col: number;
    title: string;
    /** How the column is written in an expression, like [ZIP]. */
    colRef: string;
    header: boolean;
    run: <T>(label: string, f: () => Promise<T>) => Promise<T | null>;
    onShow: (expr: string, what: string) => void;
    onClose: () => void;
  }
  let { doc, col, title, colRef, header, run, onShow, onClose }: Props = $props();

  let p = $state<Profile | null>(null);
  let problem = $state("");

  // Once, when the screen opens (the job's own state must not run it again).
  onMount(() => {
    void run("Profiling the column", () => backend().columnProfile(doc, { col, keep_first: header, read: { decimal_comma: false, date_order: "mdy" } }))
      .then((x) => {
        if (x) p = x;
        else onClose();
      })
      .catch((e: unknown) => (problem = errorText(e)));
  });

  const filled = $derived(p ? p.rows - p.blanks : 0);
  const pct = (n: number): string => (filled ? `${((n * 100) / filled).toFixed(n * 1000 < filled ? 2 : 1)}%` : "");
  const top = $derived(p?.histogram ? Math.max(1, ...p.histogram.counts) : 1);
  const q = (s: string): string => `"${s.replace(/"/g, '""')}"`;
</script>

<FormShell title={`Profile of ${title}`} wide {onClose}>
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#if p}
    <p class="note">{p.rows.toLocaleString()} shown rows: {filled.toLocaleString()} filled, {p.blanks.toLocaleString()} blank. Click a line to show its rows.</p>
    <div class="cols">
      <section aria-label="Shapes">
        <h3>Shapes <span class="note">(9 a digit, A a letter){p.shape_kinds > p.shapes.length ? `, top ${p.shapes.length} of ${p.shape_kinds.toLocaleString()}` : ""}</span></h3>
        <table>
          <tbody>
            {#each p.shapes as s (s.shape)}
              <tr class="line" onclick={() => onShow(shapeExpr(colRef, s.shape), `shape ${s.shape}`)} title={`For example: ${s.example}`}>
                <td class="mono">{s.shape}</td><td class="num">{s.count.toLocaleString()}</td><td class="num">{pct(s.count)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </section>
      {#if p.histogram}
        {@const h = p.histogram}
        <section aria-label="Histogram">
          <h3>{h.kind === "numbers" ? "Numbers" : h.kind === "dates" ? "Dates" : "Lengths"}</h3>
          <div class="hist">
            {#each h.counts as n, i (i)}
              <button type="button" class="bar" style:height="{Math.max(n ? 3 : 0, (n / top) * 100)}%" title={`${binLabel(h, i)}: ${n.toLocaleString()}`} aria-label={`${binLabel(h, i)}: ${n.toLocaleString()}`} onclick={() => onShow(binExpr(colRef, h, i), binLabel(h, i))}></button>
            {/each}
          </div>
          <p class="note">{binLabel(h, 0)} … {binLabel(h, h.counts.length - 1)}{p.mean !== null && h.kind === "numbers" ? `; average ${p.mean.toLocaleString(undefined, { maximumFractionDigits: 2 })}` : ""}</p>
        </section>
      {/if}
    </div>
    <h3>Shortest and longest</h3>
    <table>
      <tbody>
        {#if p.shortest !== null}<tr class="line" onclick={() => onShow(`LEN(${colRef}) = ${p!.min_len}`, `${p!.min_len} characters`)}><td>Shortest ({p.min_len} characters)</td><td class="mono">{p.shortest}</td></tr>{/if}
        {#if p.longest !== null}<tr class="line" onclick={() => onShow(`LEN(${colRef}) = ${p!.max_len}`, `${p!.max_len} characters`)}><td>Longest ({p.max_len} characters)</td><td class="mono">{p.longest}</td></tr>{/if}
      </tbody>
    </table>
    <h3>Odd values</h3>
    {#if !p.rare_shapes.length && !p.outliers}
      <p class="note">Nothing stands out: no shape seen only a few times{p.mean !== null ? ", and no number far from the rest" : ""}.</p>
    {:else}
      <table>
        <tbody>
          {#each p.rare_shapes as s (s.shape)}
            <tr class="line" onclick={() => onShow(shapeExpr(colRef, s.shape), `shape ${s.shape}`)}>
              <td>Shape seen {s.count === 1 ? "once" : `${s.count} times`}</td><td class="mono">{s.shape}</td><td class="mono">{s.example}</td>
            </tr>
          {/each}
          {#if p.outliers}
            {@const e = outlierExpr(colRef, p)}
            <tr class="line" onclick={() => e && onShow(e, "numbers far from the rest")}>
              <td>{p.outliers.toLocaleString()} {p.outliers === 1 ? "number" : "numbers"} far from the rest</td><td class="mono" colspan="2">{p.outlier_examples.join(", ")}</td>
            </tr>
          {/if}
        </tbody>
      </table>
    {/if}
    {#if p.blanks}
      <p><button type="button" class="link" onclick={() => onShow(`ISBLANK(${colRef})`, "blank")}>Show the {p.blanks.toLocaleString()} blank {p.blanks === 1 ? "row" : "rows"}</button> <button type="button" class="link" onclick={() => onShow(`TRIM(${colRef}) = ${q("")}`, "blank or spaces")}>with spaces too</button></p>
    {/if}
  {/if}
  {#snippet buttons()}
    <button type="button" class="primary" onclick={onClose}>Close</button>
  {/snippet}
</FormShell>

<style>
  .cols {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
  }
  @media (max-width: 700px) {
    .cols {
      grid-template-columns: 1fr;
    }
  }
  h3 {
    font-size: 13px;
    margin: 10px 0 4px;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  td {
    padding: 2px 6px;
    border-bottom: 1px solid var(--border);
  }
  tr.line {
    cursor: pointer;
  }
  tr.line:hover {
    background: var(--hover, rgb(0 0 0 / 0.05));
  }
  .num {
    text-align: right;
    color: var(--muted);
  }
  .mono {
    font-family: ui-monospace, Consolas, monospace;
    max-width: 240px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hist {
    display: flex;
    align-items: flex-end;
    gap: 2px;
    height: 120px;
    border-bottom: 1px solid var(--border);
  }
  .hist .bar.bar {
    flex: 1;
    padding: 0;
    min-width: 4px;
    border: 0;
    border-radius: 2px 2px 0 0;
    background: var(--accent);
    cursor: pointer;
    box-shadow: none;
  }
  .hist .bar.bar:hover {
    opacity: 0.75;
  }
  button.link {
    background: none;
    border: 0;
    padding: 0;
    color: var(--accent, #3b82f6);
    cursor: pointer;
    text-decoration: underline;
  }
</style>
