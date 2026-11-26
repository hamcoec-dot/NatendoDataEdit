<script lang="ts">
  // A banner under the tabs (never a window over the table) saying another program changed or removed the
  // active tab's file.
  import { diskText, type DiskNote } from "./disk";

  interface Props {
    name: string;
    note: DiskNote;
    onReload: () => void;
    onCompare: () => void;
    onSaveAs: () => void;
    onIgnore: () => void;
    onMute: () => void;
  }
  let { name, note, onReload, onCompare, onSaveAs, onIgnore, onMute }: Props = $props();
</script>

<div class="diskbar" role="status" data-testid="disk-banner">
  <span class="text">{diskText(name, note)}</span>
  {#if note.state === "changed"}
    <button type="button" onclick={onReload}>Reload</button>
    <button type="button" onclick={onCompare} title="Open the file as it is on disk and the differences from what you opened">Compare with disk</button>
  {:else}
    <button type="button" onclick={onSaveAs}>Save As…</button>
  {/if}
  <button type="button" onclick={onIgnore} title="Hide this until the file changes again">Ignore</button>
  <button type="button" onclick={onMute}>Don't warn for this file</button>
</div>

<style>
  .diskbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    background: color-mix(in srgb, var(--warn, #b45309) 14%, var(--bg));
  }
  .diskbar button {
    padding: 3px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    color: inherit;
    font: inherit;
    cursor: pointer;
  }
  .diskbar button:hover {
    border-color: var(--accent);
  }
  .text {
    flex: 1 1 260px;
  }
</style>
