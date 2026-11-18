<script lang="ts">
  // A zip file holding several tables: pick which to open (each in its own
  // tab). Tables are unpacked in memory only; the zip is never rewritten.
  import { untrack } from "svelte";
  import FormShell from "./FormShell.svelte";
  import type { ZipList } from "./backend";

  interface Props {
    list: ZipList;
    onOpen: (names: string[]) => void;
    onClose: () => void;
  }
  let { list, onOpen, onClose }: Props = $props();
  /** Most tables opened from one zip at once (as the backend allows). */
  const MAX_PICK = 32;
  let picked = $state<Set<string>>(untrack(() => new Set(list.entries.slice(0, 1).map((e) => e.name))));
  const size = (n: number) => (n >= 1 << 20 ? `${(n / (1 << 20)).toFixed(1)} MB` : n >= 1024 ? `${Math.round(n / 1024)} KB` : `${n} bytes`);
  function toggle(name: string, on: boolean): void {
    const s = new Set(picked);
    if (on) s.add(name);
    else s.delete(name);
    picked = s;
  }
  let done = false;
  function run(all = false): void {
    if (done) return;
    const names = all ? list.entries.map((e) => e.name) : list.entries.filter((e) => picked.has(e.name)).map((e) => e.name);
    if (!names.length || names.length > MAX_PICK) return;
    done = true;
    onOpen(names);
  }
</script>

<FormShell title={`Open from ${list.name}`} {onClose} onEnter={() => run()}>
  <p class="note">This zip file holds {list.entries.length} tables. Each opens in its own tab, unpacked in memory only; to keep changes, save the tab as a file of its own (the zip is never changed).</p>
  <div class="list" role="group" aria-label="Tables in the zip">
    {#each list.entries as e (e.name)}
      <label class="row"><input type="checkbox" checked={picked.has(e.name)} onchange={(ev) => toggle(e.name, (ev.currentTarget as HTMLInputElement).checked)} /> <span class="name">{e.name}</span> <span class="muted">{size(e.size)}</span></label>
    {/each}
  </div>
  {#if picked.size > MAX_PICK}<p class="warn" role="note">Open at most {MAX_PICK} tables at once.</p>{/if}
  {#snippet buttons()}
    <button type="button" onclick={onClose}>Cancel</button>
    {#if list.entries.length <= MAX_PICK}<button type="button" onclick={() => run(true)}>Open all {list.entries.length}</button>{/if}
    <button type="button" class="primary" onclick={() => run()} disabled={!picked.size || picked.size > MAX_PICK}>Open {picked.size === 1 ? "it" : picked.size}</button>
  {/snippet}
</FormShell>

<style>
  .list {
    max-height: 320px;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 8px 0;
  }
  .row {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .name {
    flex: 1;
    overflow-wrap: anywhere;
  }
  .muted {
    color: var(--muted);
    font-size: 12px;
  }
  .note {
    margin: 0;
  }
  .warn {
    color: var(--danger, #c0392b);
  }
</style>
