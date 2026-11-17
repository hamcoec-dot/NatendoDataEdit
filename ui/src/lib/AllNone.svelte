<script lang="ts">
  // "Select all" and "Unselect all" for a list of column tick boxes. Column
  // lists start with nothing ticked; these tick or clear them all at once.
  import { tick } from "svelte";

  interface Props {
    /** How many columns the list shows. */
    count: number;
    picked: number[];
    onChange: (picked: number[]) => void;
    /** At most this many can be ticked (Select all ticks the first ones). */
    max?: number;
    /** Columns the list leaves out. */
    skip?: (col: number) => boolean;
  }
  let { count, picked, onChange, max, skip }: Props = $props();
  let selectBtn = $state<HTMLButtonElement>();
  let clearBtn = $state<HTMLButtonElement>();
  const all = $derived.by(() => {
    const out: number[] = [];
    for (let c = 0; c < count && (max === undefined || out.length < max); c++) if (!skip?.(c)) out.push(c);
    return out;
  });
  const allTicked = $derived(all.length > 0 && all.every((c) => picked.includes(c)));

  // The button pressed then waits (disabled), which would drop the keyboard
  // focus out of the dialog (and Esc with it): hand it to the other one.
  function choose(v: number[], next: HTMLButtonElement | undefined): void {
    onChange(v);
    void tick().then(() => next?.focus());
  }
</script>

<div class="all-none">
  <button type="button" bind:this={selectBtn} disabled={allTicked} onclick={() => choose(all, clearBtn)}>Select all</button>
  <button type="button" bind:this={clearBtn} disabled={picked.length === 0} onclick={() => choose([], selectBtn)}>Unselect all</button>
  {#if max !== undefined && max < count}<span class="limit">(at most {max})</span>{/if}
</div>

<style>
  .all-none {
    display: flex;
    gap: 6px;
    align-items: center;
    margin: 2px 0 6px;
  }
  .all-none button {
    padding: 1px 8px;
    font-size: 0.9em;
  }
  .limit {
    color: var(--muted);
    font-size: 0.9em;
  }
</style>
