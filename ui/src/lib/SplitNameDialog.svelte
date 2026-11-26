<script lang="ts">
  // Split a column of people's names into new columns (title, first,
  // middle, last, suffix), inserted to its right. One undo step.
  import HiddenNote from "./HiddenNote.svelte";
  import { untrack } from "svelte";
  import type { DataOp } from "./backend";
  import type { NamePart } from "./clean";
  import FormShell from "./FormShell.svelte";

  interface Props {
    col: number;
    titles: string[];
    header: boolean;
    onRun: (op: DataOp, header: boolean) => void;
    onClose: () => void;
  }
  let { col, titles, header, onRun, onClose }: Props = $props();

  const PARTS: [NamePart, string][] = [
    ["title", "Title (Dr, Ms, …)"],
    ["first", "First name"],
    ["middle", "Middle names"],
    ["last", "Last name"],
    ["suffix", "Suffix (Jr, PhD, …)"],
  ];
  let column = $state(untrack(() => col));
  let chosen = $state<NamePart[]>(["first", "middle", "last"]);
  let keep = $state(true);
  let myHeader = $state(untrack(() => header));
  let problem = $state("");

  function toggle(p: NamePart): void {
    chosen = chosen.includes(p) ? chosen.filter((x) => x !== p) : PARTS.map((x) => x[0]).filter((x) => x === p || chosen.includes(x));
  }

  function run(): void {
    if (!chosen.length) {
      problem = "Tick at least one part.";
      return;
    }
    onRun({ op: "split_name", spec: { col: column, parts: [...chosen], keep_original: keep, keep_first: myHeader } }, myHeader);
  }
</script>

<FormShell title="Split names into columns" {onClose} onEnter={run}>
  <label class="row"
    >Names in
    <select bind:value={column}>
      {#each titles as t, c (c)}<option value={c}>{t}</option>{/each}
    </select>
  </label>
  <fieldset>
    <legend>New columns</legend>
    {#each PARTS as [p, label] (p)}
      <label class="row"><input type="checkbox" checked={chosen.includes(p)} onchange={() => toggle(p)} /> {label}</label>
    {/each}
  </fieldset>
  <label class="row"><input type="checkbox" bind:checked={keep} /> Keep the name column (otherwise the first part takes its place)</label>
  <label class="row"><input type="checkbox" bind:checked={myHeader} /> The first row holds titles</label>
  <p class="note">Reads "Dr. Mary Ann van der Berg Jr." and "Smith, John". Every row is split, hidden ones too.</p>
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  <HiddenNote kind="all" />
  {#snippet buttons()}
    <button type="button" class="primary" onclick={run}>Split</button>
    <button type="button" onclick={onClose}>Cancel</button>
  {/snippet}
</FormShell>

<style>
  .row {
    display: flex;
    gap: 0.5rem;
    align-items: center;
  }
  .note {
    color: var(--muted);
    font-size: 0.85rem;
  }
  .problem {
    color: var(--danger, #c00);
  }
</style>
