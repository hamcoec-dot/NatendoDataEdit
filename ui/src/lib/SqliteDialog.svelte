<script lang="ts">
  // SQLite databases (Milestone 15): open a table of a database file in a
  // new tab, or save the shown rows as a table in a database file.
  import { backend, errorText, type DocInfo } from "./backend";
  import FormShell from "./FormShell.svelte";
  import { tableLabel, tableName, type DbTable } from "./sqlite";

  interface Props {
    mode: "save" | "open";
    doc: number | null;
    docName: string;
    header: boolean;
    filtered: boolean;
    onSaved: (rows: number) => void;
    onOpened: (info: DocInfo) => void;
    onClose: () => void;
  }
  let { mode, doc, docName, header, filtered, onSaved, onOpened, onClose }: Props = $props();

  let path = $state<string | null>(null);
  let tables = $state<DbTable[]>([]);
  let picked = $state("");
  // svelte-ignore state_referenced_locally
  let table = $state(tableName(docName));
  let replace = $state(false);
  let problem = $state("");
  let running = $state(false);

  async function pick(): Promise<void> {
    problem = "";
    try {
      const r = await backend().sqlitePick();
      if (!r) return;
      [path, tables] = r;
      picked = tables[0]?.name ?? "";
      if (!tables.length) problem = "That database has no tables.";
    } catch (e) {
      problem = errorText(e);
    }
  }
  async function go(): Promise<void> {
    if (running) return;
    running = true;
    problem = "";
    try {
      if (mode === "save" && doc !== null) {
        const n = await backend().sqliteSave(doc, table.trim(), header, replace);
        if (n !== null) onSaved(n);
      } else if (picked) {
        onOpened(await backend().sqliteOpen(picked));
      }
    } catch (e) {
      problem = errorText(e);
    } finally {
      running = false;
    }
  }
  const ok = $derived(mode === "save" ? !!table.trim() && !/^sqlite_/i.test(table.trim()) : !!picked);
</script>

<FormShell title={mode === "save" ? "Save as a table in a database" : "Open a table from a database"} {onClose}>
  {#if mode === "save"}
    <p class="note">Writes {filtered ? "the rows shown" : "the table"} into a SQLite database file (.sqlite or .db). A new file is created; an existing one keeps its other tables. Every column is stored as text and empty cells as NULL.</p>
    <label class="row">Table name <input type="text" bind:value={table} maxlength="128" /></label>
    <label class="row"><input type="checkbox" bind:checked={replace} /> Replace a table of the same name if the database has one</label>
  {:else}
    <p class="note">Opens one table (or view) of a SQLite database file in a new tab. The database itself is only read, never changed. Reading stops if the database stops answering for a minute (a view is read for at most five minutes).</p>
    <label class="row"><button type="button" onclick={() => void pick()}>Choose the database…</button> <span class="path">{path ?? "no file chosen"}</span></label>
    {#if tables.length}
      <label class="row">Table
        <select bind:value={picked}>
          {#each tables as t (t.name)}
            <option value={t.name}>{tableLabel(t)}</option>
          {/each}
        </select>
      </label>
    {/if}
  {/if}
  {#if problem}<p class="warn">{problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" class="primary" data-enter disabled={!ok || running} onclick={() => void go()}>{mode === "save" ? (running ? "Saving…" : "Save…") : running ? "Opening…" : "Open"}</button>
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
