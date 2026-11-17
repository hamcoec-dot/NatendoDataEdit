<script lang="ts" module>
  export type ShiftChoice = "left" | "up" | "right" | "down" | "rows" | "cols";
</script>

<script lang="ts">
  // Insert or Delete on part of a row (Ctrl+Plus, Ctrl+Minus), as in Excel:
  // shift the other cells, or take the entire rows or columns.
  import { onMount, tick } from "svelte";
  import FormShell from "./FormShell.svelte";

  interface Props {
    /** Insert cells, or delete them. */
    insert: boolean;
    /** The selection's size, for the labels. */
    rows: number;
    cols: number;
    onRun: (choice: ShiftChoice) => void;
    onClose: () => void;
  }
  let { insert, rows, cols, onRun, onClose }: Props = $props();
  const plural = (n: number, w: string): string => `${n.toLocaleString()} ${w}${n === 1 ? "" : "s"}`;
  const options = $derived<[ShiftChoice, string][]>(
    insert
      ? [
          ["right", "Shift cells right"],
          ["down", "Shift cells down"],
          ["rows", `Entire row${rows === 1 ? "" : "s"} (${plural(rows, "row")})`],
          ["cols", `Entire column${cols === 1 ? "" : "s"} (${plural(cols, "column")})`],
        ]
      : [
          ["left", "Shift cells left"],
          ["up", "Shift cells up"],
          ["rows", `Entire row${rows === 1 ? "" : "s"} (${plural(rows, "row")})`],
          ["cols", `Entire column${cols === 1 ? "" : "s"} (${plural(cols, "column")})`],
        ],
  );
  let choice = $state<ShiftChoice>("rows");
  let box = $state<HTMLElement>();
  // The ticked choice has the keyboard focus, so arrow keys pick another.
  onMount(() => {
    void tick().then(() => setTimeout(() => box?.querySelector<HTMLInputElement>("input:checked")?.focus(), 0));
  });
  let done = false;

  function run(): void {
    if (done) return;
    done = true;
    onRun(choice);
  }
</script>

<FormShell title={insert ? "Insert" : "Delete"} {onClose} onEnter={run}>
  <fieldset class="choices" bind:this={box}>
    <legend>{insert ? "Insert" : "Delete"} {plural(rows * cols, "cell")}:</legend>
    {#each options as [value, label] (value)}
      <label>
        <input type="radio" name="shift" {value} bind:group={choice} />
        {label}
      </label>
    {/each}
  </fieldset>
  {#if choice === "up" || choice === "down"}
    <p class="warn" role="note">Shifting moves values into other rows: in a list of people, each person's cells in these columns stop lining up with the rest of their record.</p>
  {:else if choice === "left" || choice === "right"}
    <p class="warn" role="note">Shifting moves values into other columns: in these rows, values end up under the wrong headings.</p>
  {/if}
  {#snippet buttons()}
    <button type="button" onclick={onClose}>Cancel</button>
    <button type="button" class="primary" onclick={run}>OK</button>
  {/snippet}
</FormShell>

<style>
  .choices {
    border: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .choices legend {
    margin-bottom: 6px;
  }
  .warn {
    margin: 10px 0 0;
    max-width: 30em;
    color: var(--danger, #e5534b);
  }
</style>
