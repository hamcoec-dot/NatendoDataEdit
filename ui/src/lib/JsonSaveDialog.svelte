<script lang="ts">
  // C11: the first JSON save with a newly blank cell asks how blank cells are written (leave the key
  // out is the default, D-C11). Blanks the file already had keep their own form; this is only about
  // cells made blank here, new rows and new columns.
  import { untrack } from "svelte";
  import type { JsonBlanks } from "./backend";
  import FormShell from "./FormShell.svelte";

  interface Props {
    /** The file being saved (its name only is shown). */
    file: string;
    /** Preset: the answer remembered for this file, else leave out. */
    start: JsonBlanks;
    /** A private file: the answer is not remembered. */
    privateFile?: boolean;
    onSave: (blanks: JsonBlanks, remember: boolean) => void;
    onClose: () => void;
  }
  let { file, start, privateFile = false, onSave, onClose }: Props = $props();
  let blanks = $state<JsonBlanks>(untrack(() => start));
  let remember = $state(true);

  const BLANK_CHOICES: [JsonBlanks, string, string][] = [
    ["leave_out", "Leave the key out", "The record has no such key (the usual way in JSON Lines)."],
    ["empty_text", "Empty text \"\"", "The key is there with an empty string."],
    ["null", "null", "The key is there with the value null."],
  ];

  function save(): void {
    onSave(blanks, remember && !privateFile);
  }
</script>

<FormShell title="Blank cells in JSON" {onClose} onEnter={save}>
  <p class="note">Some cells you made blank, or cells in new rows or columns, have no form in {file.split(/[\\/]/).pop()} yet. Blanks that were already in the file are written as they were.</p>
  <fieldset>
    <legend>Write a new blank cell as</legend>
    {#each BLANK_CHOICES as [v, label, about] (v)}
      <label class="row" title={about}>
        <input type="radio" name="json-blanks" value={v} bind:group={blanks} />
        {label}<span class="sub">{about}</span>
      </label>
    {/each}
  </fieldset>
  {#if privateFile}
    <p class="note">This file is private, so the answer is not remembered.</p>
  {:else}
    <label class="row"><input type="checkbox" bind:checked={remember} /> Remember for this file</label>
  {/if}
  {#snippet buttons()}
    <button type="button" class="primary" onclick={save}>Save</button>
    <button type="button" onclick={onClose}>Cancel</button>
  {/snippet}
</FormShell>

