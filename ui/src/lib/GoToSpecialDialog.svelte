<script lang="ts" module>
  export type SpecialPick = "blanks" | "numbers" | "text" | "row_diff" | "edited" | "problems";
  export type SpecialAction = "light" | "fill" | "clear";
  export interface SpecialChoice {
    pick: SpecialPick;
    /** Row differences: case and spaces count. */
    exact: boolean;
    /** Only the selected cells (or the whole table). */
    inSelection: boolean;
    action: SpecialAction;
    value: string;
  }
</script>

<script lang="ts">
  // Edit › Go to special (Ctrl+Shift+G): pick the blank, number, text,
  // changed or failing cells, or the cells that differ from another column
  // of their row, and light them up, fill them or clear them in one step.
  import { untrack } from "svelte";
  import FormShell from "./FormShell.svelte";

  interface Props {
    /** Cells selected (more than one offers "the selection"). */
    selected: number;
    /** Title of the active cell's column (row differences compare with it). */
    against: string;
    /** Data checks are set up for this tab. */
    hasChecks: boolean;
    readOnly: boolean;
    last: SpecialChoice | null;
    onRun: (choice: SpecialChoice) => void;
    onClose: () => void;
  }
  let { selected, against, hasChecks, readOnly, last, onRun, onClose }: Props = $props();

  // The dialog starts from the last choices (it is opened anew each time).
  const start = untrack(() => ({ last, many: selected > 1, readOnly, hasChecks }));
  const first = start.last?.pick === "problems" && !start.hasChecks ? "blanks" : (start.last?.pick ?? "blanks");
  let pick = $state<SpecialPick>(first);
  let exact = $state(start.last?.exact ?? false);
  let inSelection = $state(start.many && (start.last?.inSelection ?? true));
  let action = $state<SpecialAction>(start.readOnly ? "light" : (start.last?.action ?? "light"));
  let value = $state(start.last?.value ?? "");

  const picks: [SpecialPick, string][] = [
    ["blanks", "Blank cells (empty or only spaces)"],
    ["numbers", "Cells with numbers"],
    ["text", "Cells with text"],
    ["row_diff", "Row differences"],
    ["edited", "Cells changed since the file was opened or saved"],
    ["problems", "Cells that fail a data check"],
  ];
  let done = false;
  function run(): void {
    if (done) return;
    if (action === "fill" && value === "") return;
    done = true;
    onRun({ pick, exact, inSelection, action, value });
  }
</script>

<FormShell title="Go to special" {onClose} onEnter={run}>
  <fieldset class="choices">
    <legend>Pick</legend>
    {#each picks as [v, label] (v)}
      <label class:off={v === "problems" && !hasChecks}>
        <input type="radio" name="pick" value={v} bind:group={pick} disabled={v === "problems" && !hasChecks} />
        {label}{#if v === "problems" && !hasChecks}<span class="muted"> (no data checks set up)</span>{/if}
      </label>
      {#if v === "row_diff" && pick === "row_diff"}
        <div class="sub">
          <span class="muted">Cells that differ from <b>{against}</b> in the same row.</span>
          <label><input type="checkbox" bind:checked={exact} /> Case and spaces count</label>
        </div>
      {/if}
    {/each}
  </fieldset>
  <fieldset class="choices">
    <legend>Look in</legend>
    <label><input type="radio" name="where" value={true} bind:group={inSelection} disabled={selected <= 1} /> The selection{selected > 1 ? ` (${selected.toLocaleString()} cells)` : ""}</label>
    <label><input type="radio" name="where" value={false} bind:group={inSelection} /> The whole table</label>
    <span class="muted">Shown rows and columns only; the title row is left out.</span>
  </fieldset>
  <fieldset class="choices">
    <legend>Then</legend>
    <label><input type="radio" name="act" value="light" bind:group={action} /> Light them up and go to the first one</label>
    <label class="fill">
      <input type="radio" name="act" value="fill" bind:group={action} disabled={readOnly} /> Fill them with
      <input aria-label="Fill them with" class="value" bind:value disabled={readOnly} onfocus={() => (action = "fill")} placeholder="UNKNOWN" />
    </label>
    <label><input type="radio" name="act" value="clear" bind:group={action} disabled={readOnly} /> Clear them</label>
    {#if action !== "light"}<span class="muted">One undo step (Ctrl+Z).</span>{/if}
  </fieldset>
  {#snippet buttons()}
    <button type="button" onclick={onClose}>Cancel</button>
    <button type="button" class="primary" onclick={run} disabled={action === "fill" && value === ""}>OK</button>
  {/snippet}
</FormShell>

<style>
  .choices {
    border: none;
    padding: 0;
    margin: 0 0 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  legend {
    font-weight: 600;
    margin-bottom: 4px;
  }
  .sub {
    margin-left: 24px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .off {
    opacity: 0.6;
  }
  .fill {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .value {
    font: inherit;
    padding: 2px 6px;
    width: 200px;
  }
  .muted {
    color: var(--muted);
    font-size: 12px;
  }
</style>
