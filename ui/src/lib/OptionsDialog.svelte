<script lang="ts">
  // Options: which file types open in this app (Windows "Open with" and
  // Default apps), the agent link for other programs, and the log folder.
  import { onMount } from "svelte";
  import { backend, errorText, type AgentStatus, type FileType } from "./backend";
  import FormShell from "./FormShell.svelte";
  import { prefs } from "./prefs.svelte";

  interface Props {
    /** The agent link setting changed (to keep it for next time). */
    onAgent: (on: boolean) => void;
    onClose: () => void;
  }
  let { onAgent, onClose }: Props = $props();

  let clearing = $state(false);
  let cleared = $state("");
  function keepVersionsChanged(): void {
    prefs.save();
    void backend().versionsKeep(prefs.data.keepEarlierSaves).catch(() => {});
  }
  async function clearVersions(): Promise<void> {
    clearing = true;
    try {
      const n = await backend().versionsClear();
      cleared = n ? `Deleted ${n.toLocaleString()} earlier save${n === 1 ? "" : "s"}.` : "There were no earlier saves.";
    } catch (e) {
      cleared = errorText(e);
    } finally {
      clearing = false;
    }
  }

  let types = $state<FileType[]>([]);
  let chosen = $state<Record<string, boolean>>({});
  let agent = $state<AgentStatus>({ on: false, port: null });
  let problem = $state("");
  let note = $state("");
  let busy = $state(false);

  onMount(() => {
    void (async () => {
      try {
        types = await backend().fileTypes();
        chosen = Object.fromEntries(types.map((t) => [t.ext, t.on]));
        agent = await backend().agentStatus();
      } catch (e) {
        problem = errorText(e);
      }
    })();
  });

  const changed = $derived(types.some((t) => chosen[t.ext] !== t.on));

  async function applyTypes(): Promise<void> {
    busy = true;
    try {
      types = await backend().setFileTypes(types.filter((t) => chosen[t.ext]).map((t) => t.ext));
      chosen = Object.fromEntries(types.map((t) => [t.ext, t.on]));
      note = "Saved. Natendo Data Edit now shows in “Open with” for the chosen types. To make it the default, choose it in Windows Settings.";
      problem = "";
    } catch (e) {
      problem = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function toggleAgent(on: boolean): Promise<void> {
    try {
      agent = await backend().setAgentLink(on);
      onAgent(agent.on);
      problem = "";
    } catch (e) {
      problem = errorText(e);
    }
  }

  async function act(f: () => Promise<void>): Promise<void> {
    try {
      await f();
      problem = "";
    } catch (e) {
      problem = errorText(e);
    }
  }
</script>

<FormShell title="Options" wide {onClose}>
  <fieldset>
    <legend>File types</legend>
    {#if types.length === 0}
      <p class="note">File types are set by the system on this computer.</p>
    {:else}
      <p class="note">Open these kinds of files with Natendo Data Edit (double-click, “Open with”, dragging onto the window). A file opens in a new tab of the window that is already open.</p>
      <div class="types">
        {#each types.filter((t) => !t.text) as t (t.ext)}
          <label class="row"><input type="checkbox" bind:checked={chosen[t.ext]} /> .{t.ext}</label>
        {/each}
      </div>
      <p class="sub">Plain text files (usually opened in a text editor):</p>
      <div class="types">
        {#each types.filter((t) => t.text) as t (t.ext)}
          <label class="row"><input type="checkbox" bind:checked={chosen[t.ext]} /> .{t.ext}</label>
        {/each}
      </div>
      <div class="actions">
        <button type="button" class="primary" disabled={!changed || busy} onclick={() => void applyTypes()}>Apply</button>
        <button type="button" onclick={() => void act(() => backend().openDefaultApps())}>Choose default apps in Windows Settings…</button>
      </div>
      <p class="note">Windows lets only you change which app is the default for a file type, so Natendo Data Edit never takes over a type another app owns; the Settings button opens the page where you can choose it.</p>
    {/if}
    {#if note}<p class="good">{note}</p>{/if}
  </fieldset>
  <fieldset>
    <legend>Agent link</legend>
    <label class="row"><input type="checkbox" checked={agent.on} onchange={(e) => void toggleAgent(e.currentTarget.checked)} /> Let programs on this computer read the open tables</label>
    <p class="note">
      For scripts and AI agents (for example <code>natendo-cli app tabs</code>). They can list tabs, read cells, open a file in a tab and move the cursor, never change data.
      Only programs that can read your own app-data folder get the secret key, and only this computer can connect. It stays on (also after a restart) until you switch it off.
      {#if agent.on && agent.port}Listening on this computer only (port {agent.port}).{/if}
    </p>
  </fieldset>
  <fieldset>
    <legend>Editing</legend>
    <label class="row"
      >After pressing Enter, move
      <select aria-label="After pressing Enter, move" bind:value={prefs.data.enterMove} onchange={() => prefs.save()}>
        <option value="down">down</option>
        <option value="right">right</option>
        <option value="up">up</option>
        <option value="left">left</option>
        <option value="none">nowhere (stay on the cell)</option>
      </select></label
    >
    <p class="note">Shift+Enter moves the other way. Inside a selected block, Enter moves within the block (unless set to stay).</p>
  </fieldset>
  <fieldset>
    <legend>Autosave and recovery</legend>
    <label class="row"><input type="checkbox" bind:checked={prefs.data.autosave} onchange={() => prefs.save()} /> Keep a recovery copy of unsaved changes every
      <input type="number" min="1" max="60" step="1" style="width: 4em" bind:value={prefs.data.autosaveMinutes} onchange={() => { prefs.data.autosaveMinutes = Math.min(60, Math.max(1, Math.round(Number(prefs.data.autosaveMinutes) || 2))); prefs.save(); }} /> minutes</label>
    <p class="note">
      Copies go to a private folder of yours, never over your files, and are removed when you save or close. If the app stops before you save, they are offered back the next time it starts (File › Recover unsaved work). Files over 200 MB get no copy.
    </p>
    <label class="row"
      >At start, the files open last time:
      <select bind:value={prefs.data.reopenLast} onchange={() => { prefs.save(); if (prefs.data.reopenLast === "never") void backend().sessionClear().catch(() => {}); }}>
        <option value="ask">Offer to reopen them</option>
        <option value="always">Reopen them</option>
        <option value="never">Don't remember them</option>
      </select></label
    >
    <p class="note">Only each file's place on disk, its view (widths, hidden columns, filter, highlights) and the active cell are kept, never what is in the cells. Files in private file mode, encrypted files and tabs never saved are not remembered. A file changed since opens fresh.</p>
    <label class="row"><input type="checkbox" bind:checked={prefs.data.keepEarlierSaves} onchange={keepVersionsChanged} /> Keep earlier saves of each file</label>
    <p class="note">
      Before each save, the file as it was is kept in the same private folder (files up to 64 MB, the last 20 saves, for 30 days; File › Earlier saves of this file). Off unless you switch it on, as these are extra copies of your data; delete them below at any time. Uninstalling the app removes them too.
    </p>
    <div class="row">
      <button type="button" disabled={clearing} onclick={clearVersions}>Delete earlier saves now</button>
      {#if cleared}<span class="note">{cleared}</span>{/if}
    </div>
  </fieldset>
  <fieldset>
    <legend>Private data</legend>
    <label class="row"><input type="checkbox" bind:checked={prefs.data.privacyWarnings} onchange={() => prefs.save()} /> Warn before exporting, printing, saving a copy or copying a lot from a file that looks private</label>
    <p class="note">
      A file looks private when a column holds things like Social Security numbers, birth dates, phone numbers, e-mail or street addresses. Tools › Private data offers a share-safe copy instead. For files that should leave no trace on this computer, use File › Private file mode: no recovery copies, no earlier saves, not in the recent files, and the clipboard is cleared when the tab closes.
    </p>
  </fieldset>
  <fieldset>
    <legend>Log files</legend>
    <p class="note">The app keeps short log files (what was opened, saved or went wrong; never your data) to help find problems. Five files of 1 MB at most.</p>
    <button type="button" onclick={() => void act(() => backend().openLogFolder())}>Open the log folder</button>
  </fieldset>
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" onclick={onClose}>Close</button>
  {/snippet}
</FormShell>

<style>
  .types {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 16px;
  }
  .actions {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    margin: 6px 0;
  }
</style>
