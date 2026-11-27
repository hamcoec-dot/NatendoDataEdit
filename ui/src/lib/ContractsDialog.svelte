<script lang="ts">
  // C9: the saved file contracts (in the app's data folder): check this tab against one, change or rename,
  // delete, export to a file and import from one.
  import type { ContractItem } from "./backend";
  import FormShell from "./FormShell.svelte";

  interface Props {
    items: ContractItem[];
    /** A tab to check against one ("" when none). */
    tabName: string;
    onCheck: (name: string) => void;
    onEdit: (name: string) => void;
    onDelete: (name: string) => void;
    onExport: (name: string) => void;
    onImport: () => void;
    onClose: () => void;
  }
  let { items, tabName, onCheck, onEdit, onDelete, onExport, onImport, onClose }: Props = $props();
  let pick = $state<string | null>(null);
  const picked = $derived(items.find((i) => i.name === pick) ?? null);
  const ok = $derived(!!picked && !picked.problem);
</script>

<FormShell title="File contracts" {onClose} onEnter={() => { if (ok && tabName && pick) onCheck(pick); }}>
  {#if items.length === 0}
    <p class="note">No contracts yet. Open a file of the kind you get regularly and use Data › Make a contract from this file.</p>
  {:else}
    <div class="list" role="listbox" aria-label="Contracts">
      {#each items as it (it.file)}
        <button type="button" role="option" aria-selected={pick === it.name} class:on={pick === it.name} onclick={() => (pick = it.name)}>
          <b>{it.name}</b>
          {#if it.problem}<span class="sub problem">cannot be read: {it.problem}</span>
          {:else}<span class="sub">{it.columns} columns{it.pattern ? `, files like ${it.pattern}` : ""}</span>{/if}
        </button>
      {/each}
    </div>
  {/if}
  <p class="note">A contract keeps column titles, types, blank shares, its checks and any allowed values ticked when it was made: those can be values from the file. Export writes all of it to the file you pick.</p>
  {#snippet buttons()}
    <button type="button" class="primary" disabled={!ok || !tabName} title={!tabName ? "Open a tab first" : ok ? `Check ${tabName} against it` : "Pick a contract"} onclick={() => pick && onCheck(pick)}>Check this tab against it</button>
    <button type="button" disabled={!ok} title="Its name, other titles, kept values and row range" onclick={() => pick && onEdit(pick)}>Change or rename…</button>
    <button type="button" disabled={!picked} onclick={() => pick && onDelete(pick)}>Delete</button>
    <button type="button" disabled={!ok} onclick={() => pick && onExport(pick)}>Export…</button>
    <button type="button" onclick={onImport}>Import…</button>
    <button type="button" onclick={onClose}>Close</button>
  {/snippet}
</FormShell>

<style>
  .list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: 45vh;
    overflow: auto;
  }
  .list button {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    padding: 5px 8px;
    border: 1px solid transparent;
    border-radius: 6px;
    background: none;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .list button.on {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, var(--bg));
  }
  .problem {
    color: var(--danger, #b91c1c);
  }
</style>
