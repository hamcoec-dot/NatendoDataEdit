<script lang="ts">
  // The filter panel: one card per column, each showing what picking in it
  // would leave given every other card (live counts). A card's picks are
  // the tab's filter rule on that column: each click is one filter change
  // (one undo step), and the filter menus show the same rules.
  import { backend, errorText, type Facet, type Facets, type ReadOptions, type Rule } from "./backend";
  import { onDestroy } from "svelte";
  import type { Tab } from "./tab.svelte";
  import { ruleFor } from "./filters";
  import { barEdges, blankRule, dayText, FACET_BINS, pickedRange, pickedValues, rangeRule, toggleValue } from "./facets";

  interface Props {
    tab: Tab;
    titles: string[];
    read: ReadOptions;
    onSetRule: (col: number, rule: Rule | null) => Promise<void>;
  }
  let { tab, titles, read, onSetRule }: Props = $props();

  let facets = $state<Facets | null>(null);
  let problem = $state("");
  let searches = $state<Record<number, string>>({});
  /** Values shown per card before "more". */
  let showAll = $state<Record<number, boolean>>({});
  let froms = $state<Record<number, string>>({});
  let tos = $state<Record<number, string>>({});

  // Count again whenever the cards, the filter or the data change (a newer
  // count cancels an older one in the engine, and so does an edit). After
  // an edit alone the count waits a second, so typing never waits for it;
  // a pick counts at once. The tab's version also moves when a pick is
  // applied, which changes neither: that run leaves the count it started.
  let seq = 0;
  let lastAsk = "";
  let lastData: number | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const t = tab;
    void t.version;
    const cards = t.facetCards.map((c) => ({ col: c.col, kind: c.kind, range: c.range, search: searches[c.col]?.trim() || null }));
    const spec = { columns: $state.snapshot(t.filter?.columns ?? []), keep_first: t.header, read: $state.snapshot(read) as ReadOptions, combine: t.filter?.combine ?? "and", invert: !!t.filter?.invert };
    const data = t.info.data_revision;
    if (!cards.length || !t.info.indexed) {
      clearTimeout(timer);
      facets = null;
      lastAsk = "";
      if (!cards.length) void backend().facetForget(t.id).catch(() => {});
      return;
    }
    const ask = JSON.stringify([t.id, cards, spec]);
    if (ask === lastAsk && data === lastData) return;
    const editOnly = ask === lastAsk;
    lastAsk = ask;
    lastData = data;
    const mine = ++seq;
    clearTimeout(timer);
    timer = setTimeout(() => {
      backend()
        .facetCounts(t.id, cards, spec as never)
        .then((f) => {
          if (mine !== seq) return;
          facets = f;
          problem = "";
          // Keep each card's range, so its bars stay put.
          f.cards.forEach((c, i) => {
            const card = t.facetCards[i];
            if (card && !card.range && c.range) card.range = c.range;
          });
        })
        .catch((e: unknown) => {
          const kind = (e as { kind?: string }).kind;
          if (mine === seq && kind !== "cancelled" && !/still working/.test(errorText(e))) problem = errorText(e);
        });
    }, editOnly ? 1000 : 120);
  });
  // Closing the panel frees what it worked out.
  onDestroy(() => {
    clearTimeout(timer);
    void backend().facetForget(tab.id).catch(() => {});
  });

  let adding = $state("");
  async function addCard(v: string): Promise<void> {
    const col = Number(v);
    if (v === "" || tab.facetCards.some((c) => c.col === col)) return;
    let kind: "values" | "numbers" | "dates" = "values";
    try {
      const types = await backend().columnTypes(tab.id, tab.header, read);
      kind = types[col] === "number" ? "numbers" : types[col] === "date" ? "dates" : "values";
    } catch {
      // Busy: a value list works for any column.
    }
    tab.facetCards = [...tab.facetCards, { col, kind, range: null }];
  }
  function removeCard(col: number): void {
    tab.facetCards = tab.facetCards.filter((c) => c.col !== col);
  }
  function switchKind(col: number): void {
    tab.facetCards = tab.facetCards.map((c) => (c.col === col ? { ...c, kind: c.kind === "values" ? "numbers" : "values", range: null } : c));
  }

  const facetOf = (col: number): Facet | undefined => facets?.cards.find((c) => c.col === col);
  const ruleOf = (col: number): Rule | null => ruleFor(tab.filter, col);
  /** The column's chip (C8): switched off, or inverted ("not"). */
  const chipOf = (col: number) => tab.filter?.columns.find((c) => c.col === col);
  /** Match any or Invert all: a card counts every row with each value (what a pick there would add). */
  const anyCounts = $derived(tab.filter?.combine === "or" || !!tab.filter?.invert);
  const title = (col: number): string => titles[col] || `Column ${col + 1}`;

  function applyRange(col: number, kind: "numbers" | "dates"): void {
    const parse = (s: string): number | null => {
      if (!s.trim()) return null;
      if (kind === "dates") {
        const t = Date.parse(s.trim());
        return Number.isNaN(t) ? null : Math.floor(t / 86_400_000);
      }
      const n = Number(s.replace(/[$,]/g, ""));
      return Number.isNaN(n) ? null : n;
    };
    const card = tab.facetCards.find((c) => c.col === col);
    const lo = parse(froms[col] ?? "") ?? card?.range?.[0] ?? null;
    const hi = parse(tos[col] ?? "") ?? card?.range?.[1] ?? null;
    if (lo === null || hi === null) return;
    void onSetRule(col, rangeRule(kind, Math.min(lo, hi), Math.max(lo, hi)));
  }
  function pickBar(col: number, kind: "numbers" | "dates", range: [number, number], i: number): void {
    const [a, b] = barEdges(range, i);
    froms[col] = kind === "dates" ? dayText(a) : String(Math.round(a * 100) / 100);
    tos[col] = kind === "dates" ? dayText(b) : String(Math.round(b * 100) / 100);
    void onSetRule(col, rangeRule(kind, a, b));
  }
  const fmt = (x: number, kind: string): string => (kind === "dates" ? dayText(x) : x.toLocaleString(undefined, { maximumFractionDigits: 2 }));
</script>

<div class="fp">
  <p class="note" data-testid="panel-counts-note">{facets ? `${facets.shown.toLocaleString()} rows show.` : ""} {anyCounts ? (tab.filter?.invert ? "Invert all is on: each card counts every row with that value, before the chips are inverted." : "Match any is on: each card counts every row with that value (a pick adds those rows).") : "Each card counts what the other cards leave."}</p>
  <label class="row add">Add a column
    <select bind:value={adding} onchange={() => { void addCard(adding); adding = ""; }}>
      <option value="">Choose…</option>
      {#each titles as t, i (i)}{#if !tab.facetCards.some((c) => c.col === i)}<option value={String(i)}>{t || `Column ${i + 1}`}</option>{/if}{/each}
    </select>
  </label>
  {#if tab.info.filtered && !(tab.info.filter_cols?.length)}
    <p class="note formula" role="status">Rows are picked by a tool (a filter chip such as Filter by formula or Show these rows); the cards count the rows it lets through, and a pick here joins it as a chip of its own.</p>
  {/if}
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#each tab.facetCards as card (card.col)}
    {@const f = facetOf(card.col)}
    {@const rule = ruleOf(card.col)}
    {@const chip = chipOf(card.col)}
    <section class="card" aria-label={`Filter card for ${title(card.col)}`}>
      <header>
        <strong>{chip?.negate ? "not " : ""}{title(card.col)}</strong>
        {#if chip && chip.enabled === false}<span class="off" title="Its chip is switched off: it lets every row through until it is switched on in the chips bar">(switched off)</span>{/if}
        {#if rule}<button type="button" class="link" onclick={() => void onSetRule(card.col, null)}>Clear</button>{/if}
        <span class="spacer"></span>
        <button type="button" class="tiny" title={card.kind === "values" ? "Show as a range" : "Show as a list of values"} onclick={() => switchKind(card.col)}>{card.kind === "values" ? "Range" : "List"}</button>
        <button type="button" class="tiny" aria-label={`Remove the ${title(card.col)} card`} onclick={() => removeCard(card.col)}>✕</button>
      </header>
      {#if !f}
        <p class="note">Counting…</p>
      {:else if card.kind === "values"}
        {@const picked = pickedValues(rule)}
        <input type="search" placeholder="Search values" value={searches[card.col] ?? ""} oninput={(e) => (searches[card.col] = (e.currentTarget as HTMLInputElement).value)} />
        {@const top = Math.max(1, ...f.values.slice(0, 50).map((v) => v.count), f.blanks)}
        <ul class="vals">
          {#each showAll[card.col] ? f.values : f.values.slice(0, 50) as v (v.value)}
            <li>
              <label>
                <input type="checkbox" checked={picked ? picked.values.has(v.value) : false} onchange={(e) => void onSetRule(card.col, toggleValue(rule, v.value, (e.currentTarget as HTMLInputElement).checked))} />
                <span class="v" title={v.value}>{v.value}</span>
                <span class="bar" style:width="{(v.count / top) * 60}px"></span>
                <span class="n">{v.count.toLocaleString()}</span>
              </label>
            </li>
          {/each}
          {#if f.blanks}
            <li>
              <label>
                <input type="checkbox" checked={picked?.blanks ?? false} onchange={(e) => void onSetRule(card.col, toggleValue(rule, "", (e.currentTarget as HTMLInputElement).checked))} />
                <span class="v muted">(blank)</span>
                <span class="bar" style:width="{(f.blanks / top) * 60}px"></span>
                <span class="n">{f.blanks.toLocaleString()}</span>
              </label>
            </li>
          {/if}
        </ul>
        {#if f.values.length > 50 && !showAll[card.col]}<button type="button" class="link" onclick={() => (showAll[card.col] = true)}>Show {f.values.length - 50} more</button>{/if}
        {#if f.many}<p class="note many" role="status">{searches[card.col]?.trim() ? "Too many values match; type more of the value." : "Nearly every value here is different, so the card does not list them (that keeps every pick quick); search to pick."}</p>
        {:else if !f.complete}<p class="note">The {f.values.length.toLocaleString()} most common of {f.distinct >= 10_000 ? "over 10,000" : f.distinct.toLocaleString()} values; search for others.</p>{/if}
      {:else}
        {@const kind = card.kind}
        {@const range = f.range}
        {@const pr = pickedRange(rule, kind)}
        {#if range}
          {@const top = Math.max(1, ...f.bins)}
          <div class="hist" role="group" aria-label="Bars: click one to show its rows">
            {#each f.bins as n, i (i)}
              {@const [a, b] = barEdges(range, i)}
              <button type="button" class="hb" style:height="{Math.max(n ? 3 : 0, (n / top) * 100)}%" title={`${fmt(a, kind)} to ${fmt(b, kind)}: ${n.toLocaleString()}`} aria-label={`${fmt(a, kind)} to ${fmt(b, kind)}: ${n.toLocaleString()}`} onclick={() => pickBar(card.col, kind, range, i)}></button>
            {/each}
          </div>
          <p class="note">{fmt(range[0], kind)} … {fmt(range[1], kind)}{f.other ? `; ${f.other.toLocaleString()} not ${kind === "dates" ? "dates" : "numbers"}` : ""}{f.blanks ? `; ${f.blanks.toLocaleString()} blank` : ""}</p>
        {:else}
          <p class="note">No {kind === "dates" ? "dates" : "numbers"} here.</p>
        {/if}
        <div class="row range">
          <input type="text" aria-label="From" placeholder={pr?.[0] ?? (range ? fmt(range[0], kind) : "from")} bind:value={froms[card.col]} onkeydown={(e) => e.key === "Enter" && applyRange(card.col, kind)} />
          <span>to</span>
          <input type="text" aria-label="To" placeholder={pr?.[1] ?? (range ? fmt(range[1], kind) : "to")} bind:value={tos[card.col]} onkeydown={(e) => e.key === "Enter" && applyRange(card.col, kind)} />
          <button type="button" class="tiny" onclick={() => applyRange(card.col, kind)}>Show</button>
        </div>
        <div class="row">
          <button type="button" class="link" onclick={() => void onSetRule(card.col, blankRule(true))}>Blank only</button>
          <button type="button" class="link" onclick={() => void onSetRule(card.col, blankRule(false))}>Not blank</button>
        </div>
      {/if}
    </section>
  {/each}
  {#if !tab.facetCards.length}
    <p class="note">Add a column to filter by its values or a range, with counts that follow every pick. <span class="muted">({FACET_BINS} bars for numbers and dates.)</span></p>
  {/if}
</div>

<style>
  .off {
    color: var(--muted, #666);
    font-size: 11px;
    margin-left: 4px;
  }
  .fp {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .card {
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 6px 8px;
    background: var(--surface);
  }
  .card header {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 4px;
  }
  .spacer {
    flex: 1;
  }
  .vals {
    list-style: none;
    margin: 4px 0;
    padding: 0;
    max-height: 260px;
    overflow-y: auto;
  }
  .vals label {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
  }
  .v {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .bar {
    height: 8px;
    border-radius: 2px;
    background: var(--accent);
    opacity: 0.55;
  }
  .n {
    color: var(--muted);
    min-width: 46px;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .hist {
    display: flex;
    align-items: flex-end;
    gap: 1px;
    height: 70px;
    border-bottom: 1px solid var(--border);
  }
  .hist .hb.hb {
    flex: 1;
    padding: 0;
    min-width: 3px;
    border: 0;
    border-radius: 2px 2px 0 0;
    background: var(--accent);
    box-shadow: none;
    cursor: pointer;
  }
  .range input {
    width: 90px;
  }
  .row {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  button.link {
    background: none;
    border: 0;
    padding: 0;
    color: var(--accent);
    cursor: pointer;
    text-decoration: underline;
    font-size: 12px;
  }
  button.tiny {
    font-size: 11px;
    padding: 0 6px;
  }
  .muted {
    color: var(--muted);
  }
  input[type="search"] {
    width: 100%;
  }
</style>
