<script lang="ts">
  // The repair bar: one ragged row at a time. Its text as it is in the file (display only), its fields with the
  // boundaries to click, and what the row will be. Join fields, join with the next row of the file, or read a quote
  // mark as text; each is one undo step. Previous and Next walk the ragged rows by their ids.
  import { tick, untrack } from "svelte";
  import { backend, errorText, type Between, type DocInfo, type RowRepair, type RowText, type SameShape } from "./backend";
  import { joinPreview, groups, nextNotes, nextPreview, noteWarning, previousNotes, rowHeading, sameShapeWords, stepRow } from "./repair";
  import type { Tab } from "./tab.svelte";

  interface Props {
    tab: Tab;
    titles: string[];
    /** The shown row to start at (null: the first ragged row). */
    start: number | null;
    /** Move the active cell to a shown row. */
    onGo: (row: number) => void;
    /** A repair was made (the tab's new facts). */
    onDone: (info: DocInfo, what: string) => void;
    onReopenLines: () => void;
    onClose: () => void;
    /** Run a long call as a job with progress and Stop (null: stopped). */
    job: <T>(label: string, call: () => Promise<T>) => Promise<T | null>;
  }
  let { tab, titles, start, onGo, onDone, onReopenLines, onClose, job }: Props = $props();

  /** Fields drawn at once. */
  const MAX_DRAWN = 200;
  let row = $state<RowText | null>(null);
  let ids = $state<number[]>([]);
  let total = $state(0);
  let joined = $state<Set<number>>(new Set());
  let cursor = $state(0);
  let mode = $state<"join" | "next" | "previous" | "quote">("join");
  let between = $state<Between>("space");
  let problem = $state("");
  let busy = $state(false);
  let box: HTMLElement | undefined = $state();
  let seq = 0;

  /** After one join: the other rows of the same shape, offered to be fixed together. */
  let offer = $state<{ fields: number; groups: number[]; same: SameShape } | null>(null);
  let listing = $state(false);
  let unticked = $state<Set<number>>(new Set());
  /** Empty columns past the usual number that could go (0: none), and the revision looked at. */
  let extra = $state<{ count: number; revision: number }>({ count: 0, revision: 0 });

  /** Look for rows of the same shape after a join, and whether the extra column is empty now. */
  async function afterRepair(shape: { fields: number; groups: number[] } | null): Promise<void> {
    offer = null;
    listing = false;
    unticked = new Set();
    try {
      if (shape && shape.fields !== (tab.info.usual_fields ?? 0)) {
        const same = await job("Looking for rows of the same shape", () => backend().sameShape(tab.id, tab.header, shape.fields, shape.groups));
        if (same?.total) offer = { ...shape, same };
      }
      const e = await job("Looking at the extra column", () => backend().emptyExtraColumns(tab.id, tab.header));
      extra = e ? { count: e[0], revision: e[1] } : { count: 0, revision: 0 };
    } catch (e) {
      problem = errorText(e);
    }
  }

  async function fixAll(): Promise<void> {
    const o = offer;
    if (!o || busy) return;
    busy = true;
    problem = "";
    try {
      const done = await job("Fixing rows", () => backend().fixAll(tab.id, tab.header, o.same.revision, o.fields, $state.snapshot(o.groups), [...unticked]));
      if (!done) return;
      const note = (done[1].run_notes ?? []).map((n) => ` ${n}.`).join("");
      onDone(done[1], `Joined the fields in ${done[0].toLocaleString()} ${done[0] === 1 ? "row" : "rows"}.${note}`);
      await afterRepair(null);
      const found = await list();
      if (found.length) await show({ id: found[0]! });
      else row = null;
    } catch (e) {
      problem = errorText(e);
      offer = null;
    } finally {
      busy = false;
    }
  }

  async function removeExtra(): Promise<void> {
    if (busy) return;
    busy = true;
    problem = "";
    try {
      const info = await job("Removing the empty column", () => backend().removeEmptyExtraColumns(tab.id, tab.header, extra.revision));
      if (!info) return;
      onDone(info, "Removed the empty column.");
      extra = { count: 0, revision: 0 };
      offer = null;
      await list();
      if (row) await show({ id: row.id });
    } catch (e) {
      problem = errorText(e);
    } finally {
      busy = false;
    }
  }

  const separator = $derived(tab.info.delimiter ?? ",");
  const at = $derived(row ? ids.indexOf(row.id) : -1);
  const drawn = $derived(row ? row.fields.slice(0, MAX_DRAWN) : []);
  const result = $derived.by(() => {
    if (!row) return [] as string[];
    if (mode === "next") return row.next ? nextPreview(row.fields, row.next.fields, between) : [];
    if (mode === "previous") return row.previous ? nextPreview(row.previous.fields, row.fields, between) : [];
    if (mode === "quote") return row.quoted_as_text;
    return joinPreview(row.fields, joined, separator);
  });
  const ready = $derived(
    !!row && !busy && (mode === "next" ? !!row.next : mode === "previous" ? !!row.previous : mode === "quote" ? row.quoted_as_text.length > 0 : joined.size > 0),
  );
  /** The row that keeps the result has a note matched on a column this repair changes. */
  const warning = $derived.by(() => {
    if (!row || !ready) return "";
    const kept = mode === "previous" ? row.previous : row;
    return kept ? noteWarning(kept.fields, result, kept.note_keys, (c) => titles[c] ?? `Column ${c + 1}`) : "";
  });

  const shown = (v: string) => {
    const t = v.replace(/\r?\n/g, "⏎").replace(/\t/g, "→");
    return t.length > 120 ? `${t.slice(0, 120)}…` : t;
  };

  async function show(wanted: { row?: number; id?: number }): Promise<void> {
    const mine = ++seq;
    try {
      const t = await backend().rowText(tab.id, wanted.row ?? null, wanted.id ?? null, tab.header);
      if (mine !== seq) return;
      row = t;
      joined = new Set();
      cursor = 0;
      mode = "join";
      problem = "";
      if (t.row !== null) onGo(t.row);
    } catch (e) {
      if (mine === seq) problem = errorText(e);
    }
  }

  async function list(): Promise<number[]> {
    const l = await backend().raggedList(tab.id, tab.header);
    ids = l.ids;
    total = l.total;
    return l.ids;
  }

  async function open(first: number | null): Promise<void> {
    try {
      const found = await list();
      if (first !== null) await show({ row: first });
      else if (found.length) await show({ id: found[0]! });
      else row = null;
    } catch (e) {
      problem = errorText(e);
    }
    await tick();
    box?.focus();
  }

  // Opened for this file; another file means another list. (Derived values: an effect runs again only when they
  // change, not whenever the tab is handed down anew, which would take the bar back to the row it was opened on.)
  const file = $derived(tab.id);
  $effect(() => {
    void file;
    untrack(() => void open(start));
  });
  // The table changed elsewhere (an edit, undo, another window): read the row again.
  const revision = $derived(`${tab.info.history_pos}/${tab.info.data_revision}`);
  $effect(() => {
    void revision;
    untrack(() => {
      if (row && !busy) void list().then(() => row && show({ id: row.id }));
    });
  });

  function step(by: 1 | -1): void {
    const to = stepRow(ids, row?.id ?? -1, Math.max(0, at), by);
    if (to !== null) void show({ id: to });
  }

  function toggle(i: number): void {
    if (!row || row.no_join) return;
    mode = "join";
    cursor = i;
    const next = new Set(joined);
    if (!next.delete(i)) next.add(i);
    joined = next;
  }

  async function apply(): Promise<void> {
    const r = row;
    if (!r || !ready) return;
    busy = true;
    problem = "";
    try {
      const repair: RowRepair =
        mode === "next"
          ? { kind: "next", between }
          : mode === "previous"
            ? { kind: "previous", between, header: tab.header }
            : mode === "quote"
              ? { kind: "quote_as_text" }
              : { kind: "join", groups: groups(r.fields.length, joined) };
      const place = Math.max(0, ids.indexOf(r.id));
      const info = await backend().repairRow(tab.id, r.id, r.revision, $state.snapshot(repair));
      const done = { next: "Joined the row with the next one.", previous: "Joined the row onto the previous one.", quote: "Read the quote marks of the row as text.", join: "Joined the fields of the row." };
      onDone(info, done[mode]);
      // Found again; stay on the next one (or this one while it is still ragged).
      const found = await list();
      const to = found.includes(r.id) ? r.id : stepRow(found, r.id, place, 1);
      if (to !== null) await show({ id: to });
      else row = null;
      await afterRepair(repair.kind === "join" ? { fields: r.width, groups: repair.groups } : null);
    } catch (e) {
      problem = errorText(e);
    } finally {
      busy = false;
      await tick();
      box?.focus();
    }
  }

  function key(e: KeyboardEvent): void {
    const control = (e.target as HTMLElement).closest("select, input, button");
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onClose();
    } else if (e.key === "ArrowUp" || e.key === "ArrowDown") {
      if (control?.tagName === "SELECT") return;
      e.preventDefault();
      step(e.key === "ArrowUp" ? -1 : 1);
    } else if ((e.key === "ArrowLeft" || e.key === "ArrowRight") && row) {
      if (control?.tagName === "SELECT") return;
      e.preventDefault();
      cursor = Math.min(Math.max(0, drawn.length - 2), Math.max(0, cursor + (e.key === "ArrowLeft" ? -1 : 1)));
    } else if (e.key === " " && !control && row) {
      e.preventDefault();
      toggle(cursor);
    } else if (e.key === "Enter" && !control) {
      e.preventDefault();
      void apply();
    }
    // (The grid behind must not take these keys.)
    if (!e.ctrlKey && !e.altKey && !e.metaKey) e.stopPropagation();
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<section class="repair" data-testid="repair" aria-label="Repair ragged rows" tabindex="0" bind:this={box} onkeydown={key}>
  <header>
    <strong>Repair rows</strong>
    <span class="muted" data-testid="repair-count">
      {#if !total}No row has a different number of fields.{:else if at >= 0}{at + 1} of {total.toLocaleString()} ragged rows{:else}{total.toLocaleString()} ragged rows{/if}
    </span>
    <button type="button" class="small" disabled={!ids.length || busy} onclick={() => step(-1)}>Previous</button>
    <button type="button" class="small" disabled={!ids.length || busy} onclick={() => step(1)}>Next</button>
    <span class="spacer"></span>
    <button type="button" class="small" onclick={onClose} aria-label="Close the repair bar">Close</button>
  </header>
  {#if row}
    <p data-testid="repair-heading">{rowHeading(row)}</p>
    {#if row.edited}
      <p class="note">This row has been changed; showing its cells.</p>
    {:else}
      <div class="raw" data-testid="repair-raw" title="The row as it is in the file">{row.raw}</div>
    {/if}
    {#if row.cut}<p class="note">Row cut at 64 KB: nothing past the cut can be joined.</p>{/if}
    {#if row.line_by_line}<p class="note">This file was opened line by line; the row is read the same way here.</p>{/if}
    {#if row.no_join}<p class="note">{row.no_join}</p>{/if}
    <div class="fields" data-testid="repair-fields">
      {#each drawn as v, i (i)}
        <span class="field">{shown(v)}</span>
        {#if i < drawn.length - 1}
          <button type="button" class="cut" class:joined={joined.has(i)} class:at={cursor === i} disabled={!!row.no_join} aria-pressed={joined.has(i)} aria-label={`${joined.has(i) ? "Split" : "Join"} fields ${i + 1} and ${i + 2}`} onclick={() => toggle(i)}>{joined.has(i) ? "⇔" : "┊"}</button>
        {/if}
      {/each}
      {#if row.fields.length > MAX_DRAWN}<span class="muted">and {(row.fields.length - MAX_DRAWN).toLocaleString()} more fields</span>{/if}
    </div>
    <div class="actions">
      <label><input type="radio" value="join" bind:group={mode} /> Join the fields marked ⇔</label>
      <label class:off={!row.next}><input type="radio" value="next" bind:group={mode} disabled={!row.next} /> Join with the next row, with
        <select aria-label="Put between the two rows" bind:value={between} onchange={() => (mode = mode === "previous" ? mode : "next")} disabled={!row.next && !row.previous}>
          <option value="space">a space</option>
          <option value="line_break">a line break</option>
          <option value="nothing">nothing</option>
        </select>
        between</label>
      <label class:off={!row.previous}><input type="radio" value="previous" bind:group={mode} disabled={!row.previous} /> Join with the previous row</label>
      {#if row.quoted_as_text.length}<label><input type="radio" value="quote" bind:group={mode} /> Treat the quote marks as text</label>{/if}
      {#if tab.info.unclosed_quotes || tab.info.long_quoted_cells}<button type="button" class="small" onclick={onReopenLines}>Reopen reading line by line</button>{/if}
    </div>
    {#if row.no_next}<p class="note">{row.no_next}</p>{/if}
    {#if row.no_previous}<p class="note">{row.no_previous}</p>{/if}
    {#if mode === "next" && row.next}<p class="note" data-testid="repair-next">{nextNotes(row.next)}</p>{/if}
    {#if mode === "previous" && row.previous}<p class="note" data-testid="repair-next">{previousNotes(row.previous, row.note_keys.length > 0)}</p>{/if}
    {#if warning}<p class="problem" data-testid="repair-warning">{warning}</p>{/if}
    <div class="result">
      <span class="muted">The row will be ({result.length.toLocaleString()} fields):</span>
      <table data-testid="repair-result">
        <tbody>
          <tr>{#each result.slice(0, MAX_DRAWN) as _, i (i)}<th>{titles[i] ?? `Column ${i + 1}`}</th>{/each}</tr>
          <tr>{#each result.slice(0, MAX_DRAWN) as v, i (i)}<td>{shown(v)}</td>{/each}</tr>
        </tbody>
      </table>
    </div>
    <div class="actions">
      <button type="button" class="primary" disabled={!ready} onclick={() => void apply()}>Apply</button>
      <span class="muted">Up and Down: other rows. Left and Right, then Space: join or split. Enter applies, Esc closes.</span>
    </div>
  {/if}
  {#if offer}
    <div class="offer" data-testid="repair-offer">
      <div class="actions">
        <span>{sameShapeWords(offer.same.total, offer.same.hidden, offer.fields)}</span>
        {#if !listing}<button type="button" onclick={() => (listing = true)}>Fix all like this…</button>{/if}
      </div>
      {#if listing}
        <p class="note">The same fields are joined in each. Untick any of the first {offer.same.first.length.toLocaleString()} to leave it as it is.</p>
        <ul>
          {#each offer.same.first as r (r.id)}
            <li><label><input type="checkbox" checked={!unticked.has(r.id)} onchange={(e) => { const n = new Set(unticked); if (e.currentTarget.checked) n.delete(r.id); else n.add(r.id); unticked = n; }} /> {r.file_row ? `Row ${r.file_row.toLocaleString()}` : "Added row"}: {r.cells.map(shown).join(" ┊ ")}</label></li>
          {/each}
        </ul>
        <button type="button" class="primary" disabled={busy || offer.same.total - unticked.size < 1} onclick={() => void fixAll()}>Fix {(offer.same.total - unticked.size).toLocaleString()} {offer.same.total - unticked.size === 1 ? "row" : "rows"}</button>
      {/if}
    </div>
  {/if}
  {#if extra.count}
    <div class="actions" data-testid="repair-extra">
      <span>{extra.count === 1 ? "The column past the usual number is empty and has no title." : `The ${extra.count} columns past the usual number are empty and have no titles.`}</span>
      <button type="button" disabled={busy} onclick={() => void removeExtra()}>{extra.count === 1 ? "Remove the empty last column" : "Remove the empty columns"}</button>
    </div>
  {/if}
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
</section>

<style>
  .repair { border-top: 1px solid var(--border); background: var(--panel, var(--bg)); padding: 6px 10px; max-height: 45vh; overflow: auto; font-size: 0.95em; }
  header, .actions { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .spacer { flex: 1; }
  button, select { font: inherit; color: inherit; background: var(--bg); border: 1px solid var(--border); border-radius: 4px; padding: 2px 10px; }
  button:hover:not(:disabled) { border-color: var(--accent); }
  button:disabled { opacity: 0.5; }
  button.primary { background: var(--accent); border-color: var(--accent); color: var(--accent-text, #fff); }
  p { margin: 4px 0; }
  .muted, .note { opacity: 0.75; }
  .raw { font-family: var(--mono, monospace); white-space: pre; overflow: auto; max-height: 4.5em; border: 1px solid var(--border); padding: 3px 6px; }
  .fields { display: flex; align-items: center; gap: 2px; overflow-x: auto; padding: 6px 0; }
  .field { font-family: var(--mono, monospace); white-space: pre; border: 1px solid var(--border); border-radius: 4px; padding: 1px 6px; }
  .cut { min-width: 1.8em; padding: 0 4px; }
  .cut.joined { background: var(--accent); color: var(--accent-text, #fff); }
  .cut.at { outline: 2px solid var(--accent); outline-offset: 1px; }
  label.off { opacity: 0.5; }
  .result { overflow-x: auto; margin: 4px 0; }
  table { border-collapse: collapse; }
  th, td { border: 1px solid var(--border); padding: 1px 8px; text-align: left; white-space: pre; font-weight: normal; }
  th { opacity: 0.7; }
  .problem { color: var(--danger, #b00020); }
  .offer { border-top: 1px solid var(--border); margin-top: 6px; padding-top: 4px; }
  ul { list-style: none; padding: 0; margin: 4px 0; max-height: 10em; overflow: auto; white-space: nowrap; }
</style>
