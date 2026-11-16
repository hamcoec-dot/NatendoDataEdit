<script lang="ts">
  // File > Open recent: the start page's recent files, at any time. Tick a
  // few and Open puts each in its own tab; a double-click opens one at once.
  import FormShell from "./FormShell.svelte";
  import type { RecentItem } from "./views";

  interface Props {
    items: RecentItem[];
    /** Names of the files already open in a tab (folder\name, lower case). */
    openPaths: Set<string>;
    onOpen: (items: RecentItem[]) => void;
    onClose: () => void;
  }
  let { items, openPaths, onOpen, onClose }: Props = $props();
  let picked = $state<Set<number>>(new Set());

  const key = (r: RecentItem) => `${r.folder}\\${r.name}`.toLowerCase();
  function toggle(r: RecentItem): void {
    const next = new Set(picked);
    if (next.has(r.index)) next.delete(r.index);
    else next.add(r.index);
    picked = next;
  }
  function openPicked(): void {
    const list = items.filter((r) => picked.has(r.index));
    if (list.length) onOpen(list);
  }
</script>

<FormShell title="Open recent" wide {onClose} onEnter={openPicked}>
  {#if items.length}
    <p class="note">Tick the files to open (each gets its own tab), or double-click one.</p>
    <ul class="recent-list" aria-label="Recent files">
      {#each items as r (r.index)}
        <li>
          <label title={`${r.folder}\\${r.name}`} ondblclick={() => onOpen([r])}>
            <input type="checkbox" checked={picked.has(r.index)} onchange={() => toggle(r)} />
            <span class="name">{r.name}</span>
            <span class="folder">{r.folder}</span>
            {#if openPaths.has(key(r))}<span class="open">open</span>{/if}
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
  label:hover {
    background: var(--hover, rgba(127, 127, 127, 0.12));
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
