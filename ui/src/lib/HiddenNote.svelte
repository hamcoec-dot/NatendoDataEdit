<script lang="ts">
  // What a dialog says about hidden rows. Nothing while every row shows. Either one of the two fixed kinds, or the
  // dialog's own words (`text`), amber when hidden rows are included.
  import { hiddenRows, hiddenWords, includeWhile, type HiddenKind } from "./shown.svelte";

  let { kind, text = "", amber = false, label = "" }: { kind?: HiddenKind; text?: string; amber?: boolean; label?: string } = $props();
  const words = $derived(text || (kind ? hiddenWords(kind, hiddenRows.hidden, hiddenRows.total) : ""));
  $effect(() => includeWhile(kind === "all"));
</script>

{#if words}
  <p class="hidden-note" class:all={amber || kind === "all"} data-testid="hidden-note">{label ? `${label}: ` : ""}{words}</p>
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
