<script lang="ts">
  // Run a recipe on many files now (Tools › Run recipe on files…): choose
  // the files (or a folder of them), a saved recipe and a results folder.
  // Each file is run and written under its own name to the results
  // folder; the files chosen are never changed.
  import { onDestroy } from "svelte";
  import { backend, errorText } from "./backend";
  import { batchSummary, type BatchStatus } from "./batch";
  import FormShell from "./FormShell.svelte";
  import { answerPairs, keepAnswers, lastAnswers } from "./recipe";

  interface Props {
    onClose: () => void;
  }
  let { onClose }: Props = $props();

  let st = $state<BatchStatus | null>(null);
  let force = $state(false);
  let problem = $state("");
  let answers = $state<Record<string, string>>({});

  function take(s: BatchStatus): void {
    const before = st?.recipe;
    st = s;
    if (s.recipe && s.recipe !== before) {
      const last = lastAnswers(s.recipe);
      answers = Object.fromEntries(s.questions.map((q) => [q.name, last[q.name] ?? q.default]));
    }
  }

  async function refresh(): Promise<void> {
    try {
      take(await backend().batchStatus());
    } catch (e) {
      problem = errorText(e);
    }
  }
  void refresh();
  const timer = window.setInterval(() => {
    if (st?.running) void refresh();
  }, 500);
  onDestroy(() => window.clearInterval(timer));

  async function pick(which: "files" | "folder" | "recipe" | "out"): Promise<void> {
    problem = "";
    try {
      take(await backend().batchPick(which));
    } catch (e) {
      problem = errorText(e);
    }
  }
  async function start(): Promise<void> {
    if (!st) return;
    problem = "";
    try {
      if (st.recipe) keepAnswers(st.recipe, answers);
      take(await backend().batchStart(answerPairs(st.questions, answers), force));
    } catch (e) {
      problem = errorText(e);
    }
  }
  async function stop(): Promise<void> {
    take(await backend().batchStop());
  }
  const ready = $derived(!!st && st.files.length > 0 && !!st.recipe && !!st.out && !st.running);
  const fileLine = $derived(
    !st || !st.files.length ? "not chosen" : st.files.length === 1 ? st.files[0]! : `${st.files.length.toLocaleString()} files (${st.files[0]!.split(/[\\/]/).pop()}, …)`,
  );
</script>

<FormShell title="Run a recipe on files" wide {onClose}>
  <p class="note">Each file is opened, run through a saved recipe (Tools ▸ Recipe) and written, under its own name, to the results folder. The files chosen are never changed.</p>
  <div class="setup">
    <span>Files</span><span class="path" title={st?.files.join("\n")}>{fileLine}</span>
    <span class="two">
      <button type="button" disabled={st?.running} onclick={() => void pick("files")}>Files…</button>
      <button type="button" disabled={st?.running} onclick={() => void pick("folder")}>Folder…</button>
    </span>
    <span>Recipe</span><span class="path">{st?.recipe ?? "not chosen"}</span>
    <button type="button" disabled={st?.running} onclick={() => void pick("recipe")}>Choose…</button>
    <span>Results folder</span><span class="path">{st?.out ?? "not chosen"}</span>
    <button type="button" disabled={st?.running} onclick={() => void pick("out")}>Choose…</button>
  </div>
  {#if st?.steps.length}
    <ol class="steps">{#each st.steps as d, i (i)}<li>{d}</li>{/each}</ol>
  {/if}
  {#if st?.questions.length}
    <fieldset class="questions">
      <legend>The recipe asks</legend>
      {#each st.questions as q (q.name)}
        <label class="row">{q.prompt || q.name} <input type="text" bind:value={answers[q.name]} disabled={st.running} /></label>
      {/each}
    </fieldset>
  {/if}
  <label class="row"><input type="checkbox" bind:checked={force} disabled={st?.running} /> Run even when a file's columns differ from the recipe's</label>
  {#if st && (st.running || st.results.length)}
    <p aria-live="polite">
      {#if st.running}<strong>Running {st.current ?? ""}</strong> ({st.results.length.toLocaleString()} of {st.files.length.toLocaleString()}). {/if}
      {batchSummary(st)}
    </p>
    <div class="results" role="table" aria-label="Results by file">
      {#each st.results as r, i (i)}
        <div role="row" class:bad={!r.ok}>
          <span role="cell">{r.ok ? "✓" : "✕"} {r.name}</span>
          <span role="cell" class="num">{r.ok ? `${r.rows_in.toLocaleString()} → ${r.rows_out.toLocaleString()} rows` : ""}</span>
          <span role="cell" class="msg">{r.message}</span>
        </div>
      {/each}
    </div>
  {/if}
  {#if problem}<p class="warn" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    {#if st?.running}
      <button type="button" class="primary" onclick={() => void stop()}>Stop after this file</button>
    {:else}
      <button type="button" class="primary" data-enter disabled={!ready} onclick={() => void start()}>Run</button>
    {/if}
    <button type="button" onclick={onClose}>Close</button>
  {/snippet}
</FormShell>

<style>
  .setup {
    display: grid;
    grid-template-columns: 9em 1fr auto;
    gap: 6px;
    align-items: center;
  }
  .two {
    display: flex;
    gap: 4px;
  }
  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    opacity: 0.85;
  }
  .steps {
    max-height: 120px;
    overflow: auto;
    margin: 6px 0;
    padding-left: 2em;
  }
  .questions {
    border: 1px solid var(--border, #8884);
    border-radius: 4px;
    padding: 4px 8px;
  }
  .results {
    max-height: 220px;
    overflow: auto;
    border: 1px solid var(--border, #8884);
    border-radius: 4px;
    padding: 4px 8px;
    font-size: 0.9em;
  }
  .results div {
    display: grid;
    grid-template-columns: minmax(8em, 1fr) 11em 2fr;
    gap: 8px;
  }
  .results .bad {
    color: var(--warn, #b45309);
  }
  .num {
    text-align: right;
  }
  .msg {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .warn {
    color: var(--warn, #b45309);
  }
</style>
