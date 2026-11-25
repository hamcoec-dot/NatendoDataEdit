<script lang="ts">
  // Checkpoints: named moments in the undo history, listed at the top of the Steps panel. A checkpoint is a name
  // and a mark (a few bytes): nothing of the table is copied. Click one to go back (or forward) to that exact
  // state in one jump; the moment you left is kept as "Before restoring ..." so the way back is one click.
  import { backend, errorText, type Checkpoint } from "./backend";
  import type { Tab } from "./tab.svelte";

  interface Props {
    tab: Tab;
    /** Go to a checkpoint (the screen does what Ctrl+Z does for every step crossed). */
    onRestore: (c: Checkpoint) => Promise<void>;
  }
  let { tab, onRestore }: Props = $props();

  let list = $state<Checkpoint[]>([]);
  let problem = $state("");
  let naming = $state(false);
  let name = $state("");
  let open = $state(true);

  async function refresh(): Promise<void> {
    try {
      list = await backend().checkpointList(tab.id);
    } catch {
      list = [];
    }
  }
  $effect(() => {
    void tab.version;
    void tab.checkpointTick;
    void refresh();
  });

  async function add(): Promise<void> {
    const n = name.trim();
    if (!n) return;
    try {
      list = await backend().checkpointAdd(tab.id, n);
      name = "";
      naming = false;
      problem = "";
    } catch (e) {
      problem = errorText(e);
    }
  }
  async function restore(c: Checkpoint): Promise<void> {
    try {
      await onRestore(c);
      problem = "";
    } catch (e) {
      problem = errorText(e);
    }
    await refresh();
  }
  async function forget(c: Checkpoint): Promise<void> {
    try {
      list = await backend().checkpointRemove(tab.id, c.id);
    } catch (e) {
      problem = errorText(e);
    }
  }
  function key(e: KeyboardEvent): void {
    e.stopPropagation();
    if (e.key === "Enter") {
      e.preventDefault();
      void add();
    } else if (e.key === "Escape") {
      e.preventDefault();
      naming = false;
    }
  }
</script>

<section class="checkpoints" data-testid="checkpoints" aria-label="Checkpoints">
  <div class="head">
    <button type="button" class="fold" aria-expanded={open} onclick={() => (open = !open)}>{open ? "▾" : "▸"} Checkpoints{list.length ? ` (${list.length})` : ""}</button>
    <span class="spacer"></span>
    <button type="button" class="small" title="Name this moment (Ctrl+Alt+K): remember exactly how the table looks now" aria-label="Name this moment" disabled={tab.info.rows === 0 && !tab.info.indexed} onclick={() => (naming = true)}>🔖 Name this moment</button>
  </div>
  {#if open}
    {#if naming}
      <div class="naming">
        <input type="text" aria-label="Checkpoint name" placeholder="Name, for example before the cleanup" maxlength="100" bind:value={name} onkeydown={key} />
        <button type="button" class="small primary" disabled={!name.trim()} onclick={() => void add()}>Save</button>
        <button type="button" class="small" onclick={() => (naming = false)}>Cancel</button>
      </div>
    {/if}
    {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
    {#if list.length === 0}
      <p class="note">No checkpoints yet. Name a moment, and the app also names one before risky changes (Replace all, remove duplicates, deleting rows, clean-ups, a big paste, joins).</p>
    {:else}
      <ul>
        {#each [...list].reverse() as c (c.id)}
          <li class:gone={!c.reachable} class:now={c.current} data-testid="checkpoint">
            <button type="button" class="go" disabled={!c.reachable || c.current} title={c.reachable ? (c.current ? "The table is in this state now" : "Go back (or forward) to this moment") : "No longer reachable: the undo history was trimmed past it, a newer change replaced it, or the file was saved"} onclick={() => void restore(c)}>
              <span class="n">{c.name}</span>
              <span class="tag">{c.current ? "now" : !c.reachable ? "no longer reachable" : c.auto ? "automatic" : ""}</span>
            </button>
            <button type="button" class="x" aria-label={`Forget ${c.name}`} title="Forget this checkpoint (the table does not change)" onclick={() => void forget(c)}>✕</button>
          </li>
        {/each}
      </ul>
      <p class="note">A checkpoint goes when the undo history is trimmed past it (10,000 steps or 512 MB), when a new change replaces what you undid, and when you save the file (saving starts the undo history again). They are not saved to disk and go when the tab closes.</p>
    {/if}
  {/if}
</section>

<style>
  .checkpoints {
    border-bottom: 1px solid var(--border);
    padding-bottom: 6px;
    margin-bottom: 8px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .spacer {
    flex: 1;
  }
  .fold {
    background: none;
    border: none;
    color: var(--text);
    font: inherit;
    font-weight: 600;
    cursor: pointer;
    padding: 0;
  }
  .naming {
    display: flex;
    gap: 6px;
    margin: 6px 0;
  }
  .naming input {
    flex: 1;
    min-width: 0;
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 2px 6px;
  }
  ul {
    list-style: none;
    margin: 6px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  li {
    display: flex;
    gap: 4px;
    align-items: center;
  }
  li.gone .go {
    opacity: 0.5;
  }
  li.now .go {
    border-color: var(--accent);
  }
  .go {
    flex: 1;
    min-width: 0;
    display: flex;
    justify-content: space-between;
    gap: 8px;
    text-align: left;
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 3px 8px;
    cursor: pointer;
  }
  .go:disabled {
    cursor: default;
  }
  .n {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tag {
    color: var(--muted);
    font-size: 11px;
    white-space: nowrap;
  }
  .x {
    background: none;
    border: none;
    color: var(--muted);
    cursor: pointer;
  }
  .note {
    color: var(--muted);
    font-size: 12px;
    margin: 6px 0 0;
  }
  .problem {
    color: #c0392b;
    margin: 6px 0 0;
  }
</style>
