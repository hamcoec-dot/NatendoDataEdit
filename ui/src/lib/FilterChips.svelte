<script lang="ts">
  // The filter chips bar (C8): one chip per column rule and one per tool that showed rows. Click a chip to switch
  // it off or on, × takes it out; its right-click menu inverts, edits, renames, takes out, shows a formula's error
  // rows or runs its tool again. Match all or Match any joins them; Invert all shows the other rows. Each change is
  // one undo step and changes no data.
  import { tick } from "svelte";
  import { backend, errorText, type ChipChange, type ChipInfo, type DocInfo, type FilterChips, type FilterSpec } from "./backend";
  import { describeRule } from "./filters";
  import type { Tab } from "./tab.svelte";

  interface Props {
    tab: Tab;
    /** A column's title as the grid shows it. */
    title: (col: number) => string;
    /** A change was made (the tab's new facts, and what to say). */
    onDone: (info: DocInfo, what: string) => void;
    /** Edit a column chip's rule (the column's filter menu). */
    onEdit: (col: number) => void;
    /** Run a tool chip's tool again (null: it cannot be, and why). */
    runAgain: (chip: ChipInfo) => { run: () => void } | { why: string };
    /** Say something without a change (a refusal). */
    onSay: (text: string) => void;
  }
  let { tab, title, onDone, onEdit, runAgain, onSay }: Props = $props();

  let chips = $state<FilterChips | null>(null);
  let spec = $state<FilterSpec | null>(null);
  let busy = $state(false);
  let menu = $state<{ i: number; x: number; y: number; hot: number } | null>(null);
  let renaming = $state<{ i: number; text: string } | null>(null);
  let bar: HTMLElement | undefined = $state();
  let seq = 0;

  // Read the chips again after any change to the table, the filter or undo.
  const key = $derived(`${tab.id}:${tab.info.history_pos}:${tab.info.rows}:${tab.info.total_rows}:${tab.info.filtered}:${tab.info.has_filter}`);
  $effect(() => {
    void key;
    const n = ++seq;
    void Promise.all([backend().filterChips(tab.id), backend().filterSpec(tab.id)])
      .then(([c, s]) => {
        if (n !== seq) return;
        chips = c;
        spec = s;
      })
      .catch(() => {
        if (n === seq) chips = null;
      });
  });

  /** A chip's words: its name, else its column and rule, else what its tool showed. */
  function text(c: ChipInfo, i: number): string {
    if (c.name) return c.name;
    if (c.kind === "column") {
      const r = spec?.columns[i]?.rule;
      const col = c.col === null ? "A deleted column" : title(c.col);
      return r ? `${col} ${describeRule(r)}` : col;
    }
    return c.label;
  }

  function count(c: ChipInfo): string {
    return c.rows === null ? "" : c.rows.toLocaleString();
  }

  function tip(c: ChipInfo, i: number): string {
    const parts = [`${c.negate ? "Not: " : ""}${text(c, i)}`];
    if (c.rows !== null) parts.push(`${c.rows.toLocaleString()} ${c.rows === 1 ? "row" : "rows"} on its own`);
    if (c.kind !== "column" && c.not_looked) parts.push(`${c.not_looked.toLocaleString()} ${c.not_looked === 1 ? "row was" : "rows were"} hidden when it was made, so it says nothing of them`);
    if (c.errors) parts.push(`${c.errors.toLocaleString()} ${c.errors === 1 ? "row" : "rows"} gave an error (neither shown nor hidden by it)`);
    if (c.stale) parts.push("made before your last edit: Run again from its right-click menu");
    parts.push(c.enabled ? "click to switch it off" : "switched off: click to switch it on");
    return parts.join("; ");
  }

  function status(f: FilterChips | null): string {
    if (!f) return "";
    const rows = `${f.shown.toLocaleString()} of ${f.total.toLocaleString()} rows`;
    if (f.all_off) return `All chips off: every row shows (${rows}).`;
    if (f.shown === 0) return `No row matches (0 of ${f.total.toLocaleString()} rows).`;
    return `${rows} show.`;
  }

  async function change(c: ChipChange, what: string): Promise<void> {
    if (busy) return;
    busy = true;
    try {
      const info = await backend().filterChange(tab.id, c);
      const next = await backend().filterChips(tab.id).catch(() => null);
      onDone(info, `${what} ${next ? status(next) : ""} Undo (Ctrl+Z) puts it back.`.replace(/\s+/g, " ").trim());
    } catch (e) {
      onSay(errorText(e));
    } finally {
      busy = false;
    }
  }

  function quote(c: ChipInfo, i: number): string {
    const t = text(c, i);
    return `“${t.length > 50 ? `${t.slice(0, 49)}…` : t}”`;
  }

  const toggle = (c: ChipInfo, i: number) => change({ op: "toggle", index: i }, `Switched the chip ${quote(c, i)} ${c.enabled ? "off" : "on"}.`);
  const remove = (c: ChipInfo, i: number) => change({ op: "remove", index: i }, `Took out the chip ${quote(c, i)}.`);
  const negate = (c: ChipInfo, i: number) =>
    change({ op: "negate", index: i }, c.negate ? `The chip ${quote(c, i)} shows its own rows again.` : `The chip ${quote(c, i)} now shows the rows it does not match.`);

  function openMenu(e: MouseEvent | KeyboardEvent, i: number): void {
    e.preventDefault();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const at = e instanceof MouseEvent && e.clientX ? { x: e.clientX, y: e.clientY } : { x: r.left, y: r.bottom + 2 };
    menu = { i, x: at.x, y: at.y, hot: 0 };
    void tick().then(() => bar?.ownerDocument.querySelector<HTMLButtonElement>(".chip-menu button:not(:disabled)")?.focus());
  }

  function closeMenu(): void {
    const i = menu?.i;
    menu = null;
    if (i !== undefined) void tick().then(() => bar?.querySelector<HTMLButtonElement>(`[data-chip="${i}"]`)?.focus());
  }

  /** A menu entry: run it while the menu's chip is still the one picked, then close. */
  function fromMenu(f: () => void): void {
    f();
    menu = null;
  }

  function menuKeys(e: KeyboardEvent): void {
    const items = [...((e.currentTarget as HTMLElement).querySelectorAll<HTMLButtonElement>("button:not(:disabled)"))];
    const at = items.indexOf(document.activeElement as HTMLButtonElement);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const n = items.length;
      items[(at + (e.key === "ArrowDown" ? 1 : n - 1)) % n]?.focus();
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closeMenu();
    }
  }

  function chipKeys(e: KeyboardEvent, c: ChipInfo, i: number): void {
    if (e.key === "Delete" || e.key === "Backspace") {
      e.preventDefault();
      void remove(c, i);
    } else if ((e.key === "F10" && e.shiftKey) || e.key === "ContextMenu") {
      openMenu(e, i);
    } else if (e.key === "F2") {
      e.preventDefault();
      startRename(c, i);
    }
  }

  function startRename(c: ChipInfo, i: number): void {
    renaming = { i, text: c.name ?? text(c, i) };
    void tick().then(() => bar?.querySelector<HTMLInputElement>(".rename")?.select());
  }

  function endRename(save: boolean): void {
    const r = renaming;
    renaming = null;
    if (!r || !chips) return;
    const c = chips.chips[r.i];
    if (!c || !save) {
      void tick().then(() => bar?.querySelector<HTMLButtonElement>(`[data-chip="${r.i}"]`)?.focus());
      return;
    }
    const name = r.text.trim();
    const words = c.kind === "column" && spec?.columns[r.i] ? `${c.col === null ? "A deleted column" : title(c.col)} ${describeRule(spec.columns[r.i]!.rule)}` : c.label;
    void change({ op: "rename", index: r.i, name: name && name !== words ? name : null }, name && name !== words ? `Named the chip “${name}”.` : "The chip has its own words again.");
  }
</script>

<svelte:window
  onpointerdown={(e) => {
    if (menu && !(e.target as HTMLElement | null)?.closest?.(".chip-menu")) menu = null;
  }}
/>

{#if chips && chips.chips.length}
  <div class="chips" role="toolbar" aria-label="Filter chips" bind:this={bar} data-testid="filter-chips">
    <span class="lead">Filter</span>
    <div class="list">
      {#each chips.chips as c, i (i)}
        <span class="chip" class:off={!c.enabled} class:not={c.negate} class:stale={c.stale} class:tool={c.kind !== "column"}>
          {#if renaming?.i === i}
            <input
              class="rename"
              aria-label="Name for this chip"
              maxlength="80"
              bind:value={renaming.text}
              onkeydown={(e) => {
                if (e.key === "Enter") endRename(true);
                else if (e.key === "Escape") {
                  e.stopPropagation();
                  endRename(false);
                }
              }}
              onblur={() => endRename(true)}
            />
          {:else}
            <button
              type="button"
              class="body"
              data-chip={i}
              aria-pressed={c.enabled}
              aria-keyshortcuts="Delete Shift+F10 F2"
              title={tip(c, i)}
              disabled={busy}
              onclick={() => void toggle(c, i)}
              oncontextmenu={(e) => openMenu(e, i)}
              onkeydown={(e) => chipKeys(e, c, i)}
            >
              {#if c.negate}<span class="neg">not</span>{/if}
              <span class="words">{text(c, i)}</span>
              {#if c.stale}<span class="mark" aria-label="made before your last edit">⟳</span>{/if}
              {#if count(c)}<span class="n">{count(c)}</span>{/if}
            </button>
          {/if}
          <button type="button" class="x" aria-label={`Take out the chip ${text(c, i)}`} title="Take out this chip" disabled={busy} onclick={() => void remove(c, i)}>×</button>
        </span>
      {/each}
    </div>
    <div class="join" role="group" aria-label="How the chips join">
      <button type="button" aria-pressed={chips.combine === "and"} disabled={busy} title="A row shows when every chip that is on lets it through" onclick={() => chips?.combine !== "and" && void change({ op: "combine", combine: "and" }, "Match all: a row shows when every chip lets it through.")}>Match all</button>
      <button type="button" aria-pressed={chips.combine === "or"} disabled={busy} title="A row shows when any chip that is on lets it through" onclick={() => chips?.combine !== "or" && void change({ op: "combine", combine: "or" }, "Match any: a row shows when any chip lets it through.")}>Match any</button>
    </div>
    <button type="button" class="inv" aria-pressed={chips.invert} disabled={busy} title="Show the rows the chips hide, and hide the rows they show" onclick={() => void change({ op: "invert", on: !chips?.invert }, chips?.invert ? "Invert all is off." : "Invert all: the rows the chips hid show now.")}>Invert all</button>
    <span class="count" class:none={chips.shown === 0 && !chips.all_off} role="status" data-testid="chips-count">{status(chips)}</span>
  </div>
{/if}

{#if menu && chips?.chips[menu.i]}
  {@const i = menu?.i ?? 0}
  {@const c = chips.chips[i]!}
  {@const again = runAgain(c)}
  <div class="chip-menu" role="menu" tabindex="-1" aria-label={`Chip ${text(c, i)}`} style:left="{menu?.x ?? 0}px" style:top="{menu?.y ?? 0}px" onkeydown={menuKeys}>
    <button type="button" role="menuitem" onclick={() => fromMenu(() => void toggle(c, i))}>{c.enabled ? "Switch off" : "Switch on"}</button>
    <button type="button" role="menuitem" onclick={() => fromMenu(() => void negate(c, i))}>{c.negate ? "Show the rows it matches" : "Invert: show the rows it does not match"}</button>
    <button type="button" role="menuitem" disabled={c.kind !== "column" || c.col === null} title={c.kind !== "column" ? "A tool made this chip: run the tool again to change it" : c.col === null ? "Its column was deleted" : undefined} onclick={() => fromMenu(() => c.col !== null && onEdit(c.col))}>Edit the rule…</button>
    <button type="button" role="menuitem" onclick={() => fromMenu(() => startRename(c, i))}>Rename…</button>
    <button type="button" role="menuitem" onclick={() => fromMenu(() => void remove(c, i))}>Take out</button>
    <button type="button" role="menuitem" disabled={!c.errors} title={c.errors ? undefined : c.kind === "formula" ? "No row gave an error" : "Only a formula's chip has error rows"} onclick={() => fromMenu(() => void change({ op: "show_errors", index: i }, `Showing the ${c.errors.toLocaleString()} ${c.errors === 1 ? "row" : "rows"} that gave an error, as a chip of its own.`))}>Show the error rows</button>
    <button type="button" role="menuitem" disabled={"why" in again} title={"why" in again ? again.why : undefined} onclick={() => fromMenu(() => "run" in again && again.run())}>Run again</button>
  </div>
{/if}

<style>
  .chips {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    font-size: 12px;
    min-height: 30px;
    flex-wrap: wrap;
  }
  .lead {
    color: var(--muted);
    font-weight: 600;
  }
  .list {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    min-width: 0;
  }
  .chip {
    display: inline-flex;
    align-items: stretch;
    border: 1px solid var(--accent);
    border-radius: 12px;
    overflow: hidden;
    max-width: 320px;
  }
  .chip.tool {
    border-style: dashed;
  }
  .chip.off {
    border-color: var(--border);
    opacity: 0.7;
  }
  .chip.off .words {
    text-decoration: line-through;
  }
  .chip button {
    border: 0;
    background: none;
    color: var(--text);
    font: inherit;
    padding: 2px 6px;
    cursor: pointer;
  }
  .chip .body {
    display: inline-flex;
    gap: 4px;
    align-items: center;
    min-width: 0;
    padding-left: 9px;
  }
  .chip .words {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .neg {
    font-weight: 700;
    color: var(--accent);
  }
  .n {
    color: var(--muted);
  }
  .mark {
    color: #b7791f;
  }
  .chip .x {
    color: var(--muted);
    padding-right: 8px;
  }
  .chip button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .rename {
    font: inherit;
    border: 0;
    padding: 2px 8px;
    width: 180px;
    background: var(--bg);
    color: var(--text);
  }
  .join {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: 4px;
    overflow: hidden;
  }
  .join button,
  .inv {
    font: inherit;
    border: 0;
    background: none;
    color: var(--text);
    padding: 2px 8px;
    cursor: pointer;
  }
  .inv {
    border: 1px solid var(--border);
    border-radius: 4px;
  }
  .join button[aria-pressed="true"],
  .inv[aria-pressed="true"] {
    background: var(--accent);
    color: var(--accent-ink);
  }
  .count {
    color: var(--muted);
    margin-left: auto;
  }
  .count.none {
    color: #b7791f;
    font-weight: 600;
  }
  .chip-menu {
    position: fixed;
    z-index: 50;
    display: flex;
    flex-direction: column;
    min-width: 220px;
    padding: 4px 0;
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: 0 4px 14px rgb(0 0 0 / 0.18);
    font-size: 13px;
  }
  .chip-menu button {
    text-align: left;
    border: 0;
    background: none;
    color: var(--text);
    padding: 4px 12px;
    font: inherit;
    cursor: pointer;
  }
  .chip-menu button:hover:not(:disabled),
  .chip-menu button:focus-visible {
    background: var(--header);
    outline: none;
  }
  .chip-menu button:disabled {
    color: var(--muted);
    cursor: default;
  }
</style>
