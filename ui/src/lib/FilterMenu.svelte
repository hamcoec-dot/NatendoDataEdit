<script lang="ts">
  // A column's filter menu: sort buttons, a ticked list of the column's
  // values (with counts and a search box), or one or two conditions.
  import { onMount, untrack } from "svelte";
  import { backend, errorText, type Rule, type ValueList } from "./backend";
  import type { Tab } from "./tab.svelte";
  import { OPS, conditionOf, emptyForm, formOf, ruleFor, withRule, type CondForm, type CondKind } from "./filters";

  interface Props {
    tab: Tab;
    col: number;
    title: string;
    x: number;
    y: number;
    onApply: (rule: Rule | null) => void;
    onSort: (descending: boolean) => void;
    onClose: () => void;
  }
  let { tab, col, title, x, y, onApply, onSort, onClose }: Props = $props();

  // The rule when the menu opened (the menu is rebuilt for another column).
  const current = untrack(() => ruleFor(tab.filter, col));
  let mode = $state<"values" | "condition">(current?.type === "conditions" ? "condition" : "values");
  let list = $state<ValueList | null>(null);
  let loading = $state(true);
  let error = $state("");
  let search = $state("");
  /** Unticked values (so "everything" stays ticked as new values load). */
  let unticked = $state(new Set<string>());
  let blanksTicked = $state(true);
  let first = $state<CondForm>(current?.type === "conditions" ? formOf(current.first) : emptyForm());
  let second = $state<CondForm | null>(current?.type === "conditions" && current.second ? formOf(current.second) : null);
  let or = $state(current?.type === "conditions" ? !!current.or : false);
  let box: HTMLDivElement;
  let searchTimer = 0;

  async function load(text: string | null): Promise<void> {
    loading = true;
    error = "";
    try {
      // Values of the rows the other columns' rules let through.
      const spec = withRule(tab.filter, col, null, tab.header);
      list = await backend().filterValues(tab.id, col, spec, text);
      if (current?.type === "values" && text === null) {
        const keep = new Set(current.values);
        unticked = new Set(list.values.map((v) => v.value).filter((v) => !keep.has(v)));
        blanksTicked = current.blanks;
      }
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    void load(null);
    box.querySelector<HTMLInputElement>("input")?.focus();
    const away = (e: MouseEvent) => {
      if (!box.contains(e.target as Node)) onClose();
    };
    window.addEventListener("mousedown", away);
    return () => window.removeEventListener("mousedown", away);
  });

  const shown = $derived.by(() => {
    if (!list) return [];
    const s = search.trim().toLowerCase();
    return s && list.complete ? list.values.filter((v) => v.value.toLowerCase().includes(s)) : list.values;
  });
  const allTicked = $derived(shown.every((v) => !unticked.has(v.value)) && (search !== "" || blanksTicked));

  function onSearch(): void {
    clearTimeout(searchTimer);
    if (list && !list.complete) searchTimer = window.setTimeout(() => void load(search.trim() || null), 300);
  }

  function toggleAll(): void {
    const next = new Set(unticked);
    for (const v of shown) {
      if (allTicked) next.add(v.value);
      else next.delete(v.value);
    }
    unticked = next;
    if (search === "") blanksTicked = !allTicked;
  }

  function toggle(v: string): void {
    const next = new Set(unticked);
    if (next.has(v)) next.delete(v);
    else next.add(v);
    unticked = next;
  }

  function apply(): void {
    if (mode === "condition") {
      const a = conditionOf(first);
      if (typeof a === "string") {
        error = a;
        return;
      }
      let b = null;
      if (second) {
        const r = conditionOf(second);
        if (typeof r === "string") {
          error = r;
          return;
        }
        b = r;
      }
      onApply({ type: "conditions", first: a, second: b, or });
      return;
    }
    if (!list) return;
    // With a search: only the ticked values that match it (as spreadsheets do).
    const base = search.trim() ? shown : list.values;
    const values = base.filter((v) => !unticked.has(v.value)).map((v) => v.value);
    const blanks = search.trim() ? false : blanksTicked;
    if (!search.trim() && list.complete && values.length === list.values.length && (blanks || list.blanks === 0)) {
      onApply(null);
      return;
    }
    if (values.length === 0 && !blanks) {
      error = "Tick at least one value.";
      return;
    }
    onApply({ type: "values", values, blanks });
  }

  function onKey(e: KeyboardEvent): void {
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "Enter" && !(e.target instanceof HTMLButtonElement)) {
      e.preventDefault();
      apply();
    }
  }

  const left = $derived(Math.max(4, Math.min(x, window.innerWidth - 324)));
  const top = $derived(Math.max(4, Math.min(y, window.innerHeight - 470)));

  function setKind(f: CondForm, k: CondKind): void {
    f.kind = k;
    f.op = OPS[k][0]![0];
  }
</script>

{#snippet condEditor(f: CondForm)}
  <div class="cond">
    <select aria-label="Kind of rule" value={f.kind} onchange={(e) => setKind(f, (e.currentTarget as HTMLSelectElement).value as CondKind)}>
      <option value="text">Text</option>
      <option value="number">Number</option>
      <option value="date">Date</option>
      <option value="blank">Empty</option>
      <option value="pattern">Pattern</option>
      <option value="top">Largest / smallest</option>
    </select>
    <select aria-label="Comparison" bind:value={f.op}>
      {#each OPS[f.kind] as [v, label] (v)}<option value={v}>{label}</option>{/each}
    </select>
    {#if f.kind !== "blank"}
      <input aria-label="Value" bind:value={f.a} placeholder={f.kind === "date" ? "e.g. 2026-11-12" : f.kind === "pattern" ? (f.patternKind === "regex" ? "regular expression" : "e.g. A*-?") : f.kind === "top" ? "how many, e.g. 10" : ""} />
      {#if f.op === "between"}<span>and</span><input aria-label="Second value" bind:value={f.b} />{/if}
    {/if}
    {#if f.kind === "pattern"}
      <select aria-label="Pattern kind" bind:value={f.patternKind}>
        <option value="wildcard">wildcards (* ?)</option>
        <option value="regex">regular expression</option>
      </select>
    {/if}
    {#if f.kind === "text" || f.kind === "pattern"}
      <label><input type="checkbox" bind:checked={f.matchCase} /> Match case</label>
    {/if}
  </div>
{/snippet}

<div class="fmenu" role="dialog" aria-label="Filter {title}" tabindex="-1" bind:this={box} style:left="{left}px" style:top="{top}px" onkeydown={onKey}>
  <div class="sorts">
    <button type="button" onclick={() => onSort(false)}>Sort A → Z</button>
    <button type="button" onclick={() => onSort(true)}>Sort Z → A</button>
  </div>
  <div class="modes" role="tablist">
    <button type="button" role="tab" aria-selected={mode === "values"} class:on={mode === "values"} onclick={() => (mode = "values")}>Values</button>
    <button type="button" role="tab" aria-selected={mode === "condition"} class:on={mode === "condition"} onclick={() => (mode = "condition")}>Condition</button>
  </div>

  {#if mode === "values"}
    <input class="search" type="search" placeholder="Search values" aria-label="Search values" bind:value={search} oninput={onSearch} />
    <div class="list" role="group" aria-label="Values">
      {#if loading && !list}
        <div class="muted">Listing values…</div>
      {:else if list}
        <label class="row all"><input type="checkbox" checked={allTicked} onchange={toggleAll} /> (Select all)</label>
        {#if list.blanks > 0 && search === ""}
          <label class="row"><input type="checkbox" bind:checked={blanksTicked} /> <span class="v">(Empty)</span><span class="n">{list.blanks.toLocaleString()}</span></label>
        {/if}
        {#each shown.slice(0, 2000) as v (v.value)}
          <label class="row" title={v.value}>
            <input type="checkbox" checked={!unticked.has(v.value)} onchange={() => toggle(v.value)} />
            <span class="v">{v.value.replace(/[\r\n\t]+/g, " ")}</span><span class="n">{v.count.toLocaleString()}</span>
          </label>
        {/each}
        {#if shown.length > 2000}<div class="muted">{(shown.length - 2000).toLocaleString()} more; search to narrow the list.</div>{/if}
      {/if}
    </div>
    {#if list && !list.complete}<div class="muted small">This column has more than 10,000 different values; not all are listed. Search, or use a condition.</div>{/if}
  {:else}
    <div class="conds">
      {@render condEditor(first)}
      {#if second}
        <div class="join">
          <label><input type="radio" name="join" checked={!or} onchange={() => (or = false)} /> And</label>
          <label><input type="radio" name="join" checked={or} onchange={() => (or = true)} /> Or</label>
          <button type="button" class="link" onclick={() => (second = null)}>Remove</button>
        </div>
        {@render condEditor(second)}
      {:else}
        <button type="button" class="link" onclick={() => (second = emptyForm())}>+ Add a second condition</button>
      {/if}
      {#if first.kind === "top" && (!second || second.kind === "top")}
        <p class="muted small">Keeps the rows holding the largest (or smallest) numbers in this column; ties are kept.</p>
      {:else}
        <p class="muted small">Dates like 2026-11-12, 11/12/2026 or 12 Nov 2026. Wildcards: * any text, ? one character.</p>
      {/if}
    </div>
  {/if}

  {#if error}<div class="err" role="alert">{error}</div>{/if}
  <div class="buttons">
    <button type="button" disabled={!current} onclick={() => onApply(null)}>Clear filter from “{title}”</button>
    <span class="spacer"></span>
    <button type="button" class="primary" onclick={apply}>OK</button>
    <button type="button" onclick={onClose}>Cancel</button>
  </div>
</div>

<style>
  .fmenu {
    position: fixed;
    z-index: 40;
    width: 320px;
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 10px 30px rgb(0 0 0 / 0.25);
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 13px;
    outline: none;
  }
  .sorts,
  .modes,
  .buttons,
  .join {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .modes button.on {
    border-color: var(--accent);
    color: var(--accent);
    font-weight: 600;
  }
  button,
  select,
  input {
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 3px 8px;
  }
  button {
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  button.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-ink);
  }
  button.link {
    border: none;
    background: none;
    color: var(--accent);
    padding: 2px 0;
    text-align: left;
  }
  input[type="checkbox"],
  input[type="radio"] {
    padding: 0;
  }
  .search {
    width: 100%;
  }
  .list {
    height: 230px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 4px;
  }
  .row {
    display: flex;
    gap: 6px;
    align-items: center;
    padding: 1px 2px;
    white-space: nowrap;
  }
  .row .v {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .row .n {
    color: var(--muted);
    font-size: 12px;
  }
  .all {
    font-weight: 600;
  }
  .conds {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .cond {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    align-items: center;
  }
  .cond input:not([type="checkbox"]) {
    flex: 1;
    min-width: 90px;
  }
  .muted {
    color: var(--muted);
  }
  .small {
    font-size: 12px;
    margin: 0;
  }
  .err {
    color: #c0392b;
  }
  .spacer {
    flex: 1;
  }
</style>
