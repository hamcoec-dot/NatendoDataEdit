<script lang="ts">
  // Recipes (Milestone 14): the steps done on this tab so far, saved as a
  // recipe file; or a recipe file opened and run on this tab, after its
  // steps are shown and its columns checked.
  import FormShell from "./FormShell.svelte";
  import { lastAnswers, needsPhrase, type RecipeOpened, type RecipeView } from "./recipe";
  import { MIN_PHRASE } from "./clean";

  interface Props {
    /** This tab's steps (null while they load). */
    current: RecipeView | null;
    /** A recipe file opened to run. */
    opened: RecipeOpened | null;
    onSave: (name: string) => void;
    onOpen: () => void;
    onRun: (force: boolean, phrase: string, answers: Record<string, string>) => void;
    onClose: () => void;
  }
  let { current, opened, onSave, onOpen, onRun, onClose }: Props = $props();

  let name = $state("");
  let force = $state(false);
  let phrase = $state("");
  /** Answers to the recipe's questions (the last ones given at first). */
  let answers = $state<Record<string, string>>({});
  let answersFor = "";
  $effect(() => {
    const r = opened;
    if (!r || answersFor === r.path) return;
    answersFor = r.path;
    const last = lastAnswers(r.path);
    answers = Object.fromEntries((r.recipe.questions ?? []).map((q) => [q.name, last[q.name] ?? q.default]));
  });
  const phraseNeeded = $derived(!!opened && needsPhrase(opened.recipe));
  const phraseOk = $derived(!phraseNeeded || [...phrase].length >= MIN_PHRASE);
  const canRun = $derived(!!opened && opened.recipe.steps.length > 0 && (opened.problems.length === 0 || force) && phraseOk);
</script>

<FormShell title={opened ? "Run a recipe" : "Recipe of this tab"} wide {onClose}>
  {#if opened}
    <p class="note">From {opened.path}{opened.recipe.name ? ` ("${opened.recipe.name}")` : ""}. The steps run in this order; each one can be undone with Ctrl+Z.</p>
    <ol class="steps">
      {#each opened.described as d, i (i)}<li>{d}</li>{/each}
    </ol>
    {#if opened.recipe.left_out.length}
      <p class="note">Not in the recipe (do these by hand): {opened.recipe.left_out.join("; ")}.</p>
    {/if}
    {#if opened.recipe.questions?.length}
      <fieldset class="questions">
        <legend>The recipe asks</legend>
        {#each opened.recipe.questions as q (q.name)}
          <label class="row">{q.prompt || q.name} <input type="text" bind:value={answers[q.name]} /></label>
        {/each}
      </fieldset>
    {/if}
    {#if opened.problems.length}
      <p class="warn">This tab's columns differ from the file the recipe was made on: {opened.problems.join("; ")}. Steps name columns by place, so they may change the wrong ones.</p>
      <label class="row"><input type="checkbox" bind:checked={force} /> Run it anyway</label>
    {/if}
    {#if phraseNeeded}
      <label class="row">Secret phrase for the scramble step (never saved in the recipe):
        <input type="password" bind:value={phrase} autocomplete="off" />
      </label>
      {#if !phraseOk}<p class="note">At least {MIN_PHRASE} characters, the same phrase as before so values scramble the same way.</p>{/if}
    {/if}
  {:else if current === null}
    <p class="note">Reading the steps…</p>
  {:else if current.recipe.steps.length === 0 && current.recipe.left_out.length === 0}
    <p>No steps yet. Sorts, filters, column changes, replacing, cleaning, computed columns and the other Data and Tools changes you make on this tab are noted here, so you can save them and run them on another file.</p>
  {:else}
    <ol class="steps">
      {#each current.described as d, i (i)}<li>{d}{#if current.holds_values[i]} <span class="vals" title="This step holds values from the file, and the recipe file will hold them too">(holds values from the file)</span>{/if}</li>{/each}
    </ol>
    {#if current.holds_values.some(Boolean) || current.recipe.columns.length}
      <p class="note">The recipe file holds the column titles{current.holds_values.some(Boolean) ? " and the values in the marked steps" : ""}. If this file is private, treat the recipe as private too.</p>
    {/if}
    {#if current.recipe.left_out.length}
      <p class="note">Left out (a recipe cannot repeat these): {current.recipe.left_out.join("; ")}.</p>
    {/if}
    <label class="row">Name <input type="text" bind:value={name} maxlength="200" placeholder="Monthly clean-up" /></label>
    <p class="note">Undone steps are left out. A secret phrase is never saved; you type it again when you run the recipe.</p>
  {/if}
  {#snippet buttons()}
    {#if opened}
      <button type="button" class="primary" data-enter disabled={!canRun} onclick={() => onRun(force, phrase, $state.snapshot(answers))}>Run on this tab</button>
    {:else}
      <button type="button" class="primary" data-enter disabled={!current || current.recipe.steps.length === 0} onclick={() => onSave(name)}>Save recipe…</button>
      <button type="button" onclick={onOpen}>Open a recipe to run…</button>
    {/if}
    <button type="button" onclick={onClose}>Close</button>
  {/snippet}
</FormShell>

<style>
  .steps {
    max-height: 280px;
    overflow: auto;
    margin: 4px 0 8px;
    padding-left: 2em;
  }
  .vals {
    opacity: 0.75;
    font-size: 0.9em;
  }
  .warn {
    color: var(--warn, #b45309);
  }
  .questions {
    border: 1px solid var(--border, #8884);
    border-radius: 4px;
    padding: 4px 8px;
    margin: 6px 0;
  }
</style>
