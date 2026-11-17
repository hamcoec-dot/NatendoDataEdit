<script lang="ts">
  // Rename a column: its title is the header row's cell. A file without a
  // header row gets one (the other columns keep their numbers as titles).
  import { untrack } from "svelte";
  import FormShell from "./FormShell.svelte";

  interface Props {
    /** The column's number, from 1, for the title bar. */
    number: number;
    /** Its title now ("" when it has none). */
    name: string;
    /** Whether the file has a header row. */
    header: boolean;
    /** Other columns' titles, lower case (to warn about a repeat). */
    others: Set<string>;
    onRun: (name: string) => void;
    onClose: () => void;
  }
  let { number, name, header, others, onRun, onClose }: Props = $props();
  let text = $state(untrack(() => name));
  let done = false;
  const repeat = $derived(text.trim() !== "" && others.has(text.trim().toLowerCase()));
  const lineBreak = $derived(/[\r\n]/.test(text));

  function run(): void {
    if (done || lineBreak) return;
    done = true;
    onRun(text);
  }
</script>

<FormShell title={`Rename column ${number}`} {onClose} onEnter={run}>
  <label class="row"
    >Name
    <!-- svelte-ignore a11y_autofocus -->
    <input
      type="text"
      class="name"
      bind:value={text}
      autofocus
      spellcheck="false"
      maxlength="32768"
      onfocus={(e) => e.currentTarget.select()}
      onkeydown={(e) => { if (e.key === "Enter") { e.preventDefault(); run(); } }}
    />
  </label>
  {#if header && !name && !text}<p class="note">Column {number} has no name yet.</p>{/if}
  {#if !header}<p class="note">This file has no header row, so one is added above the first row and "First row is header" is switched on: this column gets the name and the others keep their numbers. Undoing it (Ctrl+Z twice) takes the row away and switches the setting off again.</p>{/if}
  {#if repeat}<p class="note warn" role="status">Another column already has this name.</p>{/if}
  {#if lineBreak}<p class="note warn" role="alert">A column name can't contain a line break.</p>{/if}
  {#snippet buttons()}
    <button type="button" onclick={onClose}>Cancel</button>
    <button type="button" class="primary" disabled={lineBreak} onclick={run}>Rename</button>
  {/snippet}
</FormShell>

<style>
  .name {
    width: 22em;
    margin-left: 0.5em;
  }
  .note {
    margin: 0.5em 0 0;
    color: var(--muted);
    max-width: 30em;
  }
  .warn {
    color: var(--danger, #e5534b);
  }
</style>
