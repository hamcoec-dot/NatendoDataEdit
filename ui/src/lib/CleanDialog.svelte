<script lang="ts">
  // Cleaning tools: pick a tool, set its options, see what it would change,
  // run it, or add it to a cleaner (a saved list of steps run in order).
  import { untrack } from "svelte";
  import { backend, type ReadOptions, type Scope } from "./backend";
  import {
    CASE_CHOICES,
    KIND_CHOICES,
    MAX_STEPS,
    NAME_CHOICES,
    FIX_CHOICES,
    CALC_CHOICES,
    SPACES_CHOICES,
    TOOLS,
    copyStep,
    describe,
    localNow,
    stepProblem,
    toolOf,
    PHONE_LOOK,
    type PhoneStyle,
    MIN_PHRASE,
    type Cleaner,
    type CleanPreview,
    type CleanSpec,
    type Step,
    type ToolId,
  } from "./clean";
  import FormShell from "./FormShell.svelte";

  interface Props {
    doc: number;
    titles: string[];
    header: boolean;
    sel: { row: number; rows: number; col: number; cols: number };
    /** The cleaner being built (kept between visits). */
    cleaner: Cleaner;
    tool?: ToolId;
    onRun: (spec: CleanSpec, header: boolean, what: string) => void;
    onSay: (text: string) => void;
    onClose: () => void;
  }
  let { doc, titles, header, sel, cleaner = $bindable(), tool = "spaces", onRun, onSay, onClose }: Props = $props();

  let step = $state<Step>(untrack(() => toolOf(tool).make()));
  /** The cleaner step being changed, if one was picked from the list. */
  let editing = $state<number | null>(null);
  type Where = "selection" | "columns" | "all";
  let where = $state<Where>(untrack(() => (sel.rows > 1 || sel.cols > 1 ? "selection" : "columns")));
  let myHeader = $state(untrack(() => header));
  let dayFirst = $state(false);
  let decimalComma = $state(false);
  let previewAll = $state(false);
  let preview = $state<CleanPreview | null>(null);
  /** Sanitize pages through every changed cell (exact counts): which page, and the options it was asked for. */
  const PAGE = 50;
  let page = $state(0);
  let pageKey = "";
  let problem = $state("");
  let busy = $state(false);

  const groups = [...new Set(TOOLS.map((t) => t.group))];

  function pick(id: ToolId): void {
    step = toolOf(id).make();
    editing = null;
  }

  const scope = $derived.by((): Scope => {
    if (where === "selection") return { kind: "range", row: sel.row, rows: sel.rows, col: sel.col, cols: sel.cols };
    if (where === "columns") return { kind: "columns", cols: Array.from({ length: sel.cols }, (_, i) => sel.col + i) };
    return { kind: "all" };
  });
  const read = $derived<ReadOptions>({ decimal_comma: decimalComma, date_order: dayFirst ? "dmy" : "mdy" });
  const whereWords = $derived(
    where === "selection" ? "the selected cells" : where === "columns" ? (sel.cols === 1 ? `column ${titles[sel.col] ?? sel.col + 1}` : `${sel.cols} columns`) : "every column",
  );

  function specOf(steps: Step[]): CleanSpec {
    return {
      cleaner: { version: 1, name: cleaner.name, steps: $state.snapshot(steps) as Step[] },
      scope: $state.snapshot(scope) as Scope,
      skip_first: myHeader && where !== "selection",
      read: $state.snapshot(read) as ReadOptions,
    };
  }

  const stepProblemText = $derived(stepProblem(step));
  const previewSteps = $derived(previewAll && cleaner.steps.length ? cleaner.steps : [step]);

  // Preview, a moment after the last change.
  $effect(() => {
    const steps = previewSteps;
    const bad = previewAll && cleaner.steps.length ? cleaner.steps.map(stepProblem).find(Boolean) ?? "" : stepProblemText;
    const spec = specOf(steps);
    if (bad) {
      preview = null;
      return;
    }
    // Sanitize: a page of the changes with exact counts (every changed cell can be listed, a page at a time).
    const paged = !previewAll && steps.length === 1 && steps[0]!.tool === "sanitize";
    const key = JSON.stringify(spec);
    if (key !== pageKey) {
      pageKey = key;
      if (page !== 0) page = 0;
    }
    const at = paged ? page : 0;
    const timer = setTimeout(() => {
      (paged ? backend().cleanPreviewPage(doc, spec, localNow(), at * PAGE, PAGE) : backend().cleanPreview(doc, spec, localNow()))
        .then((p) => {
          preview = p;
          problem = "";
        })
        .catch((e: unknown) => {
          preview = null;
          problem = (e as { message?: string }).message ?? String(e);
        });
    }, 200);
    return () => clearTimeout(timer);
  });

  function addToCleaner(): void {
    if (stepProblemText) return;
    if (editing !== null) {
      cleaner.steps[editing] = copyStep(step);
      editing = null;
      return;
    }
    if (cleaner.steps.length >= MAX_STEPS) {
      problem = `A cleaner can have at most ${MAX_STEPS} steps.`;
      return;
    }
    cleaner.steps = [...cleaner.steps, copyStep(step)];
  }

  function edit(i: number): void {
    const s = cleaner.steps[i];
    if (!s) return;
    step = copyStep(s);
    editing = i;
  }

  function move(i: number, d: -1 | 1): void {
    const j = i + d;
    if (j < 0 || j >= cleaner.steps.length) return;
    const list = [...cleaner.steps];
    [list[i], list[j]] = [list[j]!, list[i]!];
    cleaner.steps = list;
    if (editing === i) editing = j;
  }

  function drop(i: number): void {
    cleaner.steps = cleaner.steps.filter((_, k) => k !== i);
    if (editing === i) editing = null;
  }

  async function save(): Promise<void> {
    try {
      busy = true;
      const name = await backend().cleanerSave($state.snapshot(cleaner) as Cleaner);
      if (name) onSay(`Saved the cleaner as ${name}.`);
    } catch (e) {
      problem = (e as { message?: string }).message ?? String(e);
    } finally {
      busy = false;
    }
  }

  async function open(): Promise<void> {
    try {
      busy = true;
      const c = await backend().cleanerOpen();
      if (c) {
        cleaner = c;
        editing = null;
        previewAll = true;
      }
    } catch (e) {
      problem = (e as { message?: string }).message ?? String(e);
    } finally {
      busy = false;
    }
  }

  function runTool(): void {
    if (stepProblemText) {
      problem = stepProblemText;
      return;
    }
    onRun(specOf([step]), myHeader, describe(step));
  }

  function runCleaner(): void {
    const bad = cleaner.steps.map(stepProblem).find(Boolean);
    if (bad) {
      problem = bad;
      return;
    }
    onRun(specOf(cleaner.steps), myHeader, cleaner.name || "the cleaner");
  }

  const num = (e: Event) => Number((e.currentTarget as HTMLInputElement).value);
</script>

<FormShell title="Cleaning tools" wide {onClose} onEnter={cleaner.steps.length && previewAll ? runCleaner : runTool}>
  <div class="layout">
    <nav class="tools" aria-label="Tools">
      {#each groups as g (g)}
        <div class="group">{g}</div>
        {#each TOOLS.filter((t) => t.group === g) as t (t.id)}
          <button type="button" class:on={step.tool === t.id} aria-pressed={step.tool === t.id} onclick={() => pick(t.id)}>{t.label}</button>
        {/each}
      {/each}
    </nav>

    <div class="main">
      <fieldset>
        <legend>{toolOf(step.tool).label}{editing !== null ? ` (step ${editing + 1} of the cleaner)` : ""}</legend>
        {#if step.tool === "replace"}
          <label class="row">Find <input type="text" bind:value={step.find.text} /></label>
          <label class="row">Replace with <input type="text" bind:value={step.with} /></label>
          <div class="row">
            <label><input type="radio" value="plain" bind:group={step.find.kind} /> Text</label>
            <label><input type="radio" value="wildcard" bind:group={step.find.kind} /> Wildcards (* ?)</label>
            <label><input type="radio" value="regex" bind:group={step.find.kind} /> Pattern (regular expression)</label>
          </div>
          <div class="row">
            <label><input type="checkbox" bind:checked={step.find.match_case} /> Match case</label>
            <label><input type="checkbox" bind:checked={step.find.whole_cell} /> Whole cell</label>
          </div>
        {:else if step.tool === "dates"}
          <div class="row">
            <label><input type="radio" name="dfrom" checked={!step.from} onchange={() => { if (step.tool === "dates") step.from = ""; }} /> Recognise the dates in the cells</label>
            <label><input type="radio" name="dfrom" checked={step.from !== undefined && step.from !== ""} onchange={() => { if (step.tool === "dates") step.from = "yyyymmdd"; }} /> The cells are written as…</label>
          </div>
          {#if step.from}
            <label class="row">Cells are written as <input type="text" bind:value={step.from} list="date-patterns" /></label>
            <datalist id="date-patterns">
              {#each ["yyyymmdd", "mmddyyyy", "ddmmyyyy", "yymmdd", "mm/dd/yyyy", "dd/mm/yyyy", "dd.mm.yyyy", "yyyy-mm-dd", "mmm d, yyyy", "d mmmm yyyy", "mm/dd/yyyy h:mm AM/PM"] as p (p)}<option value={p}></option>{/each}
            </datalist>
          {/if}
          <label class="row">Write dates as <input type="text" bind:value={step.format} /></label>
          <label class="row"><input type="checkbox" checked={!!step.past} onchange={(e) => { if (step.tool === "dates") step.past = (e.currentTarget as HTMLInputElement).checked; }} /> Two-digit years are in the past (birth dates: 26 is 1926, not 2026)</label>
          <p class="note">yyyy year, mm month (mmm Mar, mmmm March), dd day (ddd Wed), hh hours, mm minutes after hh, ss seconds, AM/PM. Text in "quotes" is written as it is. Cells that do not hold a date are left alone. Give the pattern for dates that are not recognised (20261113) or to say which way round day and month are.</p>
        {:else if step.tool === "numbers"}
          <label class="row"
            >Decimals
            <select value={step.decimals === null ? "" : String(step.decimals)} onchange={(e) => { if (step.tool === "numbers") { const v = (e.currentTarget as HTMLSelectElement).value; step.decimals = v === "" ? null : Number(v); } }}>
              <option value="">As they are</option>
              {#each [0, 1, 2, 3, 4, 5, 6] as d (d)}<option value={String(d)}>{d}</option>{/each}
            </select>
          </label>
          <label class="row"><input type="checkbox" bind:checked={step.thousands} /> Thousands separators (1,234)</label>
          <label class="row"><input type="checkbox" bind:checked={step.decimal_comma} /> Write a decimal comma (1.234,5)</label>
          <p class="note">Percentages and cells that are not numbers are left alone.</p>
        {:else if step.tool === "add"}
          <label class="row">Text to add <input type="text" bind:value={step.text} /></label>
          <label class="row"
            >Where
            <select value={step.at.kind} onchange={(e) => { if (step.tool !== "add") return; const k = (e.currentTarget as HTMLSelectElement).value; step.at = k === "position" ? { kind: "position", n: 1 } : k === "before" || k === "after" ? { kind: k, text: "" } : { kind: k as "start" | "end" }; }}>
              <option value="start">At the start</option>
              <option value="end">At the end</option>
              <option value="position">After a number of characters</option>
              <option value="before">Before some text</option>
              <option value="after">After some text</option>
            </select>
          </label>
          {#if step.at.kind === "position"}
            <label class="row">Characters <input type="number" min="0" max="100000" value={step.at.n} oninput={(e) => { if (step.tool === "add" && step.at.kind === "position") step.at.n = Math.max(0, num(e) | 0); }} /></label>
          {:else if step.at.kind === "before" || step.at.kind === "after"}
            <label class="row">Text to look for <input type="text" bind:value={step.at.text} /></label>
          {/if}
        {:else if step.tool === "random"}
          <label class="row">Characters <input type="number" min="1" max="10000" bind:value={step.length} /></label>
          <label class="row"
            >From
            <select value={step.chars.kind} onchange={(e) => { if (step.tool !== "random") return; const k = (e.currentTarget as HTMLSelectElement).value; step.chars = k === "custom" ? { kind: "custom", chars: "ABC123" } : ({ kind: k } as typeof step.chars); }}>
              <option value="letters_digits">Letters and digits</option>
              <option value="letters">Letters</option>
              <option value="upper">Capital letters</option>
              <option value="lower">Small letters</option>
              <option value="digits">Digits</option>
              <option value="hex">Hexadecimal (0-9, a-f)</option>
              <option value="custom">These characters…</option>
            </select>
          </label>
          {#if step.chars.kind === "custom"}<label class="row">Characters <input type="text" bind:value={step.chars.chars} /></label>{/if}
          <p class="note">Each cell gets new random text (not for passwords). Undo puts the old text back.</p>
        {:else if step.tool === "pad"}
          <label class="row">To a width of <input type="number" min="1" max="10000" bind:value={step.width} /> characters</label>
          <label class="row">With <input type="text" maxlength="2" class="short" bind:value={step.with} /></label>
          <div class="row">
            <label><input type="radio" value={false} bind:group={step.right} /> On the left (00042)</label>
            <label><input type="radio" value={true} bind:group={step.right} /> On the right</label>
          </div>
        {:else if step.tool === "remove"}
          <label class="row"
            >Remove
            <select value={step.what.kind} onchange={(e) => { if (step.tool !== "remove") return; const k = (e.currentTarget as HTMLSelectElement).value; step.what = k === "first" || k === "last" ? { kind: k, n: 1 } : k === "range" ? { kind: "range", from: 0, count: 1 } : k === "before" || k === "after" ? { kind: k, text: "", inclusive: false } : k === "matching" ? { kind: "matching", find: { text: "", kind: "plain", match_case: false, whole_cell: false } } : k === "kind" ? { kind: "kind", of: "punctuation" } : { kind: "characters", chars: "" }; }}>
              <option value="first">The first characters</option>
              <option value="last">The last characters</option>
              <option value="range">Characters from a position</option>
              <option value="before">What comes before some text</option>
              <option value="after">What comes after some text</option>
              <option value="matching">Text that matches</option>
              <option value="kind">Characters of a kind</option>
              <option value="characters">These characters</option>
            </select>
          </label>
          {#if step.what.kind === "first" || step.what.kind === "last"}
            <label class="row">How many <input type="number" min="0" max="100000" bind:value={step.what.n} /></label>
          {:else if step.what.kind === "range"}
            <label class="row">After <input type="number" min="0" max="100000" bind:value={step.what.from} /> characters, remove <input type="number" min="0" max="100000" bind:value={step.what.count} /></label>
          {:else if step.what.kind === "before" || step.what.kind === "after"}
            <label class="row">Text <input type="text" bind:value={step.what.text} /></label>
            <label class="row"><input type="checkbox" bind:checked={step.what.inclusive} /> Remove that text too</label>
          {:else if step.what.kind === "matching"}
            <label class="row">Text <input type="text" bind:value={step.what.find.text} /></label>
            <div class="row">
              <label><input type="radio" value="plain" bind:group={step.what.find.kind} /> Text</label>
              <label><input type="radio" value="wildcard" bind:group={step.what.find.kind} /> Wildcards</label>
              <label><input type="radio" value="regex" bind:group={step.what.find.kind} /> Pattern</label>
              <label><input type="checkbox" bind:checked={step.what.find.match_case} /> Match case</label>
            </div>
          {:else if step.what.kind === "kind"}
            <select bind:value={step.what.of} aria-label="Kind of characters">
              {#each Object.entries(KIND_CHOICES) as [k, label] (k)}<option value={k}>{label}</option>{/each}
            </select>
          {:else if step.what.kind === "characters"}
            <label class="row">Characters <input type="text" bind:value={step.what.chars} /></label>
          {/if}
        {:else if step.tool === "spaces"}
          {#each Object.entries(SPACES_CHOICES) as [k, label] (k)}
            <label class="row"><input type="radio" value={k} bind:group={step.how} /> {label}</label>
          {/each}
        {:else if step.tool === "sanitize"}
          <p class="note">Fixes what looks right but is not. Look-alike letters are changed only inside a word that mixes Latin letters with Cyrillic or Greek ones; a word that is all Cyrillic or all Greek is never touched.</p>
          <label class="row"><input type="checkbox" bind:checked={step.zero_width} /> Remove zero-width characters, the byte order mark and soft hyphens</label>
          <label class="row"><input type="checkbox" bind:checked={step.odd_spaces} /> Odd spaces (no-break, wide, thin) to a plain space</label>
          <label class="row"><input type="checkbox" bind:checked={step.smart_quotes} /> Smart quotes to straight quotes</label>
          <label class="row"><input type="checkbox" bind:checked={step.homoglyphs} /> Look-alike letters to their Latin twin (inside mixed-script words only)</label>
          <label class="row"><input type="checkbox" bind:checked={step.tabs_breaks} /> Tabs and line breaks inside a cell to a space</label>
        {:else if step.tool === "number"}
          <label class="row">Start at <input type="number" bind:value={step.start} /> and add <input type="number" bind:value={step.step} /> each row</label>
          <label class="row">At least <input type="number" min="0" max="30" bind:value={step.digits} /> digits (001)</label>
          <div class="row">
            <label><input type="radio" value={true} bind:group={step.replace} /> Replace the cell</label>
            <label><input type="radio" value={false} bind:group={step.replace} /> Add to the cell</label>
          </div>
          {#if !step.replace}
            <div class="row">
              <label><input type="radio" value={false} bind:group={step.at_end} /> At the start</label>
              <label><input type="radio" value={true} bind:group={step.at_end} /> At the end</label>
              <label>with <input type="text" class="short" bind:value={step.separator} /> between</label>
            </div>
          {/if}
        {:else if step.tool === "digits"}
          <label class="row"><input type="radio" value={false} bind:group={step.keep} /> Remove the digits</label>
          <label class="row"><input type="radio" value={true} bind:group={step.keep} /> Keep only the digits</label>
        {:else if step.tool === "now"}
          <label class="row">Format <input type="text" bind:value={step.format} /></label>
          <div class="row">
            <label><input type="radio" value={true} bind:group={step.replace} /> Replace the cell</label>
            <label><input type="radio" value={false} bind:group={step.replace} /> Add to the cell</label>
          </div>
          {#if !step.replace}
            <div class="row">
              <label><input type="radio" value={false} bind:group={step.at_end} /> At the start</label>
              <label><input type="radio" value={true} bind:group={step.at_end} /> At the end</label>
              <label>with <input type="text" class="short" bind:value={step.separator} /> between</label>
            </div>
          {/if}
          <p class="note">Every cell gets the same moment: when you press Run.</p>
        {:else if step.tool === "case"}
          {#each Object.entries(CASE_CHOICES) as [k, label] (k)}
            <label class="row"><input type="radio" value={k} bind:group={step.to} /> {label}</label>
          {/each}
        {:else if step.tool === "name"}
          {#each Object.entries(NAME_CHOICES) as [k, label] (k)}
            <label class="row"><input type="radio" value={k} bind:group={step.part} /> {label}</label>
          {/each}
          <p class="note">Reads names like "Dr. Mary Ann van der Berg Jr." and "Smith, John". To put the parts in new columns, use Data → Split names.</p>
        {:else if step.tool === "fix_number"}
          {#each Object.entries(FIX_CHOICES) as [k, label] (k)}
            <label class="row"><input type="radio" value={k} bind:group={step.how} /> {label}</label>
          {/each}
        {:else if step.tool === "calc"}
          <label class="row">
            <select bind:value={step.op}>
              {#each Object.entries(CALC_CHOICES) as [k, label] (k)}<option value={k}>{label}</option>{/each}
            </select>
            {#if step.op !== "abs"}<input type="number" step="any" bind:value={step.value} />{/if}
          </label>
          <p class="note">Cells that are not numbers are left as they are.</p>
        {:else if step.tool === "words"}
          <div class="row">
            <label><input type="radio" value={false} bind:group={step.money} /> Numbers: 12.5 → twelve point five</label>
            <label><input type="radio" value={true} bind:group={step.money} /> Amounts: 12.50 → twelve dollars and fifty cents</label>
          </div>
          <p class="note">English words. Cells that are not numbers are left as they are.</p>
        {:else if step.tool === "random_number"}
          <label class="row">From <input type="number" step="1" bind:value={step.min} /> to <input type="number" step="1" bind:value={step.max} /></label>
          <label class="row"><input type="checkbox" bind:checked={step.unique} /> Every cell of a column gets a different number</label>
          <p class="note">Each run gives new numbers (not for passwords). With different numbers, cells past the range are left as they are.</p>
        {:else if step.tool === "mask"}
          <label class="row">Keep the last <input type="number" min="0" max="10000" bind:value={step.keep_last} /> letters and digits</label>
          <label class="row">Hide the rest with <input type="text" maxlength="2" class="short" bind:value={step.with} /></label>
          <label class="row"><input type="checkbox" bind:checked={step.all} /> Hide punctuation and spaces too</label>
          <p class="note">123-45-6789 becomes ***-**-6789. The hidden characters cannot be brought back from the copy.</p>
        {:else if step.tool === "date_band"}
          <label class="row"><input type="radio" checked={step.to.kind === "year"} onchange={() => { if (step.tool === "date_band") step.to = { kind: "year" }; }} /> The year only (1980)</label>
          <label class="row"><input type="radio" checked={step.to.kind === "year_month"} onchange={() => { if (step.tool === "date_band") step.to = { kind: "year_month" }; }} /> The year and month (1980-03)</label>
          <label class="row"><input type="radio" checked={step.to.kind === "age_band"} onchange={() => { if (step.tool === "date_band") step.to = { kind: "age_band", size: 10, top: 90 }; }} /> The age today, in bands</label>
          {#if step.to.kind === "age_band"}
            <label class="row">Bands of <input type="number" min="1" max="150" bind:value={step.to.size} /> years (1: the age itself)</label>
            <label class="row">Ages from <input type="number" min="0" max="150" bind:value={step.to.top} /> up in one band (0: none)</label>
          {/if}
          <p class="note">Birth dates become less exact. Cells that are not dates are left as they are.</p>
        {:else if step.tool === "address"}
          <label class="row"><input type="radio" checked={step.style === "short"} onchange={() => { if (step.tool === "address") step.style = "short"; }} /> Short: 123 N Main St Apt 4</label>
          <label class="row"><input type="radio" checked={step.style === "long"} onchange={() => { if (step.tool === "address") step.style = "long"; }} /> Written out: 123 North Main Street Apartment 4</label>
          <p class="note">Only the street type, the directions and unit words change, in their usual places (the name in "12 Court St" stays). Addresses in capitals stay in capitals. Use Extract ▸ Unit to move "Apt 4" to its own column.</p>
        {:else if step.tool === "zip"}
          <label class="row"><input type="radio" checked={step.style === "keep"} onchange={() => { if (step.tool === "zip") step.style = "keep"; }} /> Keep the extra four digits (37402-1234)</label>
          <label class="row"><input type="radio" checked={step.style === "five"} onchange={() => { if (step.tool === "zip") step.style = "five"; }} /> Five digits only (37402)</label>
          <p class="note">Leading zeros a spreadsheet dropped are put back (2134 becomes 02134). Cells that are not a ZIP code are left and counted.</p>
        {:else if step.tool === "phone"}
          {#each Object.entries(PHONE_LOOK) as [k, look] (k)}
            <label class="row"><input type="radio" checked={step.style === k} onchange={() => { if (step.tool === "phone") step.style = k as PhoneStyle; }} /> {look}</label>
          {/each}
          <p class="note">US numbers: 10 digits, or 11 starting with 1. Extensions are kept (x45). Other cells are left and counted.</p>
        {:else if step.tool === "email"}
          <label class="row"><input type="checkbox" bind:checked={step.fix_domains} /> Fix common misspellings (gmial.com → gmail.com)</label>
          <p class="note">Spaces, "mailto:" and brackets are taken off and the address is written in small letters. Cells that are not an e-mail address are left and counted.</p>
        {:else if step.tool === "scramble"}
          <label class="row">Secret phrase <input type="password" autocomplete="off" spellcheck="false" bind:value={step.phrase} /></label>
          <label class="row"><input type="checkbox" checked={!!step.loose} onchange={(e) => { if (step.tool === "scramble") step.loose = (e.currentTarget as HTMLInputElement).checked; }} /> Ignore spaces, punctuation and case (123-45-6789 and 123456789 match)</label>
          <p class="note">Each value becomes 16 letters and digits. The same value always gives the same result with the same phrase, so scrambled columns still match and count. <strong>Anyone with the phrase can work out the originals</strong> (Social Security numbers and birth dates by trying them all): use a long phrase (at least {MIN_PHRASE} characters, several words), never reuse it, and choose Leave out instead when sharing outside the office. The phrase is never saved: a saved cleaner asks for it again.</p>
        {:else if step.tool === "sort_parts"}
          <label class="row">Parts are separated by <input type="text" class="short" bind:value={step.separator} /></label>
          <div class="row">
            <label><input type="radio" value={false} bind:group={step.descending} /> A → Z</label>
            <label><input type="radio" value={true} bind:group={step.descending} /> Z → A</label>
          </div>
          <label class="row"><input type="checkbox" bind:checked={step.trim} /> Ignore spaces around parts and drop empty parts</label>
          <label class="row"><input type="checkbox" bind:checked={step.distinct} /> Keep each part once</label>
        {:else}
          <p class="note">{describe(step)}.</p>
        {/if}
        {#if step.tool === "dates" || step.tool === "numbers" || step.tool === "fix_number" || step.tool === "calc" || step.tool === "words" || step.tool === "date_band"}
          <div class="row">
            {#if (step.tool === "dates" && !step.from) || step.tool === "date_band"}<label><input type="checkbox" bind:checked={dayFirst} /> Dates in the cells are day first (31/12)</label>{/if}
            <label><input type="checkbox" bind:checked={decimalComma} /> Numbers in the cells use a decimal comma</label>
          </div>
        {/if}
      </fieldset>

      <div class="row where">
        <span>Clean</span>
        <label><input type="radio" value="selection" bind:group={where} /> The selection</label>
        <label><input type="radio" value="columns" bind:group={where} /> {sel.cols === 1 ? "This column" : "These columns"}</label>
        <label><input type="radio" value="all" bind:group={where} /> Every column</label>
        {#if where !== "selection"}<label><input type="checkbox" bind:checked={myHeader} /> Leave the first row (titles) alone</label>{/if}
      </div>

      <div class="preview" aria-live="polite">
        <div class="row">
          <strong>Preview</strong>
          {#if cleaner.steps.length}<label><input type="checkbox" bind:checked={previewAll} /> Show the whole cleaner</label>{/if}
          <span class="note">
            {#if stepProblemText && !previewAll}{stepProblemText}
            {:else if preview}{preview.changed.toLocaleString()} {preview.changed === 1 ? "cell changes" : "cells change"} in the first {preview.rows.toLocaleString()} rows of {whereWords}{preview.more ? "" : ""}{preview.left ? `; ${preview.left.toLocaleString()} left as they are (not a date or number)` : ""}.
              {#if step.tool === "sanitize" && !previewAll}<strong class="warn" data-testid="homoglyph-count"> {(preview.homoglyph_cells ?? 0).toLocaleString()} {preview.homoglyph_cells === 1 ? "cell has" : "cells have"} a look-alike letter fixed to its Latin twin.</strong>{/if}
              {#if preview.other_order}<strong class="warn"> {preview.other_order.toLocaleString()} {preview.other_order === 1 ? "date" : "dates"} could only be read {dayFirst ? "month first (like 04/13/2026)" : "day first (like 13/04/2026)"} while the others are read {dayFirst ? "day first" : "month first"}: the column may mix the two. Check the "day first" box if the dates are day first.</strong>{/if}
            {/if}
          </span>
        </div>
        {#if step.tool === "sanitize" && !previewAll && preview && preview.changed > PAGE}
          <div class="row" data-testid="pager">
            <button type="button" class="small" disabled={page === 0} onclick={() => (page = Math.max(0, page - 1))}>Previous {PAGE}</button>
            <span class="note">Changed cells {(page * PAGE + 1).toLocaleString()} to {Math.min(preview.changed, (page + 1) * PAGE).toLocaleString()} of {preview.changed.toLocaleString()}</span>
            <button type="button" class="small" disabled={(page + 1) * PAGE >= preview.changed} onclick={() => (page += 1)}>Next {PAGE}</button>
          </div>
        {/if}
        <table>
          <thead><tr><th>Row</th><th>Column</th><th>Before</th><th>After</th></tr></thead>
          <tbody>
            {#each preview?.changes ?? [] as c (c.row * 100000 + c.col)}
              <tr><td>{c.number + 1}</td><td>{titles[c.col] ?? c.col + 1}</td><td class="text">{c.before}</td><td class="text">{c.after}</td></tr>
            {/each}
          </tbody>
        </table>
      </div>
    </div>

    <aside class="cleaner" aria-label="Cleaner">
      <label class="row">Cleaner <input type="text" placeholder="name" maxlength="200" bind:value={cleaner.name} /></label>
      <ol>
        {#each cleaner.steps as s, i (i)}
          <li class:on={editing === i}>
            <button type="button" class="step" onclick={() => edit(i)} title="Change this step">{describe(s)}</button>
            <button type="button" onclick={() => move(i, -1)} disabled={i === 0} aria-label="Move up">↑</button>
            <button type="button" onclick={() => move(i, 1)} disabled={i === cleaner.steps.length - 1} aria-label="Move down">↓</button>
            <button type="button" onclick={() => drop(i)} aria-label="Remove step">✕</button>
          </li>
        {:else}
          <li class="note">Add steps to build a cleaner you can run again and save.</li>
        {/each}
      </ol>
      <div class="row">
        <button type="button" onclick={addToCleaner} disabled={!!stepProblemText}>{editing !== null ? "Update step" : "Add to cleaner"}</button>
        <button type="button" onclick={() => void open()} disabled={busy}>Open…</button>
        <button type="button" onclick={() => void save()} disabled={busy || !cleaner.steps.length}>Save…</button>
        <button type="button" onclick={() => { cleaner.steps = []; editing = null; }} disabled={!cleaner.steps.length}>Clear</button>
      </div>
    </aside>
  </div>
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" class:primary={!cleaner.steps.length || !previewAll} onclick={runTool}>Run this tool</button>
    {#if cleaner.steps.length}<button type="button" class:primary={previewAll} onclick={runCleaner}>Run the cleaner ({cleaner.steps.length})</button>{/if}
    <button type="button" onclick={onClose}>Close</button>
  {/snippet}
</FormShell>

<style>
  .layout {
    display: grid;
    grid-template-columns: 11rem minmax(0, 1fr) 15rem;
    gap: 0.75rem;
    min-height: 22rem;
  }
  .tools {
    display: flex;
    flex-direction: column;
    gap: 2px;
    overflow: auto;
    max-height: 30rem;
  }
  .tools .group {
    font-size: 0.75rem;
    color: var(--muted);
    margin-top: 0.4rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .tools button {
    text-align: left;
    background: transparent;
    border: 1px solid transparent;
    padding: 0.2rem 0.4rem;
    border-radius: 4px;
  }
  .tools button.on {
    background: var(--sel-bg, var(--accent));
    color: var(--sel-fg, inherit);
    border-color: var(--accent);
  }
  .main {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    min-width: 0;
  }
  .row {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    flex-wrap: wrap;
  }
  .where span {
    font-weight: 600;
  }
  input.short {
    width: 3rem;
  }
  .note {
    color: var(--muted);
    font-size: 0.85rem;
  }
  .preview {
    min-height: 0;
    overflow: auto;
    max-height: 14rem;
    border: 1px solid var(--line);
    border-radius: 4px;
    padding: 0.3rem;
  }
  table {
    border-collapse: collapse;
    width: 100%;
    font-size: 0.85rem;
  }
  th,
  td {
    text-align: left;
    padding: 0.1rem 0.4rem;
    border-bottom: 1px solid var(--line);
    vertical-align: top;
  }
  td.text {
    font-family: var(--mono, monospace);
    white-space: pre-wrap;
    word-break: break-word;
  }
  .cleaner {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    border-left: 1px solid var(--line);
    padding-left: 0.6rem;
    min-width: 0;
  }
  .cleaner ol {
    margin: 0;
    padding-left: 1.2rem;
    overflow: auto;
    max-height: 20rem;
  }
  .cleaner li {
    display: flex;
    gap: 2px;
    align-items: center;
  }
  .cleaner li.on .step {
    outline: 2px solid var(--accent);
  }
  .cleaner .step {
    flex: 1;
    text-align: left;
    background: transparent;
    border: none;
    padding: 0.1rem 0.2rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .problem {
    color: var(--danger, #c00);
  }
</style>
