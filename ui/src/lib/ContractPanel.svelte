<script lang="ts">
  // C9: Review, what a file holds against a file contract. Every count with rows behind it shows those rows
  // (a filter chip). Fix what I can renames and orders the columns (one Ctrl+Z); the other buttons change the
  // contract and save it only when pressed.
  import { untrack } from "svelte";
  import type { Contract, DriftReport, DriftRows } from "./backend";
  import { acceptValues, addAlias, blankPoints, updateBaseline } from "./contractText";
  import Icon from "./Icon.svelte";

  interface Props {
    report: DriftReport;
    contract: Contract;
    /** Why the contract cannot be changed from this tab ("" when it can). */
    lockedWhy: string;
    onShow: (what: DriftRows, words: string) => void;
    onFix: () => void;
    onSave: (c: Contract, what: string) => void;
    onClose: () => void;
  }
  let { report, contract, lockedWhy, onShow, onFix, onSave, onClose }: Props = $props();
  let points = $state(untrack(() => contract.blank_points));

  const placeRenamed = $derived(report.renamed.filter((r) => !r.by_alias));
  const newCols = $derived(report.columns.filter((c) => c.new_values > 0));
  const typeCols = $derived(report.columns.filter((c) => c.type_misses > 0));
  const blankCols = $derived(report.columns.filter((c) => c.blank_jump && c.baseline !== null));
  const broken = $derived(report.rules.filter((r) => r.failing > 0));
  const failedTable = $derived(report.table.filter((t) => t.ok === false));
  const canFix = $derived(report.renamed.length > 0 || report.moved.length > 0 || report.added.length > 0 || report.missing.length < contract.columns.length);
  const pct = (x: number) => `${Math.round(x * 100)}%`;
  const n = (k: number) => k.toLocaleString();

  function accept(): void {
    const { contract: c, full } = acceptValues(contract, report, newCols.map((x) => x.col));
    onSave(c, full.length ? `Accepted the new values (${full.join(", ")} would pass 500 values, so ${full.length === 1 ? "it was" : "they were"} left as ${full.length === 1 ? "it was" : "they were"}).` : "Accepted the new values.");
  }
  function savePoints(): void {
    const v = Math.min(100, Math.max(0, typeof points === "number" && Number.isFinite(points) ? points : 0));
    onSave({ ...contract, blank_points: v }, `Blank shares are reported when they move more than ${v} points.`);
  }
</script>

<aside class="contractpanel" aria-label="Review against {report.contract}" data-testid="contract-panel">
  <header>
    <h2>Against {report.contract}</h2>
    <button type="button" class="close" aria-label="Close" onclick={onClose}><Icon name="close" size={12} /></button>
  </header>
  <p class="muted">All {n(report.rows)} data rows were counted, hidden ones too.</p>

  <section>
    <h3>Columns</h3>
    {#if !report.renamed.length && !report.missing.length && !report.added.length && !report.moved.length}
      <p class="muted">As the contract has them.</p>
    {/if}
    <ul>
      {#each placeRenamed as r (r.col)}
        <li>
          <b>{r.from}</b> is probably <b>{r.to}</b> (renamed: it sits where {r.to} was).
          <button type="button" class="link" disabled={!!lockedWhy} title={lockedWhy || `Keep ${r.from} as another title of ${r.to}`} onclick={() => onSave(addAlias(contract, r.col, r.from), `${r.from} is now another title of ${r.to}.`)}>Add this title as an alias</button>
        </li>
      {/each}
      {#each report.renamed.filter((r) => r.by_alias) as r (r.col)}
        <li class="muted">{r.from} is {r.to} (another title the contract accepts).</li>
      {/each}
      {#each report.missing as t (t)}<li>Missing: <b>{t}</b></li>{/each}
      {#each report.added as t (t)}<li>New: <b>{t}</b></li>{/each}
      {#if report.moved.length}<li>Moved: {report.moved.join(", ")}</li>{/if}
    </ul>
  </section>

  {#if typeCols.length}
    <section>
      <h3>Types</h3>
      <ul>
        {#each typeCols as c (c.col)}
          <li><button type="button" class="link" onclick={() => onShow({ kind: "type_misses", col: c.col }, `cells in ${c.title} not of its type`)}>{n(c.type_misses)} {c.type_misses === 1 ? "cell" : "cells"}</button> in {c.title} {c.type_misses === 1 ? "is" : "are"} not of its type</li>
        {/each}
      </ul>
    </section>
  {/if}

  {#if broken.length || failedTable.length || report.not_run.length}
    <section>
      <h3>Checks</h3>
      <ul>
        {#each broken as r (r.check)}
          <li><button type="button" class="link" onclick={() => onShow({ kind: "rule", check: r.check }, `rows breaking ${r.name}`)}>{n(r.failing)} {r.failing === 1 ? "row" : "rows"}</button> break {r.name}</li>
        {/each}
        {#each failedTable as t (t.check)}<li>{t.name}: {t.actual} (wants {t.expected})</li>{/each}
        {#each report.not_run as x (x.check)}<li class="muted">Rule {x.check + 1} did not run: {x.why}</li>{/each}
      </ul>
    </section>
  {/if}

  {#if newCols.length}
    <section>
      <h3>New values</h3>
      <ul>
        {#each newCols as c (c.col)}
          <li>
            <button type="button" class="link" onclick={() => onShow({ kind: "new_values", col: c.col }, `new values in ${c.title}`)}>{n(c.new_values)} {c.new_values === 1 ? "cell" : "cells"}</button>
            in {c.title}: {c.first_new.join(", ")}{c.first_new.length >= 5 ? ", …" : ""}
          </li>
        {/each}
      </ul>
      <button type="button" disabled={!!lockedWhy} title={lockedWhy || "Add the values listed to the allowed lists (check again for any not listed)"} onclick={accept}>Accept these new values</button>
    </section>
  {/if}

  <section>
    <h3>Blank cells</h3>
    {#if blankCols.length}
      <ul>
        {#each blankCols as c (c.col)}
          {@const p = blankPoints(c.blank_rate, c.baseline ?? 0)}
          <li><button type="button" class="link" onclick={() => onShow({ kind: "blanks", col: c.col }, `blanks in ${c.title}`)}>{n(c.blanks)} blank {c.blanks === 1 ? "cell" : "cells"}</button> in {c.title}: {pct(c.blank_rate)}, {p >= 0 ? "up" : "down"} {Math.abs(p)} points from {pct(c.baseline ?? 0)}</li>
        {/each}
      </ul>
    {:else}
      <p class="muted">No share of blanks moved more than {contract.blank_points} points.</p>
    {/if}
    <label class="row">Report a move of more than <input type="number" min="0" max="100" step="1" bind:value={points} aria-label="Points a share of blanks may move" /> points
      <button type="button" disabled={!!lockedWhy || points === contract.blank_points} title={lockedWhy || "Keep this in the contract"} onclick={savePoints}>Save</button></label>
  </section>

  <section>
    <h3>Rows</h3>
    <p>
      {n(report.rows)} data rows{#if report.row_range}; the contract expects {n(report.row_range.min)} to {n(report.row_range.max)}{report.rows_out ? " (outside)" : ""}{/if}.
    </p>
    <button type="button" disabled={!!lockedWhy} title={lockedWhy || "Take today's blank shares and row count into the contract"} onclick={() => onSave(updateBaseline(contract, report), "The contract's baseline is now this file's blank shares and row count.")}>Update the baseline</button>
  </section>

  <footer>
    <button type="button" class="primary" disabled={!canFix} title={canFix ? "Rename the columns to the contract's titles, put them in its order and set its types and checks (one Ctrl+Z)" : "Nothing in this file's columns pairs with the contract"} onclick={onFix}>Fix what I can</button>
    {#if lockedWhy}<p class="muted">{lockedWhy}.</p>{/if}
  </footer>
</aside>

<style>
  .contractpanel {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    z-index: 15;
    width: min(380px, 90vw);
    overflow: auto;
    padding: 10px 14px;
    border-left: 1px solid var(--border);
    background: var(--bg);
    box-shadow: -4px 0 16px rgb(0 0 0 / 0.12);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  h2 {
    margin: 0;
    font-size: 1.05em;
  }
  h3 {
    margin: 12px 0 4px;
    font-size: 0.95em;
  }
  ul {
    margin: 0;
    padding-left: 18px;
  }
  li {
    margin: 3px 0;
  }
  .muted {
    opacity: 0.75;
  }
  button {
    padding: 3px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    color: inherit;
    font: inherit;
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  button.link {
    padding: 0;
    border: none;
    color: var(--accent);
    text-decoration: underline;
  }
  button.primary {
    border-color: var(--accent);
  }
  .close {
    padding: 3px 6px;
  }
  .row input {
    width: 4em;
  }
  footer {
    margin-top: 14px;
  }
</style>
