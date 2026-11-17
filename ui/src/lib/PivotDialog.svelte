<script lang="ts">
  // Pivot table builder: drag fields into Filters, Columns, Rows and Values
  // and the result shows at once. Big files get a quick look from their
  // first rows, then the full result. Nothing in the file changes.
  import { untrack } from "svelte";
  import { backend, errorText, isCmdError, type PlaceSwap } from "./backend";
  import {
    AGGS,
    allOpen,
    CMP_OPS,
    fieldName,
    GROUPS,
    insertValue,
    moveValue,
    removeValue,
    LABEL_OPS,
    looksNumeric,
    newField,
    newSpec,
    newValue,
    QUICK_FROM,
    QUICK_ROWS,
    scanKey,
    SHOW_AS,
    toggled,
    usesAlong,
    valueName,
    type Agg,
    type Area,
    type ItemInfo,
    type PivotField,
    type PivotGrid,
    type PivotSpec,
    type ValueField,
  } from "./pivot";

  interface Props {
    doc: number;
    titles: string[];
    header: boolean;
    /** Data rows in the file (decides the quick first look). */
    rows: number;
    initial: PivotSpec | null;
    /** A few values of a column (to guess Sum or Count). */
    sample: (col: number) => string[];
    onTable: (spec: PivotSpec) => void;
    onCopy: (spec: PivotSpec) => void;
    onDrill: (spec: PivotSpec, rowPath: string[], colPath: string[]) => void;
    onSay: (text: string) => void;
    /** Ask before saving item values to a file (true: go ahead). */
    onConfirmSave: (items: number) => Promise<boolean>;
    onClose: (spec: PivotSpec) => void;
  }
  let { doc, titles, header, rows, initial, sample, onTable, onCopy, onDrill, onSay, onConfirmSave, onClose }: Props = $props();

  let spec = $state<PivotSpec>(untrack(() => structuredClone(initial ?? newSpec(header))));

  // The tab's "First row is header" decides, also for a layout kept from
  // when it was off: otherwise the header would be counted as a value.
  $effect(() => {
    if (spec.keep_first !== header) spec.keep_first = header;
  });
  let grid = $state<PivotGrid | null>(null);
  let busy = $state(false);
  let progress = $state<number | null>(null);
  let problem = $state("");
  /** A layout whose columns were found by place, not title: asked first. */
  let byPlace = $state<{ name: string; spec: PivotSpec; swaps: PlaceSwap[] } | null>(null);
  let search = $state("");
  let layoutName = $state("");
  /** The chip whose settings show. */
  let sel = $state<{ area: Area; index: number } | null>(null);
  let box: HTMLDivElement;

  const AREAS: [Area, string, string][] = [
    ["filters", "Filters", "Only rows with the picked items count"],
    ["cols", "Columns", "One column for each item"],
    ["rows", "Rows", "One row for each item"],
    ["values", "Values", "What is counted or added up"],
  ];

  // ---- live preview ----
  let gen = 0;
  let lastFull = "";
  let timer = 0;
  $effect(() => {
    const snap = JSON.stringify(spec);
    clearTimeout(timer);
    timer = window.setTimeout(() => void refresh(JSON.parse(snap) as PivotSpec), 120);
    return () => clearTimeout(timer);
  });

  $effect(() => {
    if (!busy) return;
    const poll = window.setInterval(async () => {
      try {
        const j = await backend().jobProgress(doc);
        progress = j ? j.fraction : null;
      } catch {
        // Closed meanwhile.
      }
    }, 200);
    return () => {
      clearInterval(poll);
      progress = null;
    };
  });

  async function refresh(s: PivotSpec): Promise<void> {
    const my = ++gen;
    if (!s.rows.length && !s.cols.length && !s.values.length) {
      grid = null;
      busy = false;
      problem = "";
      return;
    }
    busy = true;
    problem = "";
    try {
      const key = scanKey(s);
      if (rows > QUICK_FROM && key !== lastFull) {
        const q = await backend().pivotPreview(doc, s, QUICK_ROWS);
        if (my !== gen) return;
        grid = q;
      }
      const g = await backend().pivotPreview(doc, s, null);
      if (my !== gen) return;
      grid = g;
      lastFull = key;
    } catch (e) {
      if (my !== gen) return;
      if (isCmdError(e) && e.kind === "cancelled") return;
      problem = errorText(e);
    } finally {
      if (my === gen) busy = false;
    }
  }

  function stop(): void {
    gen++;
    busy = false;
    void backend().cancelJob(doc);
    problem = "Stopped. Change a field, or click Refresh, to try again.";
  }

  function again(): void {
    lastFull = "";
    void backend().pivotClose(doc).then(() => refresh($state.snapshot(spec) as PivotSpec));
  }

  // ---- fields ----
  function guessAgg(col: number): Agg {
    return looksNumeric(sample(col)) ? "sum" : "count";
  }

  function list(area: Area): (PivotField | ValueField)[] {
    return area === "values" ? spec.values : spec[area];
  }

  function chipText(area: Area, i: number): string {
    return area === "values" ? valueName(spec.values[i]!, titles) : fieldName(spec[area][i]!, titles);
  }

  function insert(area: Area, at: number, col: number, from?: PivotField | ValueField): void {
    if (area === "values") {
      const v = from && "agg" in from ? from : newValue(col, guessAgg(col));
      insertValue(spec, at, v);
    } else {
      const f = from && "group" in from ? from : newField(col);
      spec[area].splice(at, 0, f);
    }
    sel = { area, index: at };
  }

  function add(area: Area, col: number): void {
    insert(area, list(area).length, col);
  }

  function quickAdd(col: number): void {
    if (looksNumeric(sample(col))) add("values", col);
    else add("rows", col);
  }

  function remove(area: Area, i: number): void {
    // Sorts, top N and value filters that used a value field let it go.
    if (area === "values") removeValue(spec, i);
    else spec[area].splice(i, 1);
    sel = null;
  }

  function move(from: { area: Area; index: number }, to: Area, at: number): void {
    const item = $state.snapshot(list(from.area)[from.index]) as PivotField | ValueField | undefined;
    if (!item) return;
    if (from.area === to && (at === from.index || at === from.index + 1)) return;
    if (from.area === "values" && to === "values") {
      // Reordering values keeps what points at each one.
      const end = at > from.index ? at - 1 : at;
      moveValue(spec, from.index, end);
      sel = { area: to, index: end };
      return;
    }
    remove(from.area, from.index);
    if (from.area === to && at > from.index) at--;
    insert(to, Math.min(at, list(to).length), item.col, item);
  }

  function shift(area: Area, i: number, by: number): void {
    const j = i + by;
    const l = list(area);
    if (j < 0 || j >= l.length) return;
    move({ area, index: i }, area, by > 0 ? j + 1 : j);
    sel = { area, index: j };
  }

  // ---- dragging (pointer events: the window's own file drop takes the
  // browser's drag and drop) ----
  type Source = { area: Area | "list"; index: number; col: number; label: string };
  let pending: (Source & { x: number; y: number }) | null = null;
  let drag = $state<(Source & { x: number; y: number }) | null>(null);
  let target = $state<{ area: Area; index: number } | null>(null);

  function down(e: PointerEvent, s: Source): void {
    if (e.button !== 0 || (e.target as HTMLElement).closest("button, input, select")) return;
    pending = { ...s, x: e.clientX, y: e.clientY };
  }

  function dropAt(x: number, y: number): { area: Area; index: number } | null {
    const el = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-area]");
    if (!el) return null;
    const area = el.dataset.area as Area;
    const chips = [...el.querySelectorAll<HTMLElement>("[data-chip]")];
    let index = chips.length;
    for (let i = 0; i < chips.length; i++) {
      const r = chips[i]!.getBoundingClientRect();
      if (y < r.top + r.height / 2) {
        index = i;
        break;
      }
    }
    return { area, index };
  }

  function onMove(e: PointerEvent): void {
    if (pending && !drag && Math.hypot(e.clientX - pending.x, e.clientY - pending.y) > 5) drag = pending;
    if (!drag) return;
    drag = { ...drag, x: e.clientX, y: e.clientY };
    target = dropAt(e.clientX, e.clientY);
  }

  function onUp(): void {
    const d = drag;
    const t = target;
    pending = null;
    drag = null;
    target = null;
    if (!d) return;
    if (t) {
      if (d.area === "list") insert(t.area, t.index, d.col);
      else move({ area: d.area, index: d.index }, t.area, t.index);
    } else if (d.area !== "list") {
      // Dragged out of the areas: taken out of the pivot.
      remove(d.area, d.index);
    }
  }

  function chipKey(e: KeyboardEvent, area: Area, i: number): void {
    if (e.key === "Delete" || e.key === "Backspace") {
      e.preventDefault();
      remove(area, i);
    } else if (e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
      e.preventDefault();
      shift(area, i, e.key === "ArrowUp" ? -1 : 1);
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      sel = sel?.area === area && sel.index === i ? null : { area, index: i };
    }
  }

  // ---- settings of the selected chip ----
  const selField = $derived(sel && sel.area !== "values" ? (spec[sel.area][sel.index] ?? null) : null);
  const selValue = $derived(sel && sel.area === "values" ? (spec.values[sel.index] ?? null) : null);

  let items = $state<ItemInfo[] | null>(null);
  let itemsTotal = $state(0);
  let checked = $state<Set<string>>(new Set());
  let itemSearch = $state("");

  async function loadItems(): Promise<void> {
    const f = selField;
    if (!f) return;
    busy = true;
    try {
      const l = await backend().pivotItems(doc, $state.snapshot(f) as PivotField, $state.snapshot(spec) as PivotSpec);
      items = l.items;
      itemsTotal = l.total;
      const p = f.items;
      checked = new Set(l.items.filter((it) => !p || p.items.includes(it.key) !== p.exclude).map((it) => it.key));
    } catch (e) {
      problem = errorText(e);
    } finally {
      busy = false;
    }
  }

  function applyItems(): void {
    const f = selField;
    if (!f || !items) return;
    const on = items.filter((it) => checked.has(it.key)).map((it) => it.key);
    const off = items.filter((it) => !checked.has(it.key)).map((it) => it.key);
    if (!off.length) f.items = null;
    // Items past the list's end stay in unless the list is complete.
    else if (on.length <= off.length && itemsTotal === items.length) f.items = { exclude: false, items: on };
    else f.items = { exclude: true, items: off };
    items = null;
  }

  const shownItems = $derived(items ? items.filter((it) => it.label.toLowerCase().includes(itemSearch.toLowerCase())).slice(0, 500) : []);

  $effect(() => {
    // A different chip: its own item list.
    void sel;
    untrack(() => {
      items = null;
      itemSearch = "";
    });
  });

  function setGroup(f: PivotField, kind: string): void {
    f.group = kind === "bins" ? { kind: "bins", start: 0, size: 10, end: null } : ({ kind } as PivotField["group"]);
    f.items = null;
  }

  // ---- preview actions ----
  function toggleRow(path: string[], collapsed: boolean): void {
    spec.layout = toggled($state.snapshot(spec.layout), true, path, collapsed);
  }

  function toggleCol(path: string[], collapsed: boolean): void {
    spec.layout = toggled($state.snapshot(spec.layout), false, path, collapsed);
  }

  function drill(r: number, c: number): void {
    const g = grid;
    const row = g?.rows[r];
    const col = g?.cols[c];
    if (!g || !row || !col || g.sampled) return;
    onDrill($state.snapshot(spec) as PivotSpec, row.path, col.path);
  }

  async function saveLayout(): Promise<void> {
    // Picked items are values from the file: say so before they go in it.
    const picked = [...spec.rows, ...spec.cols, ...spec.filters].reduce((n, f) => n + (f.items?.items.length ?? 0), 0);
    if (picked && !(await onConfirmSave(picked))) return;
    try {
      const f = await backend().pivotSave(doc, layoutName, $state.snapshot(spec) as PivotSpec);
      if (f) onSay(`Saved the pivot layout as ${f}.`);
    } catch (e) {
      problem = errorText(e);
    }
  }

  async function openLayout(): Promise<void> {
    try {
      const r = await backend().pivotOpen(doc, spec.keep_first);
      if (!r) return;
      // Some of its columns were not found by name (renamed, say): ask
      // before using the columns in the same places.
      if (r[2].length) {
        byPlace = { name: r[0], spec: r[1], swaps: r[2] };
        return;
      }
      layoutName = r[0];
      spec = r[1];
      sel = null;
    } catch (e) {
      problem = errorText(e);
    }
  }

  function close(): void {
    gen++;
    if (busy) void backend().cancelJob(doc);
    onClose($state.snapshot(spec) as PivotSpec);
  }

  function onKey(e: KeyboardEvent): void {
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      if (sel) sel = null;
      else close();
    }
  }

  $effect(() => {
    box.querySelector<HTMLElement>("input")?.focus();
  });

  const fields = $derived(titles.map((t, c) => ({ t, c })).filter((f) => f.t.toLowerCase().includes(search.toLowerCase())));
  const usedCols = $derived(new Set([...spec.rows, ...spec.cols, ...spec.filters, ...spec.values].map((f) => f.col)));
  const fmt = (n: number) => n.toLocaleString();
  const ready = $derived(spec.rows.length + spec.cols.length + spec.values.length > 0);
</script>

<svelte:window onpointermove={onMove} onpointerup={onUp} />

<div class="backdrop" role="presentation">
  <div class="shell" role="dialog" aria-modal="true" aria-labelledby="pivot-title" tabindex="-1" bind:this={box} onkeydown={onKey}>
    <header>
      <h2 id="pivot-title">Pivot table</h2>
      <span class="hint">Drag fields into the boxes (or use the buttons). The result below updates as you go; double-click a number to see its rows.</span>
    </header>
    <div class="body">
      <aside class="side">
        <input type="text" placeholder="Find a field" aria-label="Find a field" bind:value={search} />
        <ul class="fields" aria-label="Fields">
          {#each fields as f (f.c)}
            <li
              class:used={usedCols.has(f.c)}
              onpointerdown={(e) => down(e, { area: "list", index: f.c, col: f.c, label: f.t })}
              ondblclick={() => quickAdd(f.c)}
            >
              <span class="name" title={f.t}>{f.t}</span>
              <span class="adds">
                <button type="button" title="Add to Rows" aria-label={`Add ${f.t} to Rows`} onclick={() => add("rows", f.c)}>R</button>
                <button type="button" title="Add to Columns" aria-label={`Add ${f.t} to Columns`} onclick={() => add("cols", f.c)}>C</button>
                <button type="button" title="Add to Values" aria-label={`Add ${f.t} to Values`} onclick={() => add("values", f.c)}>Σ</button>
                <button type="button" title="Add to Filters" aria-label={`Add ${f.t} to Filters`} onclick={() => add("filters", f.c)}>F</button>
              </span>
            </li>
          {/each}
        </ul>
        <div class="areas">
          {#each AREAS as [area, label, help] (area)}
            <section class="area" class:over={target?.area === area} data-area={area} aria-label={label}>
              <h3 title={help}>{label}</h3>
              {#each list(area) as _, i (i)}
                {#if target?.area === area && target.index === i && drag}<div class="mark"></div>{/if}
                <div
                  class="chip"
                  class:on={sel?.area === area && sel.index === i}
                  data-chip
                  role="button"
                  tabindex="0"
                  onpointerdown={(e) => down(e, { area, index: i, col: list(area)[i]!.col, label: chipText(area, i) })}
                  onclick={(e) => {
                    if (!(e.target as HTMLElement).closest("button")) sel = sel?.area === area && sel.index === i ? null : { area, index: i };
                  }}
                  onkeydown={(e) => chipKey(e, area, i)}
                >
                  <span class="name" title={chipText(area, i)}>{chipText(area, i)}</span>
                  <button type="button" title="Take out" aria-label="Take out" onclick={() => remove(area, i)}>✕</button>
                </div>
              {/each}
              {#if target?.area === area && target.index === list(area).length && drag}<div class="mark"></div>{/if}
              {#if !list(area).length}<p class="empty">{help}</p>{/if}
            </section>
          {/each}
        </div>
      </aside>

      <main class="main">
        {#if selField && sel}
          {@const f = selField}
          <section class="settings" aria-label="Field settings">
            <h3>{titles[f.col]} <span class="muted">in {AREAS.find(([a]) => a === sel?.area)?.[1]}</span></h3>
            <div class="grid3">
              <label>Group <select value={f.group.kind} onchange={(e) => setGroup(f, e.currentTarget.value)}>{#each GROUPS as [k, t] (k)}<option value={k}>{t}</option>{/each}</select></label>
              {#if f.group.kind === "bins"}
                {@const b = f.group}
                <label>From <input type="number" value={b.start} onchange={(e) => (f.group = { ...b, start: Number(e.currentTarget.value) || 0 })} /></label>
                <label>Size <input type="number" min="0" value={b.size} onchange={(e) => (f.group = { ...b, size: Math.max(Number(e.currentTarget.value) || 1, 1e-9) })} /></label>
                <label>Up to <input type="number" value={b.end ?? ""} placeholder="no end" onchange={(e) => (f.group = { ...b, end: e.currentTarget.value === "" ? null : Number(e.currentTarget.value) })} /></label>
              {/if}
              {#if sel.area !== "filters"}
                <label
                  >Sort
                  <select
                    value={f.sort.by === "label" ? (f.sort.desc ? "za" : "az") : `v${f.sort.value}${f.sort.desc ? "d" : "a"}`}
                    onchange={(e) => {
                      const v = e.currentTarget.value;
                      if (v === "az" || v === "za") f.sort = { by: "label", value: 0, desc: v === "za" };
                      else f.sort = { by: "value", value: Number(v.slice(1, -1)), desc: v.endsWith("d") };
                    }}
                  >
                    <option value="az">A to Z (and numbers, dates in order)</option>
                    <option value="za">Z to A</option>
                    {#each spec.values as v, i (i)}
                      <option value={`v${i}d`}>{valueName(v, titles)}: largest first</option>
                      <option value={`v${i}a`}>{valueName(v, titles)}: smallest first</option>
                    {/each}
                  </select>
                </label>
                <label
                  >Show
                  <select
                    value={f.top ? (f.top.bottom ? "bottom" : "top") : "all"}
                    onchange={(e) => {
                      const v = e.currentTarget.value;
                      f.top = v === "all" || !spec.values.length ? null : { n: f.top?.n ?? 10, value: f.top?.value ?? 0, bottom: v === "bottom" };
                    }}
                  >
                    <option value="all">All items</option>
                    <option value="top" disabled={!spec.values.length}>Top items</option>
                    <option value="bottom" disabled={!spec.values.length}>Bottom items</option>
                  </select>
                </label>
                {#if f.top}
                  {@const t = f.top}
                  <label>How many <input type="number" min="1" value={t.n} onchange={(e) => (f.top = { ...t, n: Math.max(1, Math.floor(Number(e.currentTarget.value) || 1)) })} /></label>
                  <label>By <select value={t.value} onchange={(e) => (f.top = { ...t, value: Number(e.currentTarget.value) })}>{#each spec.values as v, i (i)}<option value={i}>{valueName(v, titles)}</option>{/each}</select></label>
                {/if}
                <label class="check"><input type="checkbox" bind:checked={f.subtotal} /> Subtotal for this field</label>
              {/if}
            </div>
            <div class="grid3">
              <label
                >Label filter
                <select
                  value={f.label_filter?.op ?? ""}
                  onchange={(e) => {
                    const v = e.currentTarget.value;
                    f.label_filter = v ? { op: v as never, text: f.label_filter?.text ?? "", text2: f.label_filter?.text2 ?? "" } : null;
                  }}
                >
                  <option value="">(none)</option>
                  {#each LABEL_OPS as [k, t] (k)}<option value={k}>{t}</option>{/each}
                </select>
              </label>
              {#if f.label_filter}
                <label>Text <input type="text" bind:value={f.label_filter.text} /></label>
                {#if f.label_filter.op === "between" || f.label_filter.op === "not_between"}<label>and <input type="text" bind:value={f.label_filter.text2} /></label>{/if}
              {/if}
            </div>
            {#if sel.area !== "filters"}
              <div class="grid3">
                <label
                  >Value filter
                  <select
                    value={f.value_filter ? String(f.value_filter.value) : ""}
                    disabled={!spec.values.length}
                    onchange={(e) => {
                      const v = e.currentTarget.value;
                      f.value_filter = v === "" ? null : { value: Number(v), op: f.value_filter?.op ?? "greater", a: f.value_filter?.a ?? 0, b: f.value_filter?.b ?? 0 };
                    }}
                  >
                    <option value="">(none)</option>
                    {#each spec.values as v, i (i)}<option value={String(i)}>{valueName(v, titles)}</option>{/each}
                  </select>
                </label>
                {#if f.value_filter}
                  {@const vf = f.value_filter}
                  <label><select value={vf.op} onchange={(e) => (f.value_filter = { ...vf, op: e.currentTarget.value as never })}>{#each CMP_OPS as [k, t] (k)}<option value={k}>{t}</option>{/each}</select></label>
                  <label><input type="number" value={vf.a} aria-label="Number" onchange={(e) => (f.value_filter = { ...vf, a: Number(e.currentTarget.value) || 0 })} /></label>
                  {#if vf.op === "between" || vf.op === "not_between"}<label>and <input type="number" value={vf.b} onchange={(e) => (f.value_filter = { ...vf, b: Number(e.currentTarget.value) || 0 })} /></label>{/if}
                {/if}
              </div>
            {/if}
            <div class="items">
              {#if items}
                <div class="row">
                  <input type="text" placeholder="Find an item" aria-label="Find an item" bind:value={itemSearch} />
                  <button type="button" onclick={() => (checked = new Set(items?.map((i) => i.key)))}>All</button>
                  <button type="button" onclick={() => (checked = new Set())}>None</button>
                  <button type="button" class="primary" onclick={applyItems}>Use these items</button>
                  <button type="button" onclick={() => (items = null)}>Cancel</button>
                </div>
                <div class="itemlist">
                  {#each shownItems as it (it.key)}
                    <label
                      ><input
                        type="checkbox"
                        checked={checked.has(it.key)}
                        onchange={() => {
                          const n = new Set(checked);
                          if (n.has(it.key)) n.delete(it.key);
                          else n.add(it.key);
                          checked = n;
                        }}
                      />
                      {it.label} <span class="muted">({fmt(it.rows)})</span></label
                    >
                  {/each}
                </div>
                {#if itemsTotal > items.length || shownItems.length === 500}<p class="muted">Showing some of {fmt(itemsTotal)} items; find one by name.</p>{/if}
              {:else}
                <button type="button" onclick={loadItems}>Choose items…</button>
                {#if f.items}<span class="muted">{f.items.exclude ? `${fmt(f.items.items.length)} left out` : `${fmt(f.items.items.length)} picked`}</span> <button type="button" onclick={() => (f.items = null)}>Use all items</button>{/if}
              {/if}
            </div>
          </section>
        {:else if selValue}
          {@const v = selValue}
          <section class="settings" aria-label="Value settings">
            <h3>{valueName(v, titles)}</h3>
            <div class="grid3">
              <label>Summarize by <select bind:value={v.agg}>{#each AGGS as [k, t] (k)}<option value={k}>{t}</option>{/each}</select></label>
              <label>Show as <select bind:value={v.show_as} disabled={["first", "last", "join"].includes(v.agg)}>{#each SHOW_AS as [k, t] (k)}<option value={k}>{t}</option>{/each}</select></label>
              {#if usesAlong(v.show_as)}
                <label>Along <select bind:value={v.along}><option value="rows">the rows</option><option value="columns">the columns</option></select></label>
              {/if}
              <label>Name <input type="text" placeholder={valueName({ ...v, name: "" }, titles)} bind:value={v.name} /></label>
              {#if v.agg === "join"}<label>Between values <input type="text" bind:value={v.separator} /></label>{/if}
            </div>
            <div class="grid3">
              <label
                >Decimals
                <select
                  value={v.format.decimals === null ? "" : String(v.format.decimals)}
                  onchange={(e) => (v.format.decimals = e.currentTarget.value === "" ? null : Number(e.currentTarget.value))}
                >
                  <option value="">As needed</option>
                  {#each [0, 1, 2, 3, 4, 5, 6] as d (d)}<option value={String(d)}>{d}</option>{/each}
                </select>
              </label>
              <label class="check"><input type="checkbox" bind:checked={v.format.thousands} /> 1,000 separators</label>
              <label>Before <input type="text" size="4" placeholder="$" bind:value={v.format.prefix} /></label>
              <label>After <input type="text" size="4" placeholder=" kg" bind:value={v.format.suffix} /></label>
            </div>
          </section>
        {/if}

        <div class="toolbar">
          <label class="check"><input type="checkbox" bind:checked={spec.layout.grand_row} /> Grand total row</label>
          <label class="check"><input type="checkbox" bind:checked={spec.layout.grand_col} /> Grand total column</label>
          <label class="check"><input type="checkbox" bind:checked={spec.layout.row_subtotals} /> Row subtotals</label>
          <label class="check"><input type="checkbox" bind:checked={spec.layout.col_subtotals} /> Column subtotals</label>
          <label
            >Subtotals
            <select value={spec.layout.subtotals_top ? "top" : "bottom"} onchange={(e) => (spec.layout.subtotals_top = e.currentTarget.value === "top")}
              ><option value="top">on the group's row</option><option value="bottom">on a Total row below</option></select
            >
          </label>
          <button type="button" onclick={() => (spec.layout = allOpen($state.snapshot(spec.layout), true, true))}>Expand all</button>
          <button type="button" onclick={() => (spec.layout = allOpen($state.snapshot(spec.layout), true, false))}>Collapse all</button>
          <details>
            <summary>More</summary>
            <div class="more">
              <label class="check"><input type="checkbox" bind:checked={spec.layout.repeat_labels} /> Repeat labels on every row (new tab)</label>
              <label>Blank items show as <input type="text" size="10" bind:value={spec.layout.blank_label} /></label>
              <label>Empty cells show <input type="text" size="6" bind:value={spec.layout.empty_cells} /></label>
              <label class="check"><input type="checkbox" bind:checked={spec.match_case} /> "Smith" and "SMITH" are different</label>
              <label class="check"><input type="checkbox" bind:checked={spec.keep_spaces} /> Spaces at the ends count</label>
              <label class="check"
                ><input type="checkbox" checked={spec.read.decimal_comma} onchange={(e) => (spec.read = { ...spec.read, decimal_comma: e.currentTarget.checked })} /> Numbers use a decimal comma (1.234,5)</label
              >
              <label
                >Dates like 03/04
                <select value={spec.read.date_order} onchange={(e) => (spec.read = { ...spec.read, date_order: e.currentTarget.value as "mdy" | "dmy" })}
                  ><option value="mdy">are March 4</option><option value="dmy">are 3 April</option></select
                >
              </label>
            </div>
          </details>
        </div>

        <div class="status" aria-live="polite">
          {#if busy}
            <span>Working{progress !== null ? ` ${Math.round(progress * 100)}%` : ""}…</span>
            <button type="button" onclick={stop}>Stop</button>
          {/if}
          {#if grid && !problem}
            {#if grid.sampled}<strong class="sample">Quick preview from the first {fmt(grid.rows_read)} rows</strong>{#if busy}<span> — the full result is on its way</span>{/if}{:else}<span>{fmt(grid.rows_used)} of {fmt(grid.rows_read)} rows counted</span>{/if}
            <span class="muted">· {fmt(grid.total_rows)} rows × {fmt(grid.total_cols)} value columns{grid.truncated ? " (the preview shows the start; the new tab gets it all)" : ""}</span>
          {/if}
          {#if problem}<span class="problem" role="alert">{problem}</span>{/if}
          <button type="button" class="right" onclick={again} title="Read the file again">Refresh</button>
        </div>
        {#if byPlace}
          {@const b = byPlace}
          <div class="by-place" role="alertdialog" aria-label="Use columns in the same place?">
            <p>
              The layout “{b.name}” uses {b.swaps.length === 1 ? "a column" : "columns"} this file has under another name (renamed, perhaps). Use the {b.swaps.length === 1 ? "column" : "columns"} in the same place?
            </p>
            <ul>
              {#each b.swaps as w (w.col)}
                <li>“{w.was}” → column {w.col + 1}{w.now ? `, “${w.now}”` : ""}</li>
              {/each}
            </ul>
            <button
              type="button"
              class="primary"
              onclick={() => {
                layoutName = b.name;
                spec = b.spec;
                sel = null;
                byPlace = null;
              }}>Use {b.swaps.length === 1 ? "that column" : "those columns"}</button
            >
            <button type="button" onclick={() => (byPlace = null)}>Don't open the layout</button>
          </div>
        {/if}

        <div class="preview">
          {#if !ready}
            <p class="empty big">Drag a field into <b>Rows</b> to list its values, and a field into <b>Values</b> to count or add up. Add more fields to break things down further.</p>
          {:else if grid}
            <table>
              <thead>
                {#each grid.head as h, hi (hi)}
                  <tr>
                    {#each h as c, ci (ci)}
                      <th colspan={c.span} class:corner={ci < grid.label_cols}>
                        {#if c.toggle !== null}<button type="button" class="tog" aria-label={c.toggle ? "Expand" : "Collapse"} onclick={() => toggleCol(c.path, c.toggle === true)}>{c.toggle ? "▸" : "▾"}</button>{/if}{c.text}
                      </th>
                    {/each}
                  </tr>
                {/each}
              </thead>
              <tbody>
                {#each grid.rows as r, ri (ri)}
                  <tr class={r.kind}>
                    {#each r.labels as l, li (li)}
                      <th scope="row">
                        {#if r.toggle !== null && li === r.depth - 1}<button type="button" class="tog" aria-label={r.toggle ? "Expand" : "Collapse"} onclick={() => toggleRow(r.path, r.toggle === true)}>{r.toggle ? "▸" : "▾"}</button>{/if}{l}
                      </th>
                    {/each}
                    {#each r.cells as v, ci (ci)}
                      <td class:num={!grid.cols[ci]?.text} ondblclick={() => drill(ri, ci)} title={grid.sampled ? "" : "Double-click to see these rows"}>{v}</td>
                    {/each}
                  </tr>
                {/each}
              </tbody>
            </table>
          {/if}
        </div>
      </main>
    </div>
    <footer>
      <button type="button" class="primary" disabled={!ready} onclick={() => onTable($state.snapshot(spec) as PivotSpec)}>Open as a new tab</button>
      <button type="button" disabled={!ready} onclick={() => onCopy($state.snapshot(spec) as PivotSpec)}>Copy</button>
      <input type="text" size="14" placeholder="Layout name" aria-label="Layout name" bind:value={layoutName} />
      <button type="button" disabled={!ready} onclick={saveLayout}>Save layout…</button>
      <button type="button" onclick={openLayout}>Open layout…</button>
      <button
        type="button"
        onclick={() => {
          spec = newSpec(header);
          sel = null;
        }}>Clear</button
      >
      <button type="button" class="right" onclick={close}>Close</button>
    </footer>
  </div>
  {#if drag}<div class="ghost" style:left={`${drag.x + 10}px`} style:top={`${drag.y + 6}px`}>{drag.label}</div>{/if}
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgb(0 0 0 / 0.35);
    display: grid;
    place-items: center;
    z-index: 50;
    padding: 12px;
  }
  .shell {
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 10px;
    width: min(1400px, 100%);
    height: min(900px, 100%);
    display: flex;
    flex-direction: column;
    box-shadow: 0 12px 40px rgb(0 0 0 / 0.3);
    outline: none;
    font-size: 13px;
    box-sizing: border-box;
    overflow: hidden;
    user-select: none;
  }
  header,
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
    flex-wrap: wrap;
  }
  header {
    border-bottom: 1px solid var(--border);
  }
  footer {
    border-top: 1px solid var(--border);
  }
  h2 {
    font-size: 16px;
    margin: 0;
  }
  h3 {
    font-size: 12px;
    margin: 0 0 4px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--muted);
  }
  .hint,
  .muted,
  .empty {
    color: var(--muted);
  }
  .body {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .side {
    width: 300px;
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px;
    border-right: 1px solid var(--border);
    min-height: 0;
  }
  .fields {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow: auto;
    flex: 1 1 40%;
    min-height: 80px;
    border: 1px solid var(--border);
    border-radius: 6px;
  }
  .fields li {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 2px 6px;
    cursor: grab;
  }
  .fields li:hover {
    background: var(--header);
  }
  .fields li.used .name {
    font-weight: 600;
  }
  .name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .adds button {
    padding: 0 5px;
    min-width: 22px;
    font-size: 11px;
  }
  .areas {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
    flex: 1 1 60%;
    min-height: 0;
  }
  .area {
    border: 1px dashed var(--border);
    border-radius: 6px;
    padding: 6px;
    overflow: auto;
    min-height: 90px;
  }
  .area.over {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  .chip {
    display: flex;
    align-items: center;
    gap: 4px;
    border: 1px solid var(--border);
    background: var(--bg);
    border-radius: 5px;
    padding: 2px 4px 2px 8px;
    margin: 3px 0;
    cursor: grab;
  }
  .chip.on {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }
  .chip button {
    padding: 0 4px;
    border: none;
    background: none;
    color: var(--muted);
  }
  .mark {
    height: 2px;
    background: var(--accent);
    margin: 1px 0;
  }
  .empty {
    margin: 4px 0;
    font-size: 12px;
  }
  .empty.big {
    margin: 40px auto;
    max-width: 460px;
    text-align: center;
    font-size: 14px;
    line-height: 1.5;
  }
  .main {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    padding: 10px;
    gap: 6px;
  }
  .settings {
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 8px 10px;
    max-height: 40%;
    overflow: auto;
    user-select: text;
  }
  .grid3 {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 14px;
    margin: 4px 0;
    align-items: center;
  }
  .grid3 label,
  .toolbar label,
  .more label {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  input[type="number"] {
    width: 90px;
  }
  .items {
    margin-top: 6px;
  }
  .row {
    display: flex;
    gap: 6px;
    align-items: center;
    flex-wrap: wrap;
  }
  .itemlist {
    max-height: 160px;
    overflow: auto;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: 2px 10px;
    margin-top: 4px;
  }
  .itemlist label {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 14px;
    align-items: center;
  }
  details {
    position: relative;
  }
  .more {
    position: absolute;
    z-index: 2;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 8px 10px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    width: 340px;
    box-shadow: 0 6px 20px rgb(0 0 0 / 0.2);
  }
  .status {
    display: flex;
    gap: 8px;
    align-items: center;
    min-height: 26px;
    flex-wrap: wrap;
  }
  .sample {
    color: var(--accent);
  }
  .problem {
    color: var(--danger, #c00);
  }
  .by-place {
    margin: 6px 0;
    padding: 8px 12px;
    border: 1px solid var(--accent);
    border-radius: 6px;
    background: var(--surface);
  }
  .by-place p {
    margin: 0 0 4px;
  }
  .by-place ul {
    margin: 0 0 8px;
    padding-left: 20px;
  }
  .right {
    margin-left: auto;
  }
  .preview {
    flex: 1;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 6px;
    min-height: 0;
    user-select: text;
  }
  table {
    border-collapse: separate;
    border-spacing: 0;
    font-variant-numeric: tabular-nums;
  }
  th,
  td {
    border-right: 1px solid var(--grid-line);
    border-bottom: 1px solid var(--grid-line);
    padding: 2px 8px;
    white-space: nowrap;
    max-width: 360px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  thead th {
    position: sticky;
    background: var(--header);
    font-weight: 600;
    text-align: left;
    z-index: 1;
  }
  thead tr:nth-child(1) th {
    top: 0;
  }
  thead tr:nth-child(2) th {
    top: 23px;
  }
  thead tr:nth-child(3) th {
    top: 46px;
  }
  thead tr:nth-child(4) th {
    top: 69px;
  }
  tbody th {
    text-align: left;
    font-weight: normal;
    background: var(--surface);
  }
  td.num {
    text-align: right;
  }
  td {
    cursor: default;
  }
  tr.group th,
  tr.group td,
  tr.subtotal th,
  tr.subtotal td {
    font-weight: 600;
  }
  tr.grand th,
  tr.grand td {
    font-weight: 700;
    border-top: 2px solid var(--border);
  }
  .tog {
    border: none;
    background: none;
    padding: 0 4px 0 0;
    color: var(--muted);
    cursor: pointer;
  }
  .ghost {
    position: fixed;
    pointer-events: none;
    z-index: 60;
    background: var(--accent);
    color: var(--accent-ink);
    padding: 3px 8px;
    border-radius: 5px;
    font-size: 12px;
    opacity: 0.9;
  }
  .check {
    white-space: nowrap;
  }
</style>
