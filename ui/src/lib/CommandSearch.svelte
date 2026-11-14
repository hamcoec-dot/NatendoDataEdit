<script lang="ts">
  // Command search (Alt+Q): a small launcher. Type a few letters or words
  // (a command's name, an everyday word for it, or words from the README's
  // description) and the best matches show at once with where they live and
  // what they do. Arrows to pick, Enter or click to run, Esc to close.
  import { onMount, tick, untrack } from "svelte";
  import type { Command } from "./commands";
  import type { FeatureDoc } from "./featureIndex";
  import { prepare, rank } from "./launcher";

  interface Props {
    commands: Command[];
    keysOf: (id: string) => string[];
    enabled: (id: string) => boolean;
    onRun: (id: string) => void;
    onClose: () => void;
    /** README descriptions by command id. */
    docs?: Map<string, FeatureDoc>;
    /** Menu a command lives in, e.g. "Data". */
    menuOf?: (id: string) => string | undefined;
    /** Shortcut that opens this search, shown in the hint. */
    hotkey?: string;
    /** Called when the picked command can't run right now. */
    onUnavailable?: (name: string) => void;
    /** Text to start the search with. */
    initial?: string;
  }
  let { commands, keysOf, enabled, onRun, onClose, docs = new Map(), menuOf = () => undefined, hotkey = "Alt+Q", onUnavailable = () => {}, initial = "" }: Props = $props();

  let query = $state(untrack(() => initial));
  let index = $state(0);
  let input = $state<HTMLInputElement>();
  let list = $state<HTMLUListElement>();
  onMount(() => input?.focus());

  const plain = (l: string) => l.replace(/&/g, "").replace(/…$/, "");
  const prepared = $derived(
    prepare(
      commands.map((c) => {
        const d = docs.get(c.id);
        return { id: c.id, name: plain(c.label), menu: menuOf(c.id), docs: d?.text };
      }),
    ),
  );
  const found = $derived(rank(prepared, query, 50));

  function where(id: string): string {
    const m = menuOf(id);
    return m ? `${m} menu` : "";
  }

  async function show(i: number): Promise<void> {
    index = i;
    await tick();
    list?.querySelector<HTMLElement>(`[data-i="${i}"]`)?.scrollIntoView({ block: "nearest" });
  }

  function run(id: string, name: string): void {
    if (enabled(id)) onRun(id);
    else onUnavailable(name);
  }

  function key(e: KeyboardEvent): void {
    const last = found.length - 1;
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      void show(index >= last ? 0 : index + 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      void show(index <= 0 ? Math.max(0, last) : index - 1);
    } else if (e.key === "PageDown") {
      e.preventDefault();
      void show(Math.min(Math.max(0, last), index + 8));
    } else if (e.key === "PageUp") {
      e.preventDefault();
      void show(Math.max(0, index - 8));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const f = found[index];
      if (f) run(f.item.id, f.item.name);
    }
  }
</script>

<div class="backdrop" role="presentation" onclick={onClose}></div>
<div class="search" role="dialog" aria-label="Search for a command">
  <input
    bind:this={input}
    type="search"
    placeholder="What do you want to do? e.g. remove duplicates"
    bind:value={query}
    oninput={() => (index = 0)}
    onkeydown={key}
    aria-label="Command"
    aria-controls="command-results"
    aria-activedescendant={found[index] ? `cmd-${index}` : undefined}
    spellcheck="false"
    autocomplete="off"
  />
  <ul bind:this={list} id="command-results" role="listbox">
    {#each found as f, i (f.item.id)}
      {@const d = docs.get(f.item.id)}
      {@const off = !enabled(f.item.id)}
      <li role="option" id={`cmd-${i}`} aria-selected={i === index} data-i={i}>
        <button type="button" tabindex="-1" class:on={i === index} class:off aria-disabled={off} title={off ? "Not available right now (open a file first?)" : undefined} onclick={() => run(f.item.id, f.item.name)} onmousemove={() => (index = i)}>
          <span class="top">
            <span class="name">{f.item.name}</span>
            <span class="keys">{keysOf(f.item.id).join(", ")}</span>
          </span>
          {#if d || where(f.item.id)}
            <span class="desc">
              {#if where(f.item.id)}<span class="menu">{where(f.item.id)}</span>{/if}{#if d}{d.blurb}{/if}
            </span>
          {/if}
        </button>
      </li>
    {:else}
      <li class="none">Nothing matches “{query.trim()}”. Try another word.</li>
    {/each}
  </ul>
  <div class="foot">↑ ↓ to pick · Enter to open · Esc to close · {hotkey} opens this search</div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: rgb(0 0 0 / 0.12);
  }
  .search {
    position: fixed;
    top: 14vh;
    left: 50%;
    transform: translateX(-50%);
    width: min(40rem, 92vw);
    z-index: 41;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 10px;
    box-shadow: 0 12px 40px rgb(0 0 0 / 0.3);
    padding: 10px;
  }
  input {
    width: 100%;
    box-sizing: border-box;
    font-size: 1.1rem;
    padding: 8px 10px;
  }
  ul {
    list-style: none;
    margin: 8px 0 0;
    padding: 0;
    max-height: min(26rem, 60vh);
    overflow: auto;
  }
  button {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    background: transparent;
    border: none;
    padding: 5px 8px;
    text-align: left;
    border-radius: 6px;
    color: inherit;
    font: inherit;
  }
  button.on {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
  }
  button.off {
    opacity: 0.55;
  }
  .top {
    display: flex;
    justify-content: space-between;
    gap: 12px;
  }
  .name {
    font-weight: 500;
  }
  .desc {
    font-size: 0.85em;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .menu {
    display: inline-block;
    margin-right: 6px;
    padding: 0 5px;
    border-radius: 4px;
    border: 1px solid var(--line);
    font-size: 0.9em;
  }
  .keys,
  .none,
  .foot {
    color: var(--muted);
  }
  .keys {
    white-space: nowrap;
  }
  .none {
    padding: 8px;
  }
  .foot {
    margin-top: 6px;
    font-size: 0.8em;
    text-align: right;
  }
</style>
