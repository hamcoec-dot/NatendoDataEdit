<script lang="ts">
  // Unsaved work kept from a run that stopped without saving, or the
  // earlier saves of the current file. Either opens as a new tab; nothing
  // is written over a file until the user saves.
  import FormShell from "./FormShell.svelte";
  import type { RecoveryItem, VersionItem } from "./m9";

  interface Props {
    mode: "recover" | "versions";
    /** For earlier saves: the file's name. */
    name?: string;
    items: (RecoveryItem | VersionItem)[] | null;
    onOpen: (index: number, label: string) => void;
    onDiscard?: (index: number) => void;
    onClose: () => void;
  }
  let { mode, name = "", items, onOpen, onDiscard, onClose }: Props = $props();

  function when(secs: number): string {
    if (!secs) return "an unknown time";
    return new Date(secs * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
  }
  function size(b: number): string {
    return b >= 1 << 20 ? `${(b / (1 << 20)).toFixed(1)} MB` : `${Math.max(1, Math.round(b / 1024))} KB`;
  }
</script>

<FormShell title={mode === "recover" ? "Recover unsaved work" : `Earlier saves of ${name}`} wide {onClose}>
  {#if !items}
    <p class="note">Looking…</p>
  {:else if !items.length}
    <p>{mode === "recover" ? "There is no unsaved work to recover." : "No earlier saves are kept for this file yet. While “Keep earlier saves” is on in Options, each save keeps the file as it was before here (files up to 64 MB, the last 20 saves, for 30 days)."}</p>
  {:else}
    <p class="note">
      {mode === "recover"
        ? "Natendo Data Edit stopped before these changes were saved. Open one to get it back as a new tab, then save it where you want; your original file has not been changed."
        : "Each of these is the file as it was before one of your saves. Open one to look at it in a new tab; save it under a name to keep it."}
    </p>
    <ul class="items">
      {#each items as it (it.index)}
        <li>
          <div class="text">
            {#if "title" in it}
              <div class="title">{it.title}</div>
              <div class="sub">{it.path ? `From ${it.path} · ` : ""}kept {when(it.saved)} · {it.rows.toLocaleString()} rows · {size(it.bytes)}</div>
            {:else}
              <div class="title">Saved {when(it.saved)}</div>
              <div class="sub">{size(it.bytes)}</div>
            {/if}
          </div>
          <button type="button" class="primary" onclick={() => onOpen(it.index, `saved ${when(it.saved)}`)}>Open</button>
          {#if onDiscard}<button type="button" onclick={() => onDiscard(it.index)}>Discard</button>{/if}
        </li>
      {/each}
    </ul>
  {/if}
  {#snippet buttons()}
    <button type="button" onclick={onClose}>{mode === "recover" && items?.length ? "Decide later" : "Close"}</button>
  {/snippet}
</FormShell>

<style>
  .note {
    color: var(--muted);
    font-size: 0.85rem;
    max-width: 44em;
  }
  .items {
    width: min(44rem, 80vw);
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 55vh;
    overflow-y: auto;
  }
  li {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 8px 0;
    border-bottom: 1px solid var(--border);
  }
  .text {
    flex: 1;
    min-width: 0;
  }
  .title {
    font-weight: 600;
  }
  .sub {
    font-size: 0.85rem;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
