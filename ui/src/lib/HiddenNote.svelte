<script lang="ts">
  // What a dialog says about hidden rows. Nothing while every row shows.
  import { hiddenRows, hiddenWords, type HiddenKind } from "./shown.svelte";

  let { kind }: { kind: HiddenKind } = $props();
  const words = $derived(hiddenWords(kind, hiddenRows.hidden, hiddenRows.total));
</script>

{#if words}
  <p class="hidden-note" class:all={kind === "all"} data-testid="hidden-note">{words}</p>
{/if}

<style>
  .hidden-note {
    margin: 0.4rem 0;
    padding: 0.2rem 0.5rem;
    border-left: 3px solid var(--border, #8886);
  }
  /* Rows you cannot see are changed: amber. */
  .all {
    border-left-color: #d98e04;
    background: rgb(217 142 4 / 0.16);
  }
</style>
