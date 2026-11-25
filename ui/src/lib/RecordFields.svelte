<script lang="ts">
  // One row as a list of fields (title, value), shared by the Record panel and the Form view. It only shows and
  // edits what it is given: the caller reads the row and saves. Past `WINDOW_FROM` fields only the ones in view are
  // drawn (every field has the same height; a long value opens in an expander instead of growing the field).
  import { untrack } from "svelte";

  interface Props {
    /** Column numbers to show, in order. */
    order: number[];
    titles: string[];
    values: string[];
    original: string[];
    /** Why a field cannot be edited ("" when it can). */
    reason?: (col: number) => string;
    dimmed?: (col: number) => boolean;
    pinned?: (col: number) => boolean;
    onPin?: (col: number) => void;
    onJump?: (col: number) => void;
    /** Enter or leaving the field with a changed value. */
    onCommit?: (col: number) => void;
    /** Escape in a field with a changed value. */
    onRevert?: (col: number) => void;
    /** Open the long-value editor for this field. */
    onExpand?: (col: number) => void;
    onEditing?: (col: number | null) => void;
    /** Draw only the fields in view. */
    windowed?: boolean;
  }
  let { order, titles, values = $bindable(), original, reason, dimmed, pinned, onPin, onJump, onCommit, onRevert, onExpand, onEditing, windowed = false }: Props = $props();

  /** Height of one field, pixels (windowed lists only). */
  const ITEM = 30;
  const WINDOW_FROM = 100;
  let scroller = $state<HTMLDivElement>();
  let top = $state(0);
  let view = $state(400);
  const useWindow = $derived(windowed && order.length > WINDOW_FROM);
  const first = $derived(useWindow ? Math.max(0, Math.floor(top / ITEM) - 4) : 0);
  const last = $derived(useWindow ? Math.min(order.length, Math.ceil((top + view) / ITEM) + 4) : order.length);
  const slice = $derived(order.slice(first, last));
  const LONG = 80;

  const name = (c: number) => titles[c] || `Column ${c + 1}`;
  function onScroll(): void {
    if (scroller) top = scroller.scrollTop;
  }
  $effect(() => {
    const el = scroller;
    if (!el) return;
    untrack(() => (view = el.clientHeight || 400));
    const ro = new ResizeObserver(() => (view = el.clientHeight || 400));
    ro.observe(el);
    return () => ro.disconnect();
  });

  function key(e: KeyboardEvent, c: number): void {
    const el = e.currentTarget as HTMLInputElement;
    if (e.key === "Enter") {
      e.preventDefault();
      if (values[c] !== original[c]) onCommit?.(c);
    } else if (e.key === "Escape" && values[c] !== original[c]) {
      e.preventDefault();
      e.stopPropagation();
      values[c] = original[c] ?? "";
      onRevert?.(c);
    } else if ((e.key === "ArrowDown" || e.key === "ArrowUp") && !e.altKey && !e.ctrlKey && !e.shiftKey) {
      e.preventDefault();
      const fields = Array.from(el.closest(".fields")?.querySelectorAll<HTMLElement>("input.value") ?? []);
      const at = fields.indexOf(el);
      fields[at + (e.key === "ArrowDown" ? 1 : -1)]?.focus();
    }
  }
</script>

<div class="fields" class:windowed={useWindow} bind:this={scroller} onscroll={onScroll} role="list">
  {#if useWindow}<div style:height="{first * ITEM}px" aria-hidden="true"></div>{/if}
  {#each slice as c (c)}
    {@const why = reason?.(c) ?? ""}
    <div class="field" class:dim={dimmed?.(c)} class:fixed={useWindow} role="listitem" data-col={c}>
      {#if onPin}
        <button type="button" class="star" class:on={pinned?.(c)} aria-pressed={pinned?.(c) ?? false} title={pinned?.(c) ? "Unpin this field" : "Pin this field to the top"} aria-label={pinned?.(c) ? `Unpin ${name(c)}` : `Pin ${name(c)} to the top`} onclick={() => onPin(c)}>{pinned?.(c) ? "★" : "☆"}</button>
      {/if}
      {#if onJump}
        <button type="button" class="name link" title="Go to this cell in the grid" onclick={() => onJump(c)}>{name(c)}</button>
      {:else}
        <span class="name" title={name(c)}>{name(c)}</span>
      {/if}
      <input
        class="value"
        class:changed={values[c] !== original[c]}
        type="text"
        aria-label={name(c)}
        disabled={why !== ""}
        title={why}
        bind:value={values[c]}
        onfocus={() => onEditing?.(c)}
        onblur={() => {
          onEditing?.(null);
          if (values[c] !== original[c]) onCommit?.(c);
        }}
        onkeydown={(e) => key(e, c)}
      />
      {#if onExpand && ((values[c] ?? "").length > LONG || (values[c] ?? "").includes("\n"))}
        <button type="button" class="more" title="Open this long value in a bigger editor" aria-label={`Open ${name(c)} in a bigger editor`} onclick={() => onExpand(c)}>⤢</button>
      {/if}
    </div>
  {/each}
  {#if useWindow}<div style:height="{Math.max(0, order.length - last) * ITEM}px" aria-hidden="true"></div>{/if}
</div>

<style>
  .fields {
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-height: 0;
    flex: 1;
  }
  .fields.windowed {
    gap: 0;
  }
  .field {
    display: grid;
    grid-template-columns: auto minmax(70px, 34%) 1fr auto;
    gap: 6px;
    align-items: center;
  }
  .field.fixed {
    height: 30px;
    box-sizing: border-box;
  }
  .field.dim {
    opacity: 0.55;
  }
  .name {
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: left;
  }
  .link {
    background: none;
    border: none;
    padding: 0;
    cursor: pointer;
    font: inherit;
  }
  .link:hover {
    text-decoration: underline;
    color: var(--text);
  }
  .value {
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 2px 6px;
    min-width: 0;
  }
  .value.changed {
    border-color: var(--accent);
  }
  .value:disabled {
    opacity: 0.6;
  }
  .star,
  .more {
    font: inherit;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--muted);
    padding: 0 2px;
  }
  .star.on {
    color: var(--accent);
  }
  .field:not(:has(.star)) {
    grid-template-columns: minmax(70px, 34%) 1fr auto;
  }
</style>
