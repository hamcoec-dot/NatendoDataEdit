<script lang="ts">
  // Ask for one line of text (a view's name, a font name), with a check.
  import { onMount, untrack } from "svelte";
  import FormShell from "./FormShell.svelte";

  interface Props {
    title: string;
    label: string;
    value: string;
    note?: string;
    /** A problem with the text, or "" when it is fine. */
    check?: (v: string) => string;
    onDone: (v: string) => void;
    onClose: () => void;
  }
  let { title, label, value, note = "", check = () => "", onDone, onClose }: Props = $props();
  let text = $state(untrack(() => value));
  let problem = $state("");
  let input = $state<HTMLInputElement>();
  onMount(() => input?.select());

  function done(): void {
    const p = check(text);
    if (p) {
      problem = p;
      return;
    }
    onDone(text.trim());
  }
</script>

<FormShell {title} {onClose} onEnter={done}>
  <label class="row">{label} <input bind:this={input} type="text" maxlength="200" bind:value={text} /></label>
  {#if note}<p class="note">{note}</p>{/if}
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" class="primary" onclick={done}>OK</button>
    <button type="button" onclick={onClose}>Cancel</button>
  {/snippet}
</FormShell>

<style>
  .row {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .row input {
    flex: 1;
  }
  .note {
    color: var(--muted);
    font-size: 0.85rem;
  }
  .problem {
    color: var(--danger, #c00);
  }
</style>
