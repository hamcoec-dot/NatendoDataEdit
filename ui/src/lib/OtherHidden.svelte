<script lang="ts">
  // A tool that reads another tab reads its shown rows only: say so when some of them are hidden.
  import type { OtherTable } from "./transfer";

  let { table, matching = false }: { table: OtherTable | undefined; matching?: boolean } = $props();
</script>

{#if table?.hidden}
  <p class="other-hidden" data-testid="other-hidden">
    {table.hidden.toLocaleString()} {table.hidden === 1 ? "row" : "rows"} hidden in {table.name} {table.hidden === 1 ? "is" : "are"} not read.{matching ? " A match in a hidden row counts as no match." : ""}
  </p>
{/if}

<style>
  .other-hidden {
    margin: 0.4rem 0;
    padding: 0.2rem 0.5rem;
    border-left: 3px solid #d98e04;
    background: rgb(217 142 4 / 0.16);
  }
</style>
