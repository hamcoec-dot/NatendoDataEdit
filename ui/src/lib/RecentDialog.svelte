<script lang="ts">
  // File > Open recent: the start page's recent files, at any time. Tick a
  // few and Open puts each in its own tab; a double-click opens one at once;
  // a middle-click opens one in a new tab and keeps the list open, so several
  // middle-clicks open several files.
  import FormShell from "./FormShell.svelte";
  import type { RecentItem } from "./views";

  interface Props {
    items: RecentItem[];
    /** The documents open in tabs (a file is open when its open_doc is one). */
    openDocs: Set<number>;
    onOpen: (items: RecentItem[]) => void;
    /** Middle-click: open this one, and leave the list open. */
    onOpenKeep: (item: RecentItem) => void;
    onClose: () => void;
  }
  let { items, openDocs, onOpen, onOpenKeep, onClose }: Props = $props();
  /** Ticked files by id: the list's order changes as files open, so ticks
   *  follow the file, not its place. */
  let picked = $state<Set<string>>(new Set());
  let list = $state<HTMLUListElement>();

  const isOpen = (r: RecentItem) => r.open_doc != null && openDocs.has(r.open_doc);
  function toggle(r: RecentItem): void {
    const next = new Set(picked);
    if (next.has(r.id)) next.delete(r.id);
    else next.add(r.id);
    picked = next;
  }
  /** Open what is ticked; with nothing ticked, the file that has focus. */
  function openPicked(): void {
    const ticked = items.filter((r) => picked.has(r.id));
    if (ticked.length) return onOpen(ticked);
    const id = (document.activeElement as HTMLElement | null)?.closest<HTMLElement>(".recent-list li")?.dataset.id;
    const r = items.find((x) => x.id === id);
    if (r) onOpen([r]);
  }
  /** Up and Down move between files, Home and End to the first and last. */
  function keys(e: KeyboardEvent): void {
    const boxes = [...(list?.querySelectorAll<HTMLInputElement>("input[type=checkbox]") ?? [])];
    const at = boxes.indexOf(document.activeElement as HTMLInputElement);
    const to = { ArrowDown: at + 1, ArrowUp: at - 1, Home: 0, End: boxes.length - 1 }[e.key];
    if (to === undefined || !boxes.length) return;
    e.preventDefault();
    const box = boxes[Math.max(0, Math.min(boxes.length - 1, to))]!;
    box.focus();
    box.scrollIntoView({ block: "nearest" });
  }
  function middle(e: MouseEvent, r: RecentItem): void {
    if (e.button !== 1) return;
    e.preventDefault();
    onOpenKeep(r);
  }
</script>

<FormShell title="Open recent" wide {onClose} onEnter={openPicked}>
  {#if items.length}
    <p class="note">Tick the files to open (each gets its own tab), double-click one, or middle-click files to open each in a new tab while this list stays open. Up and Down move; Enter opens.</p>
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <ul class="recent-list" aria-label="Recent files" bind:this={list} onkeydown={keys}>
      {#each items as r (r.id)}
        <li data-id={r.id}>
          <!-- Middle-click: mousedown is stopped so the browser does not start its scroll mode (the tick box and Open do the same from the keyboard). -->
          <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
          <label title={r.folder ? `${r.folder}\\${r.name}` : r.name} ondblclick={() => onOpen([r])} onmousedown={(e) => { if (e.button === 1) e.preventDefault(); }} onauxclick={(e) => middle(e, r)}>
            <input type="checkbox" checked={picked.has(r.id)} onchange={() => toggle(r)} />
            <span class="name">{r.name}</span>
            <span class="folder">{r.folder}</span>
            {#if isOpen(r)}<span class="open">open</span>{/if}
          </label>
        </li>
      {/each}
    </ul>
  {:else}
    <p class="note">No recent files yet. Files you open show here (private files never do).</p>
  {/if}
  {#snippet buttons()}
    <button type="button" onclick={onClose}>Cancel</button>
    <button type="button" class="primary" disabled={picked.size === 0} onclick={openPicked}>
      Open{picked.size > 1 ? ` ${picked.size} files` : ""}
    </button>
  {/snippet}
</FormShell>

<style>
  .note {
    margin: 0 0 8px;
    color: var(--muted);
  }
  .recent-list {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 60vh;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 6px;
  }
  label {
    display: flex;
    width: 100%;
    box-sizing: border-box;
    align-items: baseline;
    gap: 8px;
    padding: 6px 10px;
    cursor: pointer;
    min-width: 0;
  }
  label:hover,
  label:focus-within {
    background: var(--hover, rgba(127, 127, 127, 0.12));
  }
  label:focus-within {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .name {
    font-weight: 600;
    white-space: nowrap;
  }
  .folder {
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    flex: 1;
  }
  .open {
    font-size: 0.85em;
    color: var(--accent);
  }
</style>
