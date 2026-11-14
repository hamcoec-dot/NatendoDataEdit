<script lang="ts">
  // Pick a date from a calendar for the current cell (Milestone 12, the
  // ASAP "calendar pop-up"). The date is written in the style the cell
  // already uses (03/14/2026 stays month first), else as 2026-03-14.
  import { untrack } from "svelte";
  import { dateLike, isoOf } from "./editing";
  import FormShell from "./FormShell.svelte";

  interface Props {
    /** The cell's text now. */
    current: string;
    onPick: (text: string) => void;
    onClose: () => void;
  }
  let { current, onPick, onClose }: Props = $props();
  let value = $state(untrack(() => isoOf(current) ?? isoOf("") ?? ""));

  function pick(): void {
    if (/^\d{4}-\d{2}-\d{2}$/.test(value)) onPick(dateLike(current, value));
  }
</script>

<FormShell title="Pick a date" {onClose} onEnter={pick}>
  <!-- svelte-ignore a11y_autofocus -->
  <input type="date" bind:value autofocus aria-label="Date" />
  <p class="note">Written as {value ? dateLike(current, value) : "…"}. Today's date: Ctrl+;</p>
  {#snippet buttons()}
    <button type="button" class="primary" onclick={pick} disabled={!value}>OK</button>
    <button type="button" onclick={onClose}>Cancel</button>
  {/snippet}
</FormShell>
