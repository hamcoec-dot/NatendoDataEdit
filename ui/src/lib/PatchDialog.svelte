<script lang="ts">
  // Patch files (.ndpatch). "save": the differences between the two compared
  // tables. "apply": choose a patch, see the dry run, apply it as one undo
  // step. A patch holds data, so saving one offers a phrase every time.
  import { untrack } from "svelte";
  import { backend, errorText } from "./backend";
  import FormShell from "./FormShell.svelte";
  import { MIN_PASSWORD } from "./encrypt";
  import { leftCounts, OUTCOME_WORDS, PATCH_SHOWN, savedText, willText, type PatchPreview, type PatchSpec } from "./patch";

  interface Props {
    /** Save: the older and newer tabs, the compare's keys and rules. */
    save?: { a: number; b: number; aName: string; bName: string; keys: string[]; spec: PatchSpec; sensitive: boolean } | null;
    /** Apply: the tab. */
    apply?: { doc: number; name: string } | null;
    /** Run a job with progress and Cancel (null: cancelled or busy). */
    run: <T>(label: string, f: () => Promise<T>) => Promise<T | null>;
    onCancelRun: () => void;
    onSaved: (message: string) => void;
    /** Apply the dry run; false when it was refused. */
    onApply: () => Promise<string>;
    onSaveReport: () => Promise<string>;
    onClose: () => void;
  }
  let { save = null, apply = null, run, onCancelRun, onSaved, onApply, onSaveReport, onClose }: Props = $props();

  // Save.
  let protect = $state(untrack(() => save?.sensitive ?? false));
  let phrase = $state("");
  let again = $state("");
  let show = $state(false);
  let leaveOut = $state(false);
  // Apply.
  let path = $state<string | null>(null);
  let needs = $state(false);
  let preview = $state<PatchPreview | null>(null);

  let problem = $state("");
  let note = $state("");
  let busy = $state(false);

  const check = $derived(
    !protect ? "" : phrase.length < MIN_PASSWORD ? `The phrase needs at least ${MIN_PASSWORD} characters.` : phrase !== again ? "The two phrases differ." : "",
  );

  async function work(f: () => Promise<void>): Promise<void> {
    if (busy) return;
    busy = true;
    problem = "";
    note = "";
    try {
      await f();
    } catch (e) {
      problem = errorText(e);
    } finally {
      busy = false;
    }
  }

  function savePatch(): void {
    const s = save;
    if (!s || check) return;
    void work(async () => {
      const to = await backend().patchPickSave(s.a);
      if (to === null) return;
      const done = await run("Making a patch", () =>
        backend().patchMake(s.a, s.b, { ...s.spec, leave_out: leaveOut }, to, protect ? phrase : null, !protect),
      );
      if (done) onSaved(savedText(done));
    });
  }

  function choose(): void {
    void work(async () => {
      const p = await backend().patchPick();
      if (p === null) return;
      path = p;
      preview = null;
      phrase = "";
      needs = await backend().patchProtected(p);
      if (!needs) await dryRun();
    });
  }

  async function dryRun(): Promise<void> {
    const t = apply;
    const p = path;
    if (!t || !p) return;
    preview = null;
    preview = await run("Reading the patch", () => backend().patchPreview(t.doc, p, needs ? phrase : null));
  }

  function applyIt(): void {
    void work(async () => {
      const said = await onApply();
      if (said) {
        problem = said;
        preview = null;
      }
    });
  }
</script>

<FormShell title={save ? "Save changes as a patch" : "Apply a patch"} wide={!!apply} onClose={() => { if (busy) onCancelRun(); onClose(); }}>
  {#if save}
    <p class="note">Writes what changed from <b>{save.aName}</b> (older) to <b>{save.bName}</b> (newer), finding rows by <b>{save.keys.join(" + ")}</b>. Every row counts, hidden ones too. Each key must be there once.</p>
    <p class="note"><b>A patch contains your data:</b> old and new values and whole added and removed rows.</p>
    <label class="row"><input type="checkbox" bind:checked={protect} /> Protect with a phrase</label>
    {#if protect}
      <label class="row">Phrase <input type={show ? "text" : "password"} bind:value={phrase} autocomplete="off" /></label>
      <label class="row">Type it again <input type={show ? "text" : "password"} bind:value={again} autocomplete="off" /></label>
      <label class="row"><input type="checkbox" bind:checked={show} /> Show the phrase</label>
      <p class="note">{check || "Without the phrase nobody can read the patch, and the app cannot get it back for you."}</p>
    {:else if save.sensitive}
      <p class="warn">One of these tables is private or came from an encrypted file. Unprotected, anyone who has the patch can read the values in it.</p>
    {/if}
    <label class="row"><input type="checkbox" bind:checked={leaveOut} /> Leave out columns only one table has (otherwise the patch is not made)</label>
  {:else if apply}
    <p class="note">Applies a patch to <b>{apply.name}</b>. A value is changed only if it still holds the patch's old value; anything else is left alone. Columns are found by title. Every row is checked, hidden ones too.</p>
    <div class="row"><button type="button" disabled={busy} onclick={choose}>Choose the patch file…</button> <span class="path">{path ?? "no file chosen"}</span></div>
    {#if needs && path}
      <label class="row">Phrase <input type="password" bind:value={phrase} autocomplete="off" />
        <button type="button" data-enter={preview ? undefined : true} disabled={busy || !phrase} onclick={() => void work(dryRun)}>Check (dry run)</button></label>
    {/if}
    {#if preview}
      {@const r = preview.report}
      <p><b>Dry run:</b> {willText(r)}</p>
      {#if leftCounts(r).length}
        <ul class="counts">
          {#each leftCounts(r) as [words, count] (words)}<li>{count.toLocaleString("en-US")} {words}</li>{/each}
        </ul>
      {/if}
      {#if r.missing.length}<p class="warn">Not in this table: {r.missing.join(", ")}. Entries that need these columns are not applied.</p>{/if}
      {#if r.extra.length}<p class="note">The patch does not look at these columns: {r.extra.join(", ")}. They are not compared, and added rows leave them blank.</p>{/if}
      {#if preview.private}<p class="note">This patch is protected or private, so applying it turns private file mode on for this tab.</p>{/if}
      {#if r.lines.length}
        <div class="lines">
          <table>
            <thead><tr><th>What happens</th><th>Kind</th>{#each r.keys as k, i (i)}<th>{k}</th>{/each}<th>Column</th></tr></thead>
            <tbody>
              {#each r.lines.slice(0, PATCH_SHOWN) as l, i (i)}
                <tr><td>{OUTCOME_WORDS[l.outcome]}</td><td>{l.kind}</td>{#each l.key as v, j (j)}<td>{v}</td>{/each}<td>{l.column}</td></tr>
              {/each}
            </tbody>
          </table>
        </div>
        {#if r.lines_total > PATCH_SHOWN}<p class="note">The first {PATCH_SHOWN} of {r.lines_total.toLocaleString("en-US")} lines are shown; the saved report holds up to 10,000.</p>{/if}
      {/if}
    {/if}
  {/if}
  {#if note}<p class="note" role="status">{note}</p>{/if}
  {#if problem}<p class="warn" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    {#if save}
      <button type="button" class="primary" data-enter disabled={!!check || busy} onclick={savePatch}>Save patch…</button>
    {:else}
      <button type="button" class="primary" disabled={busy || !preview?.report.will_apply} onclick={applyIt}>Apply</button>
      <button type="button" disabled={busy || !preview} onclick={() => void work(async () => { note = await onSaveReport(); })}>Save report as CSV…</button>
    {/if}
    {#if busy}<button type="button" onclick={onCancelRun}>Cancel</button>{/if}
    <button type="button" onclick={() => { if (busy) onCancelRun(); onClose(); }}>Close</button>
  {/snippet}
</FormShell>

<style>
  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .warn {
    color: var(--warn, #b45309);
  }
  .counts {
    margin: 0 0 8px;
  }
  .lines {
    max-height: 220px;
    overflow: auto;
  }
  table {
    border-collapse: collapse;
    width: 100%;
  }
  th,
  td {
    border-bottom: 1px solid var(--border, #ddd);
    padding: 2px 8px;
    text-align: left;
    white-space: nowrap;
  }
</style>
