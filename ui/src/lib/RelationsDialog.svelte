<script lang="ts">
  // Data › Find relationships: pick columns, see which settle which and which
  // are blank together, then look at the rows that break a line.
  import { tick as drawn, untrack } from "svelte";
  import AllNone from "./AllNone.svelte";
  import FormShell from "./FormShell.svelte";
  import { breaks, cautions, copyText, findingText, MAX_RELATION_COLS, type Finding, type RelationsReport, type RelationsSpec } from "./relations";

  interface Props {
    titles: string[];
    hidden: number[];
    header: boolean;
    onRun: (spec: RelationsSpec) => Promise<RelationsReport | null>;
    onCancelRun: () => void;
    /** False when it was refused (the screen stays). */
    onExceptions: (f: Finding) => Promise<boolean>;
    /** Adds the finding's data checks; false when nothing was added. */
    onMakeCheck: (f: Finding) => Promise<boolean>;
    onCopy: (text: string) => void;
    onClose: () => void;
  }
  let { titles, hidden, header, onRun, onCancelRun, onExceptions, onMakeCheck, onCopy, onClose }: Props = $props();
  const init = untrack(() => ({ header }));
  let cols = $state<number[]>([]);
  let percent = $state(95);
  let busy = $state(false);
  let report = $state<RelationsReport | null>(null);
  let ran = $state("");
  let findButton = $state<HTMLButtonElement>();
  /** Findings (by line) made into checks since the last Find. */
  let made = $state<number[]>([]);

  const title = (c: number) => titles[c] ?? `Column ${c + 1}`;
  const shown = $derived(titles.map((t, i) => [i, t] as const).filter(([i]) => !hidden.includes(i)));
  const spec = $derived<RelationsSpec>({ cols: [...cols].sort((a, b) => a - b), header: init.header, min_percent: Number(percent) || 0, seconds: 20 });
  const stale = $derived(!report || ran !== JSON.stringify(spec));
  const ready = $derived(cols.length >= 2 && cols.length <= MAX_RELATION_COLS && percent >= 80 && percent <= 100);

  function tick(c: number, on: boolean): void {
    cols = on ? [...cols, c].slice(0, MAX_RELATION_COLS) : cols.filter((x) => x !== c);
  }
  async function find(): Promise<void> {
    if (busy || !ready) return;
    const s = $state.snapshot(spec) as RelationsSpec;
    busy = true;
    try {
      const r = await onRun(s);
      if (!r) return;
      report = r;
      made = [];
      ran = JSON.stringify(s);
    } finally {
      busy = false;
    }
  }
  async function exceptions(f: Finding): Promise<void> {
    if (!(await onExceptions($state.snapshot(f) as Finding))) {
      report = null;
      void drawn().then(() => findButton?.focus());
    }
  }
  async function makeCheck(f: Finding, i: number): Promise<void> {
    if (made.includes(i)) return;
    if (await onMakeCheck($state.snapshot(f) as Finding)) made = [...made, i];
  }
  function close(): void {
    if (busy) onCancelRun();
    onClose();
  }
</script>

<FormShell title="Find relationships" wide onClose={close} onEnter={() => void find()}>
  <p class="muted">Pick 2 to {MAX_RELATION_COLS} columns. Shown rows only{init.header ? "; the title row is left out" : ""}. Values are compared exactly (spaces at the ends aside), so TN and tn differ: clean first if capitals should not count.</p>
  <AllNone count={titles.length} picked={cols} max={MAX_RELATION_COLS} skip={(c) => hidden.includes(c)} onChange={(v) => (cols = v)} />
  <div class="cols">
    {#each shown as [i, t] (i)}
      <label><input type="checkbox" checked={cols.includes(i)} disabled={busy} onchange={(e) => tick(i, (e.currentTarget as HTMLInputElement).checked)} /> {t}</label>
    {/each}
  </div>
  <label>Show what holds in at least <input type="number" min="80" max="100" step="0.5" bind:value={percent} disabled={busy} /> percent</label>

  {#if busy}
    <p role="status">Looking for relationships… <button type="button" onclick={onCancelRun}>Stop</button></p>
  {:else if report && stale}
    <p class="warn" role="status">Something changed: press Find again.</p>
  {:else if report}
    {@const r = report}
    <section aria-label="Relationships found">
      <p role="status">{r.findings.length ? `${r.findings.length.toLocaleString()} found, best first.` : "No relationship holds that well between these columns."}</p>
      {#if r.sampled < r.rows}<p class="muted">Found from a sample of {r.sampled.toLocaleString()} of {r.rows.toLocaleString()} rows; a sorted file may hide some. The best 25 are counted over every row.</p>{/if}
      {#if r.unchecked}<p class="warn">Time ran out with {r.unchecked.toLocaleString()} pairs of columns not looked at: pick fewer columns.</p>{/if}
      <ol class="found">
        {#each r.findings as f, i (i)}
          <li>
            <span>{findingText(f, title)}{#each cautions(f) as c (c)} <em>{c}</em>{/each}</span>
            <button type="button" disabled={!breaks(f)} aria-label={`Show the exceptions to line ${i + 1}`} onclick={() => void exceptions(f)}>Show the exceptions</button>
            <button type="button" disabled={busy || made.includes(i)} aria-label={`Make line ${i + 1} a check`}
              title={f.kind === "one_to_one" ? "Adds two data checks, one each way" : f.kind === "co_blank" ? "Adds a row rule: blank in both or in neither" : "Adds a data check that marks the rows that break it"}
              onclick={() => void makeCheck(f, i)}>{made.includes(i) ? "Added ✓" : "Make this a check"}</button>
          </li>
        {/each}
      </ol>
      {#if r.left_out.length}<p class="muted">Left out: {r.left_out.map(([c, why]) => `${title(c)} (${why === "unique" ? "nearly every row its own value, so it settles everything" : "one value only"})`).join(", ")}.</p>{/if}
      {#if r.mostly_one.length}
        <details>
          <summary>{r.mostly_one.length.toLocaleString()} pairs that tell you little</summary>
          <ul class="found">
            {#each r.mostly_one.slice(0, 50) as [a, b], i (i)}<li>{title(a)} -> {title(b)}: {title(b)} is mostly one value, so this tells you little.</li>{/each}
          </ul>
        </details>
      {/if}
    </section>
  {/if}
  {#snippet buttons()}
    <button type="button" onclick={close}>Close</button>
    <button type="button" disabled={busy || stale || !report?.findings.length} onclick={() => report && onCopy(copyText(report, title))}>Copy</button>
    <button type="button" class="primary" bind:this={findButton} disabled={busy || !ready} onclick={() => void find()}>Find</button>
  {/snippet}
</FormShell>

<style>
  .cols {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 14px;
    max-height: 120px;
    overflow: auto;
    margin-bottom: 8px;
  }
  input[type="number"] {
    width: 70px;
  }
  .found {
    max-height: 230px;
    overflow: auto;
    font-size: 13px;
    margin: 6px 0;
  }
  .found li {
    margin-bottom: 6px;
  }
  .found li span {
    display: block;
  }
  em {
    color: var(--muted);
    margin-left: 6px;
  }
  .muted {
    color: var(--muted);
    font-size: 12px;
  }
  .warn {
    color: var(--danger, #c0392b);
  }
</style>
