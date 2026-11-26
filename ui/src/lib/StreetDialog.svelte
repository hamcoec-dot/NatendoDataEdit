<script lang="ts">
  // Street range lookup (Milestone 15): the precinct, district or other
  // value for each address, from a street index (street, from and to house
  // numbers, odd/even side) open in another tab.
  import OtherHidden from "./OtherHidden.svelte";
  import AllNone from "./AllNone.svelte";
  import { untrack } from "svelte";
  import FormShell from "./FormShell.svelte";
  import type { OtherTable } from "./transfer";
  import { guessIndexCols, type IndexCols, type StreetSpec } from "./streets";

  interface Props {
    doc: number;
    titles: string[];
    header: boolean;
    col: number;
    others: OtherTable[];
    titlesOf: (id: number, header: boolean) => Promise<string[]>;
    onOpen: () => Promise<OtherTable | null>;
    onRun: (other: number, cols: IndexCols, spec: StreetSpec) => void;
    onClose: () => void;
  }
  let { doc, titles, header, col, others, titlesOf, onOpen, onRun, onClose }: Props = $props();

  let list = $state<OtherTable[]>(untrack(() => others.filter((o) => o.id !== doc)));
  let other = $state<number>(untrack(() => list[0]?.id ?? -1));
  let otherHeader = $state(true);
  let theirs = $state<string[]>([]);
  let cols = $state<IndexCols>({ street: 0, low: 1, high: 2, side: null, area: null, bring: [], header: true });
  let spec = $state<StreetSpec>(untrack(() => ({ address: col, number: null, area: null, keep_first: header, note: true })));
  let split = $state(false);

  $effect(() => {
    const o = other;
    const h = otherHeader;
    if (o < 0) return;
    void titlesOf(o, h).then((t) => {
      theirs = t;
      cols = { ...guessIndexCols(t), header: h };
    });
  });
  async function openFile(): Promise<void> {
    const o = await onOpen();
    if (o) {
      list = [...list.filter((x) => x.id !== o.id), o];
      other = o.id;
    }
  }
  const ready = $derived(other >= 0 && cols.bring.length > 0);
  const opt = (v: string): number | null => (v === "" ? null : Number(v));
</script>

<FormShell title="Look up by street range" wide {onClose}>
  <p class="note">For each address, finds the row of a street index whose street matches and whose house numbers (from–to, on the odd, even or both sides) hold the address's number, and adds that row's columns, such as the precinct or district.</p>
  {#if !list.length}
    <p>Open the street index first (a file with the street, the from and to house numbers, and the columns to bring in).</p>
  {/if}
  <label class="row">Street index
    <select bind:value={other}>{#each list as o (o.id)}<option value={o.id}>{o.name}</option>{/each}</select>
    <button type="button" onclick={() => void openFile()}>Open a file…</button>
  </label>
  {#if other >= 0}
    <label class="row"><input type="checkbox" bind:checked={otherHeader} /> Its first row is titles</label>
    <div class="grid">
      <label>Street <select bind:value={cols.street}>{#each theirs as t, c (c)}<option value={c}>{t}</option>{/each}</select></label>
      <label>From number <select bind:value={cols.low}>{#each theirs as t, c (c)}<option value={c}>{t}</option>{/each}</select></label>
      <label>To number <select bind:value={cols.high}>{#each theirs as t, c (c)}<option value={c}>{t}</option>{/each}</select></label>
      <label>Odd/even side <select value={cols.side ?? ""} onchange={(e) => (cols.side = opt((e.currentTarget as HTMLSelectElement).value))}><option value="">(both sides)</option>{#each theirs as t, c (c)}<option value={c}>{t}</option>{/each}</select></label>
      <label>ZIP or city <select value={cols.area ?? ""} onchange={(e) => (cols.area = opt((e.currentTarget as HTMLSelectElement).value))}><option value="">(none)</option>{#each theirs as t, c (c)}<option value={c}>{t}</option>{/each}</select></label>
    </div>
    <fieldset>
      <legend>Bring in</legend>
      <AllNone count={theirs.length} picked={cols.bring} onChange={(v) => (cols.bring = v)} max={32} />
      <div class="bring">
        {#each theirs as t, c (c)}
          <label><input type="checkbox" checked={cols.bring.includes(c)} onchange={(e) => { cols.bring = (e.currentTarget as HTMLInputElement).checked ? [...cols.bring, c].slice(0, 32) : cols.bring.filter((x) => x !== c); }} /> {t}</label>
        {/each}
      </div>
    </fieldset>
    <fieldset>
      <legend>This table's addresses</legend>
      <label class="row"><input type="checkbox" bind:checked={split} onchange={() => { if (!split) spec.number = null; }} /> The house number is in its own column</label>
      <div class="grid">
        {#if split}
          <label>House number <select value={spec.number ?? ""} onchange={(e) => (spec.number = opt((e.currentTarget as HTMLSelectElement).value))}><option value="">(choose)</option>{#each titles as t, c (c)}<option value={c}>{t}</option>{/each}</select></label>
        {/if}
        <label>{split ? "Street" : "Address"} <select bind:value={spec.address}>{#each titles as t, c (c)}<option value={c}>{t}</option>{/each}</select></label>
        {#if cols.area !== null}
          <label>ZIP or city <select value={spec.area ?? ""} onchange={(e) => (spec.area = opt((e.currentTarget as HTMLSelectElement).value))}><option value="">(choose)</option>{#each titles as t, c (c)}<option value={c}>{t}</option>{/each}</select></label>
        {/if}
      </div>
      <label class="row"><input type="checkbox" bind:checked={spec.keep_first} /> This table's first row is titles</label>
      <label class="row"><input type="checkbox" bind:checked={spec.note} /> Add a "Lookup note" column saying why a row found nothing</label>
    </fieldset>
  {/if}
  <OtherHidden table={list.find((o) => o.id === other)} />
  {#snippet buttons()}
    <button type="button" class="primary" data-enter disabled={!ready || (split && spec.number === null) || (cols.area !== null && spec.area === null)} onclick={() => onRun(other, $state.snapshot(cols) as IndexCols, $state.snapshot(spec) as StreetSpec)}>Look up</button>
    <button type="button" onclick={onClose}>Close</button>
  {/snippet}
</FormShell>

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(14em, 1fr));
    gap: 6px;
  }
  .grid label {
    display: flex;
    flex-direction: column;
  }
  .bring {
    max-height: 140px;
    overflow: auto;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(12em, 1fr));
  }
</style>
