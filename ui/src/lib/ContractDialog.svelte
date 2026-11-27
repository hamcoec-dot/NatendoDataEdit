<script lang="ts">
  // C9: make a file contract from this tab (or change one). Nothing is written until Save. Allowed values are
  // real cell values, so no column is ticked to begin with, and one that looks like personal data asks first.
  import { untrack } from "svelte";
  import type { Contract, ContractDraft } from "./backend";
  import { nameProblem } from "./contractMock";
  import FormShell from "./FormShell.svelte";

  interface Props {
    draft: ContractDraft;
    /** The tab's data checks (rules and table checks): how many. */
    checks: number;
    /** The name it had (changing a saved contract), else null. */
    was: string | null;
    confirmPersonal: (title: string) => Promise<boolean>;
    onSave: (c: Contract) => void;
    onClose: () => void;
  }
  let { draft, checks, was, confirmPersonal, onSave, onClose }: Props = $props();
  const start = untrack(() => draft);
  let name = $state(start.contract.name);
  let pattern = $state(start.contract.recognise.pattern ?? "");
  let byTitles = $state(start.contract.recognise.titles);
  let withChecks = $state(untrack(() => checks > 0) || !!start.contract.checks);
  let rowsOn = $state(!!start.contract.rows);
  let min = $state(start.contract.rows?.min ?? Math.floor(start.rows * 0.8));
  let max = $state(start.contract.rows?.max ?? Math.ceil(start.rows * 1.2));
  let cols = $state(
    start.contract.columns.map((c, i) => ({
      title: c.title,
      aliases: (c.aliases ?? []).join(", "),
      ty: c.ty,
      // Changing a saved contract: its lists stay ticked. A new one: none ticked.
      allowed: was !== null && !!c.allowed,
      values: start.values.find((v) => v.col === i) ?? null,
      list: c.allowed?.values ?? [],
    })),
  );
  let problem = $state("");

  const typeWords = (t: unknown): string => (t === "number" ? "Number" : t === "text" ? "Text" : t === "id" ? "ID" : t && typeof t === "object" ? "Date" : "Any");

  async function tick(i: number, box: HTMLInputElement): Promise<void> {
    const c = cols[i]!;
    const on = box.checked;
    if (on && c.values?.looks_personal && !(await confirmPersonal(c.title))) {
      c.allowed = false;
      box.checked = false;
      return;
    }
    c.allowed = on;
  }

  function save(): void {
    const bad = nameProblem(name);
    if (bad) {
      problem = bad;
      return;
    }
    if (pattern.trim() === "" && !byTitles) {
      problem = "Choose how the next file is recognised: a file-name pattern, the same titles, or both.";
      return;
    }
    if (rowsOn && !(min <= max)) {
      problem = "The row range is the wrong way round.";
      return;
    }
    const c: Contract = {
      contract: 1,
      name,
      columns: cols.map((x, i) => {
        const base = start.contract.columns[i]!;
        const aliases = x.aliases.split(",").map((a) => a.trim()).filter((a) => a && a !== x.title).slice(0, 20);
        return {
          title: x.title,
          ...(aliases.length ? { aliases } : {}),
          ...(x.ty && x.ty !== "auto" ? { ty: x.ty } : {}),
          ...(x.allowed && x.list.length ? { allowed: { values: x.list, match_case: base.allowed?.match_case ?? true } } : {}),
          ...(base.blank_rate !== null && base.blank_rate !== undefined ? { blank_rate: base.blank_rate } : {}),
        };
      }),
      ...(withChecks && start.contract.checks ? { checks: start.contract.checks } : {}),
      rows: rowsOn ? { min: Math.max(0, Math.floor(min)), max: Math.max(0, Math.floor(max)) } : null,
      recognise: { pattern: pattern.trim() || null, titles: byTitles },
      blank_points: start.contract.blank_points,
    };
    onSave(c);
  }
</script>

<FormShell title={was ? `Change the contract ${was}` : "Make a contract from this file"} {onClose} onEnter={save}>
  <p class="note">A contract says what the next file of this kind should look like. When such a file opens, a bar above the table says what differs. It keeps the column titles, types, blank shares, the checks you include and the allowed values you tick, never other cells.</p>
  <label class="row">Name <input type="text" bind:value={name} maxlength="100" aria-label="Contract name" /></label>
  <fieldset>
    <legend>Recognise the next file by</legend>
    <label class="row">File name like <input type="text" bind:value={pattern} placeholder="voters_*.csv" maxlength="200" aria-label="File-name pattern" /> <span class="sub">* any characters, ? one</span></label>
    <label class="row"><input type="checkbox" bind:checked={byTitles} /> The same column titles (at least 80% of them)</label>
  </fieldset>
  <div class="cols" role="table" aria-label="Columns">
    <div class="head" role="row"><span role="columnheader">Column</span><span role="columnheader">Other titles (commas)</span><span role="columnheader">Type</span><span role="columnheader">Allowed values</span></div>
    {#each cols as c, i (i)}
      <div class="line" role="row">
        <span role="cell" class="title">{c.title}</span>
        <span role="cell"><input type="text" bind:value={c.aliases} aria-label="Other titles of {c.title}" /></span>
        <span role="cell">{typeWords(c.ty)}</span>
        <span role="cell">
          {#if c.values?.refused}
            <span class="sub" title={c.values.refused}>Too many ({c.values.refused})</span>
          {:else if c.list.length || c.values}
            <label title={c.list.slice(0, 30).join(", ")}><input type="checkbox" checked={c.allowed} aria-label="Keep the values of {c.title}" onchange={(e) => void tick(i, e.currentTarget as HTMLInputElement)} /> {c.list.length.toLocaleString()} values</label>
          {/if}
        </span>
      </div>
    {/each}
  </div>
  {#if checks > 0 || start.contract.checks}
    <label class="row"><input type="checkbox" bind:checked={withChecks} /> {checks ? `Include this tab's data checks (${checks})` : "Keep its data checks"}</label>
  {/if}
  <label class="row"><input type="checkbox" bind:checked={rowsOn} /> Expect between <input type="number" min="0" bind:value={min} disabled={!rowsOn} aria-label="Fewest rows" /> and <input type="number" min="0" bind:value={max} disabled={!rowsOn} aria-label="Most rows" /> data rows <span class="sub">(this file has {start.rows.toLocaleString()})</span></label>
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" class="primary" onclick={save}>Save</button>
    <button type="button" onclick={onClose}>Cancel</button>
  {/snippet}
</FormShell>

<style>
  .cols {
    display: grid;
    grid-template-columns: minmax(90px, 1fr) minmax(120px, 1.4fr) auto auto;
    gap: 4px 10px;
    max-height: 40vh;
    overflow: auto;
    margin: 8px 0;
  }
  .head,
  .line {
    display: contents;
  }
  .head span {
    font-weight: 600;
  }
  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .problem {
    color: var(--danger, #b91c1c);
  }
  input[type="number"] {
    width: 6em;
  }
</style>
