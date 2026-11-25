<script lang="ts">
  // The steps panel (View › Recipe steps, like Power Query's applied
  // steps): the changes made to this tab that a recipe can repeat. Click a
  // step to see the table as it was after it; turn a step off, move it,
  // take it out or change its settings, and the steps after it run again.
  // A text in a step can become a question the recipe asks each time.
  import { onDestroy } from "svelte";
  import { backend, errorText, type DocInfo } from "./backend";
  import { questionNameOk, type RecipeStep, type StepChange, type StepsView } from "./recipe";
  import type { Tab } from "./tab.svelte";
  import CheckpointsPanel from "./CheckpointsPanel.svelte";

  interface Props {
    tab: Tab;
    /** Take the new table state after a peek or a change. */
    onInfo: (info: DocInfo) => void;
    /** Run a step change as a job (progress, Cancel); null when busy. */
    run: (label: string, f: () => Promise<DocInfo>) => Promise<DocInfo | null>;
    /** Ask before undoing what cannot be done again; true to go ahead. */
    confirmLoss: (what: string) => Promise<boolean>;
    onSave: () => void;
    now: () => number;
    /** Go to a checkpoint (the screen does what Ctrl+Z does for every step crossed). */
    restoreCheckpoint: (c: import("./backend").Checkpoint) => Promise<void>;
    /** Column titles now, going to a cell, and stopping a running comparison (the checkpoint comparison). */
    titles: string[];
    goToCell: (row: number, col: number) => void;
    stopJob: () => void;
  }
  let { tab, onInfo, run, confirmLoss, onSave, now, restoreCheckpoint, titles, goToCell, stopJob }: Props = $props();

  let view = $state<StepsView | null>(null);
  let problem = $state("");
  /** The step whose settings or question are open. */
  let editing = $state<{ k: number; what: "settings" | "ask" } | null>(null);
  let settingsText = $state("");
  let askPath = $state("");
  let askName = $state("");
  let askPrompt = $state("");

  async function refresh(): Promise<void> {
    try {
      view = await backend().stepsView(tab.id);
      tab.peeking = (!!view && view.at < view.steps.length) || !!tab.info.peeking;
      problem = "";
    } catch (e) {
      problem = errorText(e);
    }
  }
  $effect(() => {
    void tab.version;
    void refresh();
  });

  // Leaving the panel while looking at an earlier step goes back to the
  // latest (the table can be edited again).
  onDestroy(() => {
    const v = view;
    if (tab.peeking && v && v.steps.length) {
      tab.peeking = false;
      void backend()
        .stepsGoto(tab.id, v.steps.length)
        .then(onInfo)
        .catch(() => {});
    }
  });

  /** The right-click menu on a step. */
  let menu = $state<{ k: number; x: number; y: number } | null>(null);
  function openMenu(e: MouseEvent, k: number): void {
    const s = view?.steps[k];
    if (!s || s.kind === "left_out") return;
    e.preventDefault();
    e.stopPropagation();
    menu = { k, x: Math.min(e.clientX, window.innerWidth - 220), y: Math.min(e.clientY, window.innerHeight - 220) };
  }
  function fromMenu(f: () => void): void {
    menu = null;
    f();
  }

  async function goto(k: number | null): Promise<void> {
    try {
      onInfo(await backend().stepsGoto(tab.id, k));
    } catch (e) {
      problem = errorText(e);
    }
    await refresh();
  }

  async function change(k: number, c: StepChange, force = false): Promise<void> {
    problem = "";
    try {
      const info = await run("Running the steps again", () => backend().stepsChange(tab.id, k, c, now(), force));
      if (info) onInfo(info);
      editing = null;
    } catch (e) {
      const kind = (e as { kind?: string }).kind;
      if (kind === "steps_lost" && !force) {
        if (await confirmLoss(errorText(e))) await change(k, c, true);
      } else problem = errorText(e);
    }
    await refresh();
  }

  function openSettings(k: number, op: RecipeStep): void {
    editing = { k, what: "settings" };
    settingsText = JSON.stringify(op, null, 2);
  }
  async function applySettings(): Promise<void> {
    if (!editing) return;
    let op: RecipeStep;
    try {
      op = JSON.parse(settingsText) as RecipeStep;
    } catch {
      problem = "The settings are not valid JSON.";
      return;
    }
    await change(editing.k, { kind: "replace", op });
  }

  function openAsk(k: number): void {
    const row = view?.steps[k];
    editing = { k, what: "ask" };
    askPath = row?.texts[0]?.[0] ?? "";
    askName = "";
    askPrompt = "";
  }
  async function ask(): Promise<void> {
    if (!editing) return;
    try {
      view = await backend().stepsAsk(tab.id, editing.k, askPath, askName.trim(), askPrompt.trim());
      editing = null;
      problem = "";
    } catch (e) {
      problem = errorText(e);
    }
  }

  const count = $derived(view?.steps.length ?? 0);
  const steps = $derived(view?.steps.filter((s) => s.kind !== "left_out").length ?? 0);
</script>

<svelte:window onpointerdown={(e) => { if (menu && !(e.target as HTMLElement | null)?.closest?.(".step-menu")) menu = null; }} onkeydown={(e) => { if (menu && e.key === "Escape") menu = null; }} />

<div class="steps-panel">
  <CheckpointsPanel {tab} onRestore={restoreCheckpoint} {titles} onGo={goToCell} onStop={stopJob} />
  {#if menu && view?.steps[menu.k]}
    {@const k = menu.k}
    {@const s = view.steps[k]!}
    <div class="step-menu" role="menu" aria-label={`Step ${k + 1}`} style:left="{menu.x}px" style:top="{menu.y}px">
      <button type="button" role="menuitem" onclick={() => fromMenu(() => void goto(k))}>Show the table after this step</button>
      <button type="button" role="menuitem" onclick={() => fromMenu(() => void change(k, { kind: "toggle" }))}>{s.kind === "off" ? "Turn on" : "Turn off"}</button>
      <button type="button" role="menuitem" disabled={k === 0} onclick={() => fromMenu(() => void change(k, { kind: "move_up" }))}>Move up</button>
      <button type="button" role="menuitem" disabled={k + 1 >= count} onclick={() => fromMenu(() => void change(k, { kind: "move_down" }))}>Move down</button>
      <button type="button" role="menuitem" onclick={() => fromMenu(() => s.op && openSettings(k, s.op))}>Change its settings…</button>
      {#if s.texts.length}<button type="button" role="menuitem" onclick={() => fromMenu(() => openAsk(k))}>Ask for a value each run…</button>{/if}
      <button type="button" role="menuitem" onclick={() => fromMenu(() => void change(k, { kind: "remove" }))}>Take this step out</button>
    </div>
  {/if}
  {#if !view}
    <p class="note">Reading the steps…</p>
  {:else if count === 0}
    <p class="note">No steps yet. Sorts, filters, column changes, replacing, cleaning, computed columns and the other Data and Tools changes you make are listed here as you go. Click one to see the table as it was then; change or turn off a step and the ones after it run again.</p>
  {:else}
    {#if view.at < count || tab.info.peeking}
      <div class="peek" role="status">
        Looking at the table after {view.at === 0 ? "none of the steps" : `step ${view.at}`}: changes and saving are off until you go back to the latest step.
        <button type="button" onclick={() => void goto(count)}>Back to the latest step</button>
      </div>
    {/if}
    <ol class="list" aria-label="Recipe steps">
      <li class="start" class:here={view.at === 0}>
        <button type="button" class="step" onclick={() => void goto(null)}>Before the first step</button>
      </li>
      {#each view.steps as s, k (k)}
        <li class:here={view.at === k + 1} class:later={!s.done} class:off={s.kind === "off"} class:out={s.kind === "left_out"} oncontextmenu={(e) => openMenu(e, k)}>
          <button type="button" class="step" title={s.kind === "left_out" ? "Done by hand: a recipe cannot repeat this" : "Show the table after this step"} disabled={s.kind === "left_out"} onclick={() => void goto(k)}>
            <span class="n">{k + 1}</span> {s.text}{#if s.kind === "off"} <em>(off)</em>{/if}{#if s.kind === "left_out"} <em>(not in the recipe)</em>{/if}
          </button>
          {#if s.kind !== "left_out"}
            <span class="acts">
              <button type="button" aria-label={`Move step ${k + 1} up`} title="Move up" disabled={k === 0} onclick={() => void change(k, { kind: "move_up" })}>↑</button>
              <button type="button" aria-label={`Move step ${k + 1} down`} title="Move down" disabled={k + 1 >= count} onclick={() => void change(k, { kind: "move_down" })}>↓</button>
              <button type="button" aria-label={`${s.kind === "off" ? "Turn on" : "Turn off"} step ${k + 1}`} title={s.kind === "off" ? "Turn on" : "Turn off"} onclick={() => void change(k, { kind: "toggle" })}>{s.kind === "off" ? "On" : "Off"}</button>
              <button type="button" aria-label={`Settings of step ${k + 1}`} title="Change its settings" onclick={() => s.op && openSettings(k, s.op)}>⚙</button>
              {#if s.texts.length}
                <button type="button" aria-label={`Ask for a value in step ${k + 1}`} title="Ask for one of its values each time the recipe runs" onclick={() => openAsk(k)}>?</button>
              {/if}
              <button type="button" aria-label={`Remove step ${k + 1}`} title="Take this step out" onclick={() => void change(k, { kind: "remove" })}>✕</button>
            </span>
          {/if}
          {#if editing?.k === k && editing.what === "settings"}
            <div class="edit">
              <label for="step-settings">Settings (the steps after it run again)</label>
              <textarea id="step-settings" rows="8" spellcheck="false" bind:value={settingsText}></textarea>
              <div class="row">
                <button type="button" class="primary" onclick={() => void applySettings()}>Apply</button>
                <button type="button" onclick={() => (editing = null)}>Cancel</button>
              </div>
            </div>
          {/if}
          {#if editing?.k === k && editing.what === "ask"}
            <div class="edit">
              <label>Value to ask for
                <select bind:value={askPath}>
                  {#each s.texts as [p, v] (p)}<option value={p}>{v || "(blank)"} ({p})</option>{/each}
                </select>
              </label>
              <label>Name (for natendo-cli run --set) <input type="text" bind:value={askName} maxlength="40" placeholder="county" /></label>
              <label>Question <input type="text" bind:value={askPrompt} maxlength="200" placeholder="Which county?" /></label>
              <div class="row">
                <button type="button" class="primary" disabled={!questionNameOk(askName)} onclick={() => void ask()}>Ask each time</button>
                <button type="button" onclick={() => (editing = null)}>Cancel</button>
              </div>
            </div>
          {/if}
        </li>
      {/each}
    </ol>
    {#if view.questions.length}
      <p class="note">Asked each time the recipe runs: {view.questions.map((q) => `${q.prompt || q.name} (last: ${q.default || "blank"})`).join("; ")}.</p>
    {/if}
    {#if view.hand_changes}
      <p class="note">Changes made by hand after the last step are not steps; changing a step asks before undoing them.</p>
    {/if}
    <div class="row">
      <button type="button" disabled={steps === 0} onclick={onSave}>Save as a recipe…</button>
    </div>
  {/if}
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
</div>

<style>
  .list {
    list-style: none;
    margin: 0 0 8px;
    padding: 0;
  }
  .list li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px;
    border-left: 3px solid transparent;
    padding: 1px 0 1px 4px;
  }
  .list li.here {
    border-left-color: var(--accent, #1f7a6d);
    background: color-mix(in srgb, var(--accent, #1f7a6d) 10%, transparent);
  }
  .list li.later,
  .list li.off {
    opacity: 0.6;
  }
  .list li.out em,
  .list li.off em {
    font-size: 0.9em;
  }
  .step {
    flex: 1;
    text-align: left;
    background: none;
    border: none;
    padding: 3px 2px;
    cursor: pointer;
    color: inherit;
    font: inherit;
  }
  .step:disabled {
    cursor: default;
  }
  .n {
    display: inline-block;
    min-width: 1.6em;
    opacity: 0.6;
  }
  .acts {
    display: flex;
    gap: 1px;
  }
  .acts button {
    min-width: 1.8em;
    padding: 1px 4px;
    font-size: 0.85em;
  }
  .edit {
    flex-basis: 100%;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 4px 0 6px 1.6em;
  }
  .edit textarea {
    font-family: var(--mono, monospace);
    font-size: 0.85em;
  }
  .row {
    display: flex;
    gap: 6px;
  }
  .step-menu {
    position: fixed;
    z-index: 50;
    display: flex;
    flex-direction: column;
    min-width: 200px;
    padding: 4px 0;
    background: var(--bg, #fff);
    border: 1px solid var(--border, #bbb);
    box-shadow: 0 4px 14px rgb(0 0 0 / 0.18);
  }
  .step-menu button {
    text-align: left;
    border: 0;
    background: none;
    padding: 4px 12px;
  }
  .step-menu button:hover:not(:disabled) {
    background: var(--hover, rgb(0 0 0 / 0.08));
  }
  .peek {
    border: 1px solid var(--accent, #1f7a6d);
    border-radius: 4px;
    padding: 6px 8px;
    margin-bottom: 6px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .problem {
    color: var(--warn, #b45309);
  }
</style>
