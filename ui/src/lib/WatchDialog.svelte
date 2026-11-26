<script lang="ts">
  // Watched folder (Milestone 15): while the app is open, new table files
  // dropped into a folder are run through a saved recipe and written to a
  // results folder. The screen can be closed; watching goes on until
  // Stop or the app closes.
  import { onDestroy } from "svelte";
  import { backend, errorText, type ReceiptAsk } from "./backend";
  import ReceiptChoice from "./ReceiptChoice.svelte";
  import FormShell from "./FormShell.svelte";
  import type { WatchStatus } from "./watch";

  interface Props {
    onClose: () => void;
  }
  let { onClose }: Props = $props();

  let st = $state<WatchStatus | null>(null);
  let force = $state(false);
  let receipt = $state<ReceiptAsk | null>(null);
  let existing = $state(false);
  let problem = $state("");

  async function refresh(): Promise<void> {
    try {
      st = await backend().watchStatus();
    } catch (e) {
      problem = errorText(e);
    }
  }
  void refresh();
  const timer = window.setInterval(() => void refresh(), 2000);
  onDestroy(() => window.clearInterval(timer));

  async function pick(which: "folder" | "recipe" | "out"): Promise<void> {
    problem = "";
    try {
      st = await backend().watchPick(which);
    } catch (e) {
      problem = errorText(e);
    }
  }
  async function start(): Promise<void> {
    problem = "";
    try {
      st = await backend().watchStart(force, existing, receipt);
    } catch (e) {
      problem = errorText(e);
    }
  }
  async function stop(): Promise<void> {
    st = await backend().watchStop();
  }
  const ready = $derived(!!st?.folder && !!st?.recipe && !!st?.out);
</script>

<FormShell title="Watch a folder" wide {onClose}>
  <p class="note">While the app is open, every new CSV, TSV or text file put in the watched folder is run through a saved recipe (Tools ▸ Recipe) and the result is written, under the same name, to the results folder. The files put in are never changed. You can close this screen; watching goes on until you press Stop or close the app.</p>
  <div class="setup">
    <span>Watched folder</span><span class="path">{st?.folder ?? "not chosen"}</span><button type="button" disabled={st?.running} onclick={() => void pick("folder")}>Choose…</button>
    <span>Recipe</span><span class="path">{st?.recipe ?? "not chosen"}</span><button type="button" disabled={st?.running} onclick={() => void pick("recipe")}>Choose…</button>
    <span>Results folder</span><span class="path">{st?.out ?? "not chosen"}</span><button type="button" disabled={st?.running} onclick={() => void pick("out")}>Choose…</button>
  </div>
  <label class="row"><input type="checkbox" bind:checked={existing} disabled={st?.running} /> Also run the files already in the folder</label>
  <label class="row"><input type="checkbox" bind:checked={force} disabled={st?.running} /> Run even when a file's columns differ from the recipe's</label>
  <ReceiptChoice value={receipt} onChange={(v) => (receipt = v)} steps disabled={st?.running} />
  {#if st}
    <p><strong>{st.running ? "Watching" : "Not watching"}.</strong> {st.done.toLocaleString()} done, {st.failed.toLocaleString()} not done.</p>
    {#if st.log.length}
      <div class="log">{#each st.log as l, i (i)}<div>{l}</div>{/each}</div>
    {/if}
  {/if}
  {#if problem}<p class="warn">{problem}</p>{/if}
  {#snippet buttons()}
    {#if st?.running}
      <button type="button" class="primary" onclick={() => void stop()}>Stop</button>
    {:else}
      <button type="button" class="primary" data-enter disabled={!ready} onclick={() => void start()}>Start watching</button>
    {/if}
    <button type="button" onclick={onClose}>Close</button>
  {/snippet}
</FormShell>

<style>
  .setup {
    display: grid;
    grid-template-columns: 9em 1fr auto;
    gap: 6px;
    align-items: center;
  }
  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    opacity: 0.85;
  }
  .log {
    max-height: 200px;
    overflow: auto;
    font-family: var(--mono, monospace);
    font-size: 0.85em;
    border: 1px solid var(--border, #8884);
    border-radius: 4px;
    padding: 4px 8px;
  }
  .warn {
    color: var(--warn, #b45309);
  }
</style>
