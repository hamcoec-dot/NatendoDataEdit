<script lang="ts">
  // Private data (Milestone 13): the columns that look private, and a
  // share-safe copy of the table with columns dropped, masked, made less
  // exact or scrambled. The open file never changes.
  import { untrack } from "svelte";
  import type { ColumnAction, Finding, PrivateKind, SafeCopy } from "./backend";
  import { MIN_PHRASE, type Step } from "./clean";
  import FormShell from "./FormShell.svelte";
  import { KIND_WORDS } from "./privacy";
  import { GUARD_LABEL } from "./csvguard";
  import { prefs } from "./prefs.svelte";

  type Choice = "keep" | "drop" | "mask" | "year" | "age" | "scramble";

  interface Props {
    titles: string[];
    /** Every column in the document. */
    cols: number;
    header: boolean;
    /** What the finder found (null while it looks). */
    findings: Finding[] | null;
    problem?: string;
    onRun: (spec: SafeCopy) => void;
    onClose: () => void;
  }
  let { titles, cols, header, findings, problem = "", onRun, onClose }: Props = $props();
  /** Preset from the setting; changing it here changes this copy only. */
  let guard = $state(prefs.data.csvFormulaGuard);

  /** Privacy-safe first choices for each kind. */
  const DEFAULT: Record<PrivateKind, Choice> = {
    ssn: "drop",
    birth_date: "age",
    phone: "mask",
    email: "drop",
    address: "drop",
    licence: "drop",
  };

  let choices = $state<Choice[]>(untrack(() => titles.map(() => "keep" as Choice)));
  let phrase = $state("");
  let dayFirst = $state(false);
  /** Kinds whose values are written many ways (dashes, spaces): scrambled
   *  loosely so copies still match. */
  const LOOSE: PrivateKind[] = ["ssn", "phone", "licence", "email"];
  let myProblem = $state("");
  let applied = false;
  $effect(() => {
    if (!findings || applied) return;
    applied = true;
    const next = [...untrack(() => choices)];
    for (const f of findings) if (f.col < next.length) next[f.col] = DEFAULT[f.kind];
    choices = next;
  });
  const foundAt = $derived(new Map((findings ?? []).map((f) => [f.col, f])));
  const tooMany = $derived(titles.length < cols);
  const needsPhrase = $derived(choices.includes("scramble"));

  function step(c: Choice, col = -1): Step | null {
    switch (c) {
      case "mask":
        return { tool: "mask", keep_last: 4, with: "*", all: false };
      case "year":
        return { tool: "date_band", to: { kind: "year" } };
      case "age":
        return { tool: "date_band", to: { kind: "age_band", size: 10, top: 90 } };
      case "scramble":
        return { tool: "scramble", phrase, loose: col < 0 ? false : LOOSE.includes(foundAt.get(col)?.kind ?? "address") };
      default:
        return null;
    }
  }

  function run(): void {
    myProblem = "";
    if (tooMany) {
      myProblem = "This file has too many columns to list here.";
      return;
    }
    if (choices.every((c) => c === "drop")) {
      myProblem = "Keep at least one column.";
      return;
    }
    if (needsPhrase && [...phrase].length < MIN_PHRASE) {
      myProblem = `Type a secret phrase of at least ${MIN_PHRASE} characters for scrambling.`;
      return;
    }
    const columns: ColumnAction[] = choices.map((c, col) => {
      if (c === "keep") return { action: "keep" };
      if (c === "drop") return { action: "drop" };
      return { action: "clean", cleaner: { version: 1, name: "", steps: [step(c, col)!] } };
    });
    onRun({ columns, header, read: { decimal_comma: false, date_order: dayFirst ? "dmy" : "mdy" }, guard });
  }

  const LABELS: [Choice, string][] = [
    ["keep", "Keep as it is"],
    ["drop", "Leave out"],
    ["mask", "Mask all but the last 4"],
    ["year", "Year only (dates)"],
    ["age", "Age band (dates)"],
    ["scramble", "Scramble"],
  ];
</script>

<FormShell title="Private data and share-safe copy" {onClose} onEnter={run}>
  {#if findings === null}
    <p class="note">Looking at the first 20,000 rows…</p>
  {:else if findings.length === 0}
    <p class="note">No column looks like it holds private data (Social Security, birth date, phone, e-mail, address, licence numbers). Check the list below anyway.</p>
  {:else}
    <p>These columns look like they hold private data:</p>
    <ul class="found">
      {#each findings as f (f.col)}
        <li><strong>{f.title}</strong>: {KIND_WORDS[f.kind]}{f.by_title ? "" : ` (${f.hits.toLocaleString()} of ${f.looked.toLocaleString()} values)`}</li>
      {/each}
    </ul>
  {/if}
  {#if findings !== null}<p class="note">Looked at the column titles and the first 20,000 rows only: check the other columns yourself.</p>{/if}
  {#if tooMany}
    <p class="problem">This file has {cols.toLocaleString()} columns; only {titles.length.toLocaleString()} can be listed here.</p>
  {:else}
    <div class="cols" role="list" aria-label="What to do with each column">
      {#each titles as t, c (c)}
        <div class="row" role="listitem" class:private={foundAt.has(c)}>
          <span class="name" title={t}>{t}</span>
          <select bind:value={choices[c]} aria-label={`What to do with ${t}`}>
            {#each LABELS as [v, label] (v)}<option value={v}>{label}</option>{/each}
          </select>
        </div>
      {/each}
    </div>
  {/if}
  {#if choices.includes("age") || choices.includes("year")}
    <label class="row"><input type="checkbox" bind:checked={dayFirst} /> Dates are day first (31/12/1980)</label>
  {/if}
  <label class="row"><input type="checkbox" data-testid="safe-guard" bind:checked={guard} /> {GUARD_LABEL}</label>
  {#if needsPhrase}
    <label class="row">Secret phrase <input type="password" autocomplete="off" spellcheck="false" bind:value={phrase} /></label>
    <p class="note">The same value always gives the same scrambled text with the same phrase, so copies still match each other (Social Security, phone, licence and e-mail columns ignore spaces, punctuation and case). <strong>Anyone with the phrase can work out the originals</strong>: use a long phrase (at least {MIN_PHRASE} characters, several words), never reuse it, and choose Leave out instead when sharing outside the office. The phrase is never saved.</p>
  {/if}
  <p class="note">A value a step cannot read (a date it does not understand, say) is left blank in the copy, never written as it was.</p>
  <p class="note">The copy is a new file: the open file does not change. Rows hidden by a filter are left out{header ? "; the first row (titles) is kept as it is" : ""}.</p>
  {#if myProblem || problem}<p class="problem" role="alert">{myProblem || problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" class="primary" onclick={run} disabled={tooMany}>Save a share-safe copy…</button>
    <button type="button" onclick={onClose}>Close</button>
  {/snippet}
</FormShell>

<style>
  .cols {
    max-height: 300px;
    overflow: auto;
  }
  .cols .name {
    min-width: 14em;
    max-width: 20em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cols .private .name {
    font-weight: 600;
  }
  .found {
    margin: 0 0 8px;
    padding-left: 18px;
  }
</style>
