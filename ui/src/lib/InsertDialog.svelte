<script lang="ts">
  // Ask how many rows or columns to insert, and on which side of the selection.
  import { untrack } from "svelte";
  import FormShell from "./FormShell.svelte";
  import { insertCount } from "./insert";

  interface Props {
    kind: "rows" | "cols";
    /** Starting count (the selection's size). */
    count: number;
    onRun: (count: number, after: boolean) => void;
    onClose: () => void;
  }
  let { kind, count, onRun, onClose }: Props = $props();
  let text = $state(untrack(() => String(count)));
  let after = $state(false);
  let problem = $state("");

  let done = false;

  function run(): void {
    if (done) return;
    const n = insertCount(text, kind);
    if (typeof n === "string") {
      problem = n;
      return;
    }
    done = true;
    onRun(n, after);
  }
</script>

<FormShell title={kind === "rows" ? "Insert rows" : "Insert columns"} {onClose} onEnter={run}>
  <label class="row"
    >How many
    <input type="text" inputmode="numeric" bind:value={text} oninput={() => (problem = "")} onfocus={(e) => e.currentTarget.select()} />
  </label>
  <label class="row"><input type="radio" name="side" checked={!after} onchange={() => (after = false)} /> {kind === "rows" ? "Above the selection" : "Left of the selection"}</label>
  <label class="row"><input type="radio" name="side" checked={after} onchange={() => (after = true)} /> {kind === "rows" ? "Below the selection" : "Right of the selection"}</label>
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" class="primary" onclick={run}>Insert</button>
    <button type="button" onclick={onClose}>Cancel</button>
  {/snippet}
</FormShell>

<style>
  input[type="text"] {
    width: 8em;
    margin-left: 0.5em;
  }
  .problem {
    color: var(--danger, #e5534b);
    margin: 0.25em 0 0;
  }
</style>
