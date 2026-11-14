<script lang="ts">
  // Every keyboard shortcut, grouped by menu, with a search box (Ctrl+/ or
  // F1). Shows the keys in use now, including custom ones.
  import FormShell from "./FormShell.svelte";
  import type { KeyGroup } from "./m9";

  interface Props {
    groups: KeyGroup[];
    /** Keys the grid itself handles (not commands). */
    basics: { label: string; keys: string }[];
    onCustomize: () => void;
    onClose: () => void;
  }
  let { groups, basics, onCustomize, onClose }: Props = $props();
  let q = $state("");

  const norm = (s: string) => s.toLowerCase().replace(/&/g, "");
  const shown = $derived.by(() => {
    const t = norm(q.trim());
    const hit = (s: string) => !t || norm(s).includes(t);
    return groups
      .map((g) => ({ name: g.name, items: g.items.filter((i) => hit(i.label) || i.keys.some(hit)) }))
      .filter((g) => g.items.length);
  });
  const basicShown = $derived(basics.filter((b) => !q.trim() || norm(b.label).includes(norm(q.trim())) || norm(b.keys).includes(norm(q.trim()))));
</script>

<FormShell title="Keyboard shortcuts" wide {onClose}>
  <input class="search" type="search" placeholder="Search shortcuts (a command or a key, such as Ctrl+S)" aria-label="Search shortcuts" maxlength="100" bind:value={q} />
  <p class="note">Shortcuts follow Excel's where Excel has one.</p>
  <div class="cols">
    {#if basicShown.length}
      <section>
        <h3>Moving and typing</h3>
        <dl>
          {#each basicShown as b (b.label)}<dt>{b.label}</dt><dd><kbd>{b.keys}</kbd></dd>{/each}
        </dl>
      </section>
    {/if}
    {#each shown as g (g.name)}
      <section>
        <h3>{g.name}</h3>
        <dl>
          {#each g.items as i (i.label)}
            <dt>{i.label}</dt>
            <dd>{#each i.keys as k, n (k)}{#if n > 0}<span class="or"> or </span>{/if}<kbd>{k}</kbd>{/each}</dd>
          {/each}
        </dl>
      </section>
    {:else}
      {#if !basicShown.length}<p class="note">No shortcut matches “{q}”.</p>{/if}
    {/each}
  </div>
  {#snippet buttons()}
    <button type="button" onclick={onCustomize}>Customize shortcuts…</button>
    <button type="button" class="primary" onclick={onClose}>Close</button>
  {/snippet}
</FormShell>

<style>
  .search {
    width: 100%;
    box-sizing: border-box;
  }
  .note {
    color: var(--muted);
    font-size: 0.85rem;
  }
  .cols {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 22em), 1fr));
    align-items: start;
    gap: 4px 24px;
    max-height: 60vh;
    overflow-y: auto;
    overflow-x: hidden;
  }
  section {
    break-inside: avoid;
    margin-bottom: 12px;
  }
  h3 {
    font-size: 0.95rem;
    margin: 4px 0;
  }
  dl {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 2px 12px;
    margin: 0;
  }
  dt,
  dd {
    margin: 0;
    font-size: 0.88rem;
  }
  dd {
    text-align: right;
  }
  kbd {
    font-family: inherit;
    font-size: 0.8rem;
    border: 1px solid var(--border);
    border-bottom-width: 2px;
    border-radius: 4px;
    padding: 0 4px;
    background: var(--bg);
    white-space: nowrap;
  }
  .or {
    color: var(--muted);
    font-size: 0.8rem;
  }
</style>
