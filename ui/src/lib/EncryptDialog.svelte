<script lang="ts">
  // Password-protected files (Milestone 15): save an encrypted copy of the
  // shown rows, or open an encrypted file (decrypted in memory only; the
  // tab is private).
  import { backend, errorText, type DocInfo } from "./backend";
  import FormShell from "./FormShell.svelte";
  import { MIN_PASSWORD, passwordProblem } from "./encrypt";

  interface Props {
    mode: "save" | "open";
    doc: number | null;
    filtered: boolean;
    /** Save over this encrypted file (the tab came from it). */
    same?: string | null;
    onSaved: (rows: number) => void;
    onOpened: (info: DocInfo) => void;
    onClose: () => void;
  }
  let { mode, doc, filtered, same = null, onSaved, onOpened, onClose }: Props = $props();

  let password = $state("");
  let again = $state("");
  let show = $state(false);
  let path = $state<string | null>(null);
  let problem = $state("");
  let running = $state(false);
  const check = $derived(mode === "save" ? passwordProblem(password, again) : password ? "" : "Type the password.");

  async function pick(): Promise<void> {
    problem = "";
    try {
      path = await backend().pickEncrypted();
    } catch (e) {
      problem = errorText(e);
    }
  }
  async function go(): Promise<void> {
    if (check || running) return;
    running = true;
    problem = "";
    try {
      if (mode === "save" && doc !== null) {
        const n = await backend().saveEncrypted(doc, password, !!same);
        if (n !== null) onSaved(n);
      } else if (path) {
        onOpened(await backend().openEncrypted(password));
      }
    } catch (e) {
      problem = errorText(e);
    } finally {
      running = false;
    }
  }
</script>

<FormShell title={same ? "Save encrypted" : mode === "save" ? "Save an encrypted copy" : "Open an encrypted file"} {onClose}>
  {#if same}
    <p class="note">Saves {filtered ? "the rows shown" : "the table"} encrypted over <b>{same}</b>. Type the password for it (the same one, or a new one). Without the password nobody can read it, and the app cannot get it back for you.</p>
  {:else if mode === "save"}
    <p class="note">Writes {filtered ? "the rows shown" : "the table"} as a password-protected file (.ndenc). Without the password nobody can read it, and the app cannot get it back for you, so keep the password somewhere safe.</p>
  {:else}
    <p class="note">Opens a password-protected file (.ndenc). It is read into memory only and the tab is private: no recovery copies and never in the recent files. Save it again with File ▸ Save an encrypted copy.</p>
    <label class="row"><button type="button" onclick={() => void pick()}>Choose the file…</button> <span class="path">{path ?? "no file chosen"}</span></label>
  {/if}
  <label class="row">Password <input type={show ? "text" : "password"} bind:value={password} autocomplete="off" /></label>
  {#if mode === "save"}
    <label class="row">Type it again <input type={show ? "text" : "password"} bind:value={again} autocomplete="off" /></label>
  {/if}
  <label class="row"><input type="checkbox" bind:checked={show} /> Show the password</label>
  {#if mode === "save" && check && password}<p class="note">{check}</p>{/if}
  {#if mode === "save" && !password}<p class="note">At least {MIN_PASSWORD} characters; a few unrelated words is easiest to remember.</p>{/if}
  {#if problem}<p class="warn">{problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" class="primary" data-enter disabled={!!check || running || (mode === "open" && !path)} onclick={() => void go()}>{mode === "save" ? (running ? "Encrypting…" : "Save…") : running ? "Opening…" : "Open"}</button>
    <button type="button" onclick={onClose}>Close</button>
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
</style>
