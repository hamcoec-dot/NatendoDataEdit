<script lang="ts">
  // Custom shortcuts: give any command keys of your own. Excel's keys stay
  // the defaults; a key already in use asks which command keeps it.
  import { untrack } from "svelte";
  import { keyOf, type Command } from "./commands";
  import FormShell from "./FormShell.svelte";
  import { keyProblem, MAX_BINDINGS, type Shortcuts } from "./views";

  interface Props {
    commands: Command[];
    custom: Shortcuts;
    onSave: (custom: Shortcuts) => void;
    onClose: () => void;
  }
  let { commands, custom, onSave, onClose }: Props = $props();

  let mine = $state<Shortcuts>(untrack(() => JSON.parse(JSON.stringify(custom)) as Shortcuts));
  let query = $state("");
  let listening = $state<string | null>(null);
  let problem = $state("");

  const plain = (l: string) => l.replace(/&/g, "").replace(/…$/, "");
  const keysOf = (c: Command) => mine[c.id] ?? c.keys ?? [];
  const shown = $derived(commands.filter((c) => plain(c.label).toLowerCase().includes(query.trim().toLowerCase())).slice(0, 300));

  function owner(k: string, except: string): Command | undefined {
    return commands.find((c) => c.id !== except && keysOf(c).includes(k));
  }

  function capture(e: KeyboardEvent, id: string): void {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      listening = null;
      return;
    }
    const k = keyOf(e);
    if (["Ctrl", "Alt", "Shift", "Ctrl+Shift", "Ctrl+Alt", "Alt+Shift", "Ctrl+Alt+Shift", ""].includes(k)) return;
    const bad = keyProblem(k);
    if (bad) {
      problem = bad;
      return;
    }
    const other = owner(k, id);
    if (other && !confirm(`${k} runs “${plain(other.label)}”. Give it to this command instead?`)) return;
    if (other) mine[other.id] = keysOf(other).filter((x) => x !== k);
    const c = commands.find((x) => x.id === id);
    if (!c) return;
    const total = Object.values(mine).reduce((n, ks) => n + ks.length, 0);
    if (total >= MAX_BINDINGS) {
      problem = `At most ${MAX_BINDINGS} custom keys.`;
      return;
    }
    mine[id] = [...keysOf(c).filter((x) => x !== k), k].slice(-4);
    listening = null;
    problem = "";
  }
</script>

<FormShell title="Keyboard shortcuts" wide {onClose} onEnter={() => onSave($state.snapshot(mine) as Shortcuts)}>
  <input type="search" placeholder="Find a command" bind:value={query} aria-label="Find a command" />
  <div class="list">
    <table>
      <thead><tr><th>Command</th><th>Keys</th><th></th></tr></thead>
      <tbody>
        {#each shown as c (c.id)}
          <tr>
            <td>{plain(c.label)}</td>
            <td>
              {#if listening === c.id}
                <input class="listen" readonly value="Press the keys…" onkeydown={(e) => capture(e, c.id)} onblur={() => (listening = null)} use:focus />
              {:else}
                {keysOf(c).join(", ") || "—"}
              {/if}
            </td>
            <td class="act">
              <button type="button" onclick={() => (listening = c.id)}>Add key</button>
              <button type="button" disabled={!keysOf(c).length} onclick={() => (mine[c.id] = [])}>Clear</button>
              <button type="button" disabled={!(c.id in mine)} onclick={() => { delete mine[c.id]; mine = { ...mine }; }}>Default</button>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" class="primary" onclick={() => onSave($state.snapshot(mine) as Shortcuts)}>Save</button>
    <button type="button" onclick={() => (mine = {})}>All defaults</button>
    <button type="button" onclick={onClose}>Cancel</button>
  {/snippet}
</FormShell>

<script lang="ts" module>
  export function focus(node: HTMLElement): void {
    node.focus();
  }
</script>

<style>
  .list {
    max-height: 26rem;
    overflow: auto;
    margin-top: 6px;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th,
  td {
    text-align: left;
    padding: 2px 6px;
    border-bottom: 1px solid var(--line);
  }
  .act {
    white-space: nowrap;
  }
  .listen {
    width: 10rem;
  }
  .problem {
    color: var(--danger, #c00);
  }
</style>
