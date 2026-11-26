<script lang="ts">
  // The value editor: every distinct value of a column with how many rows
  // hold it. Type a new value next to one to change every cell holding it,
  // merge look-alike spellings in one click, or apply a translation list
  // (old → new) pasted in or read from two columns of another tab. One undo
  // step; with a filter on, only shown rows change.
  import { untrack } from "svelte";
  import { backend, errorText, isCmdError, type DataOp, type Dictionary, type DictionaryItem, type LookAlikeMethod, type LookAlikes, type ReadDictionary, type UnmatchedOut, type ValueTally } from "./backend";
  import { dictFromEdits, editPairs, editsFromDict, mergeInto, parsePairs } from "./editing";
  import FormShell from "./FormShell.svelte";
  import type { OtherTable } from "./transfer";

  interface Props {
    doc: number;
    titles: string[];
    col: number;
    header: boolean;
    /** How look-alikes are grouped when it opens. */
    startMethod?: "letters" | LookAlikeMethod;
    others: OtherTable[];
    titlesOf: (id: number, header: boolean) => Promise<string[]>;
    onRun: (op: DataOp, header: boolean) => void;
    onClose: () => void;
  }
  let { doc, titles, col: startCol, header, startMethod = "letters", others, titlesOf, onRun, onClose }: Props = $props();

  let col = $state(untrack(() => startCol));
  let myHeader = $state(untrack(() => header));
  let mode = $state<"edit" | "list">("edit");
  let tally = $state<ValueTally | null>(null);
  let loading = $state(false);
  let problem = $state("");
  let search = $state("");
  /** Old value → new value typed in. */
  let edits = $state(new Map<string, string>());
  // Translation list.
  let from = $state<"paste" | "tab">("paste");
  let pasted = $state("");
  let other = $state<number>(untrack(() => others.find((o) => o.id !== doc)?.id ?? -1));
  let otherHeader = $state(true);
  let otherTitles = $state<string[]>([]);
  let oldCol = $state(0);
  let newCol = $state(1);
  let loose = $state(true);

  const SHOWN = 2000;

  // Group look-alikes by sound or spelling (the first choice is the groups
  // the list already shows: the same letters in other case, spacing or accents).
  let method = $state<"letters" | LookAlikeMethod>(untrack(() => startMethod));
  let found = $state<LookAlikes | null>(null);
  let grouping = $state(false);
  let groupProgress = $state<number | null>(null);
  let groupProblem = $state("");
  /** Values ticked out of their group, by group number then value. */
  let dropped = $state(new Map<string, string>());
  /** Group number → the value the group becomes (default: the first). */
  let winners = $state(new Map<number, string>());
  let gen = 0;
  /** The look-alike search's job name (to cancel it and nothing else). */
  const LOOK = "Looking for look-alikes";
  /** Which column (and title-row setting) the values listed were counted for:
   *  the search waits for the count of the column now chosen. */
  let countedFor = $state<string | null>(null);
  /** "Put all groups in the list" asks first, saying what it will add. */
  let confirmAll = $state<{ groups: number; values: number } | null>(null);

  // ---- merging by dragging (C6) ----
  /** The dialog's own undo: each merge's changed values and what they were before. */
  let history = $state<{ key: string; had: string | undefined }[][]>([]);
  /** Values ticked in the table (dragged or merged together). */
  let ticked = $state(new Set<string>());
  let lastTicked = -1;
  /** A drag under way: the values, their cells, where the pointer is and which row it is over. */
  let drag = $state<{ values: string[]; cells: number; x: number; y: number; over: string | null; moving: boolean } | null>(null);
  let dragFrom = { x: 0, y: 0 };
  /** The Merge into… box. */
  let mergeOpen = $state(false);
  let mergeTo = $state("");
  // ---- dictionaries ----
  let dictDir = $state("");
  let dictItems = $state<DictionaryItem[]>([]);
  let dictPick = $state("");
  /** The dictionary loaded, its settings and what loading it said. */
  let dict = $state<{ path: string; read: ReadDictionary } | null>(null);
  let conflicts = $state<{ value: string; mine: string; theirs: string }[]>([]);
  let dictNote = $state("");
  let unmatched = $state<UnmatchedOut | null>(null);
  let onlyNew = $state(false);
  /** How Change compares cells: as typed (the editor's way) or the loaded dictionary's way. */
  let applyCase = $state(true);
  let applyTrim = $state(false);
  let saveOpen = $state(false);
  let saveName = $state("");
  let saveAsk = $state<{ path: string; dict: Dictionary } | null>(null);
  let saveNote = $state("");
  const dictPairs = $derived<[string, string][]>(dict ? dict.read.dict.pairs : []);

  /** Run `f`, trying again for a few seconds while another job on the file
   *  is finishing (the file does one thing at a time). */
  async function whenFree<T>(f: () => Promise<T>, alive: () => boolean): Promise<T> {
    for (let i = 0; ; i++) {
      try {
        return await f();
      } catch (e) {
        if (i < 20 && alive() && errorText(e).includes("still running")) {
          await new Promise((r) => setTimeout(r, 300));
          continue;
        }
        throw e;
      }
    }
  }

  // Looks only once the values are counted (the file does one job at a time).
  $effect(() => {
    const m = method;
    const c = col;
    const h = myHeader;
    const counted = !loading && tally !== null && countedFor === `${c}:${h}`;
    found = null;
    confirmAll = null;
    groupProblem = "";
    dropped = new Map();
    winners = new Map();
    if (m === "letters" || !counted) return;
    const mine = ++gen;
    let running = true;
    grouping = true;
    groupProgress = null;
    const extra = m === "abbreviations" ? dictPairs : undefined;
    whenFree(() => backend().lookalikeGroups(doc, { col: c, keep_first: h, method: m, ...(extra?.length ? { extra } : {}) }), () => mine === gen)
      .then((r) => {
        if (mine === gen) found = r;
      })
      .catch((e: unknown) => {
        if (mine !== gen) return;
        groupProblem = isCmdError(e) && e.kind === "cancelled" ? "Stopped." : errorText(e);
      })
      .finally(() => {
        running = false;
        if (mine === gen) grouping = false;
      });
    return () => {
      gen++;
      // Changed while looking: stop, so the next job can start.
      if (running) void backend().cancelJob(doc, LOOK);
    };
  });

  $effect(() => {
    if (!grouping) return;
    const poll = window.setInterval(async () => {
      try {
        const j = await backend().jobProgress(doc);
        groupProgress = j ? j.fraction : null;
      } catch {
        // Closed meanwhile.
      }
    }, 200);
    return () => window.clearInterval(poll);
  });

  function stopGrouping(): void {
    void backend().cancelJob(doc, LOOK);
  }

  /** The value a group becomes. */
  function winnerOf(k: number, g: { value: string }[]): string {
    return winners.get(k) ?? g[0]!.value;
  }
  function isDropped(k: number, v: string): boolean {
    return dropped.get(`${k}\u0000${v}`) !== undefined;
  }
  function setDropped(k: number, v: string, out: boolean): void {
    const next = new Map(dropped);
    if (out) next.set(`${k}\u0000${v}`, v);
    else next.delete(`${k}\u0000${v}`);
    dropped = next;
  }
  /** Add the group's ticked values to `into`, as the winner. */
  function addGroup(into: Map<string, string>, k: number, g: { value: string }[]): void {
    const to = winnerOf(k, g);
    for (const v of g) {
      if (isDropped(k, v.value) || v.value === to) {
        if (v.value === to) into.delete(v.value);
        continue;
      }
      into.set(v.value, to);
    }
  }
  /** Put the group's ticked values into the list of changes, as the winner. */
  function mergeGroup(k: number, g: { value: string }[]): void {
    const next = new Map(edits);
    addGroup(next, k, g);
    commit(next);
  }
  /** A "maybe" group (state or country codes in a column that is mostly not states): never put in all at once. */
  function isMaybe(k: number): boolean {
    return !!found?.maybe?.[k];
  }
  /** Every group at once, with each group's suggested value (one copy of the
   *  list, not one per group: there can be tens of thousands of groups); "maybe" groups are left out. */
  function mergeAll(): void {
    if (!found) return;
    const next = new Map(edits);
    found.groups.forEach((g, k) => !isMaybe(k) && addGroup(next, k, g));
    commit(next);
    confirmAll = null;
  }
  /** What "Put all groups" would add, shown before it does. */
  function askMergeAll(): void {
    if (!found) return;
    const probe = new Map<string, string>();
    let n = 0;
    found.groups.forEach((g, k) => {
      if (isMaybe(k)) return;
      addGroup(probe, k, g);
      n++;
    });
    confirmAll = { groups: n, values: probe.size };
  }

  /** Make `next` the pending changes, remembering what each changed value was (Undo merge). */
  function commit(next: Map<string, string>): void {
    const step: { key: string; had: string | undefined }[] = [];
    for (const k of new Set([...edits.keys(), ...next.keys()])) if (edits.get(k) !== next.get(k)) step.push({ key: k, had: edits.get(k) });
    if (!step.length) return;
    history = [...history.slice(-199), step];
    edits = next;
  }
  /** Step back one merge (before anything is applied). */
  function undoMerge(): void {
    const step = history.at(-1);
    if (!step) return;
    history = history.slice(0, -1);
    const next = new Map(edits);
    for (const { key, had } of step) {
      if (had === undefined) next.delete(key);
      else next.set(key, had);
    }
    edits = next;
  }
  /** Rows holding these values (cells that will change). */
  function cellsOf(values: string[]): number {
    return values.reduce((n, v) => n + (countOf.get(v) ?? 0), 0);
  }
  function tick(i: number, v: string, e: MouseEvent): void {
    const next = new Set(ticked);
    if (e.shiftKey && lastTicked >= 0) {
      const [a, b] = [Math.min(lastTicked, i), Math.max(lastTicked, i)];
      for (const r of shown.slice(a, b + 1)) next.add(r.value);
    } else if (next.has(v)) next.delete(v);
    else next.add(v);
    lastTicked = i;
    ticked = next;
  }
  /** Merge the values into `target` (dropped on it, or chosen in Merge into…). */
  function mergeValues(values: string[], target: string): void {
    if (!target.trim()) return;
    commit(mergeInto(edits, values, target));
    ticked = new Set();
  }
  function down(e: PointerEvent, v: string): void {
    if (e.button !== 0) return;
    const values = ticked.has(v) ? [...ticked] : [v];
    dragFrom = { x: e.clientX, y: e.clientY };
    drag = { values, cells: cellsOf(values), x: e.clientX, y: e.clientY, over: null, moving: false };
    e.preventDefault();
  }
  function move(e: PointerEvent): void {
    if (!drag) return;
    const moving = drag.moving || Math.hypot(e.clientX - dragFrom.x, e.clientY - dragFrom.y) > 4;
    const row = (document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null)?.closest<HTMLElement>("tr[data-value]");
    const over = row?.dataset.value ?? null;
    drag = { ...drag, x: e.clientX, y: e.clientY, moving, over: over !== null && !drag.values.includes(over) ? over : null };
  }
  function up(): void {
    const d = drag;
    drag = null;
    if (d?.moving && d.over !== null) mergeValues(d.values, d.over);
  }
  function onWindowKey(e: KeyboardEvent): void {
    // Esc during a drag cancels it (and only it: the dialog stays open, nothing is merged).
    if (drag && e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      drag = null;
    }
  }
  function tableKey(e: KeyboardEvent): void {
    if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "m") {
      e.preventDefault();
      if (ticked.size) openMerge();
    } else if ((e.ctrlKey || e.metaKey) && !e.shiftKey && e.key.toLowerCase() === "z" && !(e.target instanceof HTMLInputElement && e.target.type === "text")) {
      e.preventDefault();
      undoMerge();
    }
  }
  function openMerge(): void {
    mergeTo = [...ticked].sort((a, b) => (countOf.get(b) ?? 0) - (countOf.get(a) ?? 0))[0] ?? "";
    mergeOpen = true;
  }
  function doMerge(): void {
    if (!mergeTo.trim()) return;
    mergeValues([...ticked], mergeTo);
    mergeOpen = false;
  }

  // ---- dictionaries ----
  async function listDicts(): Promise<void> {
    try {
      const [dir, items] = await backend().dictionaryList();
      dictDir = dir;
      dictItems = items;
      // A dictionary no longer there (moved or deleted) is not offered.
      if (!items.some((i) => i.path === dictPick && !i.problem)) dictPick = items.find((i) => !i.problem)?.path ?? "";
    } catch (e) {
      dictNote = errorText(e);
    }
  }
  $effect(() => {
    if (mode === "edit") void listDicts();
  });
  /** A dictionary's name in the list (the file's name too when two share a name). */
  function dictLabel(i: DictionaryItem): string {
    const twin = dictItems.filter((o) => o.name.toLowerCase() === i.name.toLowerCase()).length > 1;
    return `${i.name}${twin ? ` (${i.file})` : ""} · ${i.pairs.toLocaleString()} pairs${i.problem ? ` (cannot be used: ${i.problem})` : ""}`;
  }
  async function loadDict(path: string | null): Promise<void> {
    dictNote = "";
    if (!path) return;
    try {
      const r = await backend().dictionaryRead(path);
      const d = r.dict;
      const norm = (s: string) => {
        const x = d.trim ? s.trim() : s;
        return d.match_case ? x : x.toLowerCase();
      };
      const res = editsFromDict(edits, d.pairs, norm);
      commit(res.edits);
      conflicts = res.conflicts;
      dict = { path, read: r };
      const settings = d.match_case !== applyCase || d.trim !== applyTrim;
      applyCase = d.match_case;
      applyTrim = d.trim;
      const left = r.self_pairs ? ` ${r.self_pairs.toLocaleString()} ${r.self_pairs === 1 ? "pair" : "pairs"} from a value to itself left out.` : "";
      dictNote =
        `Loaded “${d.name || "dictionary"}”: ${res.added.toLocaleString()} ${res.added === 1 ? "pair" : "pairs"} put in the list of changes.${left}` +
        (settings ? ` This dictionary ${d.match_case ? "matches capitals" : "ignores capitals"} and ${d.trim ? "ignores" : "keeps"} spaces at the start and end, so Change does the same.` : "");
      unmatched = null;
      unmatched = await whenFree(() => backend().dictionaryUnmatched(doc, col, myHeader, d), () => true);
    } catch (e) {
      dictNote = errorText(e);
    }
  }
  async function loadOther(): Promise<void> {
    const p = await backend().dictionaryPick();
    if (p) await loadDict(p);
  }
  /** The dictionary's targets for the values the person had merged elsewhere. */
  function useTheirs(): void {
    let next = new Map(edits);
    for (const c of conflicts) next = mergeInto(next, [c.value], c.theirs);
    commit(next);
    conflicts = [];
  }
  function dictToSave(): Dictionary {
    const pairs = dictFromEdits(edits);
    return { name: saveName.trim() || (dict?.read.dict.name ?? ""), match_case: applyCase, trim: applyTrim, pairs };
  }
  async function saveDict(confirmed = false): Promise<void> {
    saveNote = "";
    const d: Dictionary = $state.snapshot(saveAsk?.dict ?? dictToSave());
    if (!d.pairs.length) {
      saveNote = "There are no merges to save yet.";
      return;
    }
    const path = saveAsk?.path ?? (await backend().dictionaryPickSave(d.name || "dictionary").catch(() => null));
    if (!path) return;
    try {
      const written = await backend().dictionaryWrite(doc, path, d, confirmed);
      saveAsk = null;
      saveOpen = false;
      saveNote = `Saved “${d.name || "dictionary"}” (${d.pairs.length.toLocaleString()} ${d.pairs.length === 1 ? "pair" : "pairs"}) to ${written}.`;
      void listDicts();
    } catch (e) {
      if (isCmdError(e) && e.kind === "private") {
        // Asked once, with the place already chosen; never saved without a yes.
        saveAsk = { path, dict: d };
        return;
      }
      saveNote = errorText(e);
    }
  }

  $effect(() => {
    const c = col;
    const h = myHeader;
    loading = true;
    problem = "";
    edits = new Map();
    let live = true;
    whenFree(() => backend().valueTally(doc, { col: c, keep_first: h }), () => live)
      .then((t) => {
        if (live) {
          tally = t;
          countedFor = `${c}:${h}`;
        }
      })
      .catch((e: unknown) => {
        if (live) problem = errorText(e);
      })
      .finally(() => {
        if (live) loading = false;
      });
    return () => {
      live = false;
    };
  });

  $effect(() => {
    const id = other;
    const h = otherHeader;
    if (mode !== "list" || from !== "tab" || id < 0) return;
    void titlesOf(id, h).then((t) => (otherTitles = t));
  });

  const shown = $derived.by(() => {
    if (!tally) return [];
    const q = search.trim().toLowerCase();
    const fresh = onlyNew && unmatched ? new Set(unmatched.values.map((v) => v.value)) : null;
    const list = tally.values.map((v, i) => ({ ...v, i })).filter((v) => !fresh || fresh.has(v.value));
    return (q ? list.filter((v) => v.value.toLowerCase().includes(q)) : list).slice(0, SHOWN);
  });

  function setEdit(old: string, value: string): void {
    const next = new Map(edits);
    if (value === old || value === "") next.delete(old);
    else next.set(old, value);
    edits = next;
  }

  /** Every member of a look-alike group becomes `to`. */
  function merge(members: number[], to: string): void {
    if (!tally) return;
    commit(mergeInto(edits, members.map((i) => tally!.values[i]!.value), to));
  }

  const pairs = $derived(mode === "edit" ? editPairs(edits) : from === "paste" ? parsePairs(pasted) : []);
  /** How many rows hold each value (looked up once per value, not searched). */
  const countOf = $derived(new Map((tally?.values ?? []).map((v) => [v.value, v.count] as const)));
  const changing = $derived(mode === "edit" && tally ? pairs.reduce((n, [old]) => n + (countOf.get(old) ?? 0), 0) : 0);

  function run(): void {
    problem = "";
    if (mode === "edit") {
      if (pairs.length === 0) {
        problem = "Type a new value next to at least one value, or merge a group.";
        return;
      }
      onRun({ op: "map_values", spec: { cols: [col], pairs: pairs.map(([a, b]) => [a, b]), match_case: applyCase, trim: applyTrim, keep_first: myHeader } }, myHeader);
      return;
    }
    if (from === "paste") {
      if (pairs.length === 0) {
        problem = "Paste the list: one old value and its new value per line, separated by a tab, = or a comma.";
        return;
      }
      onRun({ op: "map_values", spec: { cols: [col], pairs, match_case: !loose, trim: loose, keep_first: myHeader } }, myHeader);
      return;
    }
    if (other < 0) {
      problem = "Open the tab that holds the list first.";
      return;
    }
    if (oldCol === newCol) {
      problem = "Choose two different columns for the old and new values.";
      return;
    }
    onRun(
      {
        op: "map_values",
        spec: { cols: [col], pairs: [], match_case: !loose, trim: loose, keep_first: myHeader },
        from: { doc: other, old: oldCol, new: newCol, header: otherHeader },
      },
      myHeader,
    );
  }
</script>

<svelte:window onpointermove={move} onpointerup={up} onkeydowncapture={onWindowKey} />

<FormShell title="Edit values" {onClose} onEnter={run}>
  <div class="row">
    <label class="row">Column
      <select bind:value={col}>
        {#each titles as t, c (c)}<option value={c}>{t}</option>{/each}
      </select>
    </label>
    <label class="row"><input type="checkbox" bind:checked={myHeader} /> First row is header</label>
  </div>
  <div class="row">
    <label class="row"><input type="radio" name="vmode" value="edit" bind:group={mode} /> Change the values listed</label>
    <label class="row"><input type="radio" name="vmode" value="list" bind:group={mode} /> Use a translation list (old → new)</label>
  </div>
  {#if mode === "edit"}
    {#if loading}
      <p class="note">Counting values…</p>
    {:else if tally}
      <p class="note">
        {tally.values.length.toLocaleString()} different values in {tally.rows.toLocaleString()} rows{tally.blanks ? `, ${tally.blanks.toLocaleString()} blank` : ""}.
        {#if !tally.complete}Only the {tally.values.length.toLocaleString()} most common are listed.{/if}
        Type a new value next to one to change every cell that holds it.
      </p>
      <div class="row">
        <label class="row">Look-alikes:
          <select bind:value={method} aria-label="Group look-alikes by">
            <option value="letters">Same letters (case, spaces, accents)</option>
            <option value="sounds">Sound alike (Smith, Smyth)</option>
            <option value="spelling">Close spelling (Jonson, Johnson)</option>
            <option value="both">Sound or spelling</option>
            <option value="abbreviations">Abbreviations and full names (St = Street, NY = New York, USA = United States)</option>
          </select>
        </label>
      </div>
      {#if method !== "letters"}
        {#if grouping}
          <p class="note">
            Looking for look-alikes…{groupProgress !== null ? ` ${Math.round(groupProgress * 100)}%` : ""}
            <button type="button" class="small" onclick={stopGrouping}>Cancel</button>
          </p>
        {:else if groupProblem}
          <p class="problem" role="alert">{groupProblem}</p>
        {:else if found}
          <fieldset>
            <legend>{method === "sounds" ? "Sound alike" : method === "spelling" ? "Close spelling" : method === "abbreviations" ? "Abbreviations and full names" : "Sound or spelling"} ({found.groups.length.toLocaleString()} {found.groups.length === 1 ? "group" : "groups"})</legend>
            {#if !found.complete}<p class="note">Only the {found.values.toLocaleString()} most common values were looked at.</p>{/if}
            {#if found.too_long}<p class="note">{found.too_long.toLocaleString()} very long {found.too_long === 1 ? "value was" : "values were"} left out of spelling groups.</p>{/if}
            {#if found.groups.length === 0}
              <p class="note">No look-alikes found.</p>
            {:else}
              <p class="note">
                Each group becomes the value chosen next to it (the most common one to begin with). Untick a value to leave it alone, then put the group in the list of changes.
                <button type="button" class="small" onclick={askMergeAll}>Put all groups in the list</button>
              </p>
              {#if confirmAll}
                <p class="note" role="alert">
                  Put {confirmAll.groups.toLocaleString()} {confirmAll.groups === 1 ? "group" : "groups"} ({confirmAll.values.toLocaleString()} {confirmAll.values === 1 ? "value" : "values"}) in the list of changes? Look the groups over first: they are only suggestions.
                  <button type="button" class="small primary" onclick={mergeAll}>Put them in the list</button>
                  <button type="button" class="small" onclick={() => (confirmAll = null)}>Not now</button>
                </p>
              {/if}
              <div class="groups" aria-label="Look-alike groups">
                {#each found.groups.slice(0, 200) as g, k (k)}
                  {#if !isMaybe(k)}
                  <div class="row group">
                    {#each g as v (v.value)}
                      <label class="row"><input type="checkbox" checked={!isDropped(k, v.value)} onchange={(e) => setDropped(k, v.value, !(e.currentTarget as HTMLInputElement).checked)} aria-label={`Include ${v.value}`} /> {v.value} ({v.count.toLocaleString()})</label>
                    {/each}
                    <select value={winnerOf(k, g)} onchange={(e) => { const next = new Map(winners); next.set(k, (e.currentTarget as HTMLSelectElement).value); winners = next; }} aria-label="Becomes">
                      {#each g as v (v.value)}<option value={v.value}>{v.value}</option>{/each}
                    </select>
                    <button type="button" class="small" onclick={() => mergeGroup(k, g)}>Put in list</button>
                  </div>
                  {/if}
                {/each}
              </div>
              {#if found.maybe?.some(Boolean)}
                <p class="note" data-testid="maybe-note">Maybe states or countries: codes like OK, OR and IN are matched as states (and USA as a country) only one group at a time here, because most of this column is not states or countries. They are left out of "Put all groups".</p>
                <div class="groups maybe" aria-label="Maybe states or countries">
                  {#each found.groups.slice(0, 200) as g, k (k)}
                    {#if isMaybe(k)}
                      <div class="row group">
                        {#each g as v (v.value)}
                          <label class="row"><input type="checkbox" checked={!isDropped(k, v.value)} onchange={(e) => setDropped(k, v.value, !(e.currentTarget as HTMLInputElement).checked)} aria-label={`Include ${v.value}`} /> {v.value} ({v.count.toLocaleString()})</label>
                        {/each}
                        <select value={winnerOf(k, g)} onchange={(e) => { const next = new Map(winners); next.set(k, (e.currentTarget as HTMLSelectElement).value); winners = next; }} aria-label="Becomes">
                          {#each g as v (v.value)}<option value={v.value}>{v.value}</option>{/each}
                        </select>
                        <button type="button" class="small" onclick={() => mergeGroup(k, g)}>Put in list</button>
                      </div>
                    {/if}
                  {/each}
                </div>
              {/if}
              {#if found.groups.length > 200}<p class="note">Showing the first 200 groups.</p>{/if}
            {/if}
          </fieldset>
        {/if}
      {:else if tally.look_alikes.length}
        <fieldset>
          <legend>Look-alike spellings ({tally.look_alikes.length.toLocaleString()})</legend>
          <div class="groups">
            {#each tally.look_alikes.slice(0, 200) as g, k (k)}
              <div class="row">
                <span class="members">{g.members.map((i) => `${tally!.values[i]!.value} (${tally!.values[i]!.count.toLocaleString()})`).join(" · ")}</span>
                <select onchange={(e) => merge(g.members, (e.currentTarget as HTMLSelectElement).value)} aria-label="Merge into">
                  <option value="" selected disabled>Merge into…</option>
                  {#each g.members as i (i)}<option value={tally!.values[i]!.value}>{tally!.values[i]!.value}</option>{/each}
                </select>
              </div>
            {/each}
          </div>
        </fieldset>
      {/if}
      <fieldset class="dict" data-testid="dictionary">
        <legend>Dictionary</legend>
        <div class="row">
          <select bind:value={dictPick} aria-label="Saved dictionaries">
            {#if !dictItems.length}<option value="">(none saved yet)</option>{/if}
            {#each dictItems as d (d.path)}<option value={d.path} disabled={!!d.problem}>{dictLabel(d)}</option>{/each}
          </select>
          <button type="button" class="small" disabled={!dictPick} title={dictPick ? "" : "Save a dictionary first, or choose a file"} onclick={() => void loadDict(dictPick)}>Load</button>
          <button type="button" class="small" onclick={() => void loadOther()}>Choose a file…</button>
          <button type="button" class="small" disabled={!editPairs(edits).length} title={editPairs(edits).length ? "" : "Merge some values first: the merges are what a dictionary saves"} onclick={() => { saveOpen = true; saveName = saveName || (dict?.read.dict.name ?? ""); saveNote = ""; }}>Save as dictionary…</button>
        </div>
        {#if saveOpen}
          <div class="row">
            <label class="row">Name <input type="text" bind:value={saveName} placeholder="Counties" aria-label="Dictionary name" /></label>
            <button type="button" class="small primary" onclick={() => void saveDict()}>Save…</button>
            <button type="button" class="small" onclick={() => { saveOpen = false; saveAsk = null; }}>Not now</button>
          </div>
        {/if}
        {#if saveAsk}
          <p class="note" role="alert" data-testid="dict-private">
            This dictionary holds values from a private file. Save it anyway? It is only saved now, never by itself.
            <button type="button" class="small primary" onclick={() => void saveDict(true)}>Save anyway</button>
            <button type="button" class="small" onclick={() => (saveAsk = null)}>Do not save</button>
          </p>
        {/if}
        {#if saveNote}<p class="note" aria-live="polite">{saveNote}</p>{/if}
        {#if dictNote}<p class="note" aria-live="polite" data-testid="dict-note">{dictNote}</p>{/if}
        {#if conflicts.length}
          <p class="note" role="alert" data-testid="dict-conflicts">
            {conflicts.length.toLocaleString()} {conflicts.length === 1 ? "value is" : "values are"} merged differently by you and by the dictionary; your merges are kept:
            {conflicts.slice(0, 5).map((c) => `${c.value} (yours: ${c.mine}; dictionary: ${c.theirs})`).join("; ")}{conflicts.length > 5 ? " …" : ""}
            <button type="button" class="small" onclick={useTheirs}>Use the dictionary's for these</button>
          </p>
        {/if}
        {#if unmatched}
          <label class="row" data-testid="dict-new"><input type="checkbox" bind:checked={onlyNew} /> Show only values not in the dictionary ({unmatched.count.toLocaleString()} {unmatched.count === 1 ? "value" : "values"}, {unmatched.rows.toLocaleString()} rows): drag them into place</label>
        {/if}
        {#if !dictItems.length && dictDir}<p class="note">Dictionaries are kept in {dictDir}.</p>{/if}
      </fieldset>
      <div class="row tools">
        <button type="button" class="small" disabled={!ticked.size} title={ticked.size ? "" : "Tick values first"} onclick={openMerge}>Merge into… (Ctrl+M)</button>
        <button type="button" class="small" disabled={!history.length} title={history.length ? "" : "Nothing merged yet"} onclick={undoMerge}>Undo merge</button>
        {#if ticked.size}<span class="note">{ticked.size.toLocaleString()} ticked ({cellsOf([...ticked]).toLocaleString()} cells)</span>{/if}
      </div>
      {#if mergeOpen}
        <div class="row merge-box">
          <label class="row">Merge the ticked values into <input type="text" list="ve-values" bind:value={mergeTo} aria-label="Merge into" onkeydown={(e) => { if (e.key === "Enter") { e.preventDefault(); e.stopPropagation(); doMerge(); } else if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); mergeOpen = false; } }} /></label>
          <datalist id="ve-values">{#each shown.slice(0, 500) as v (v.i)}<option value={v.value}></option>{/each}</datalist>
          <button type="button" class="small primary" disabled={!mergeTo.trim()} onclick={doMerge}>Merge</button>
          <button type="button" class="small" onclick={() => (mergeOpen = false)}>Cancel</button>
        </div>
      {/if}
      <input type="search" bind:value={search} placeholder="Find a value" aria-label="Find a value" />
      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
      <div class="table-preview values" aria-label="Values and counts" tabindex="0" onkeydown={tableKey} role="group">
        <table>
          <tbody>
            <tr class="head"><td></td><td>Value</td><td>Rows</td><td>Change to</td></tr>
            {#each shown as v, i (v.i)}
              <tr class:edited={edits.has(v.value)} class:over={drag?.moving && drag.over === v.value} class:dragging={drag?.moving && drag.values.includes(v.value)} data-value={v.value}>
                <td class="grip-cell"><input type="checkbox" checked={ticked.has(v.value)} onclick={(e) => tick(i, v.value, e)} aria-label={`Tick ${v.value}`} /><span class="grip" title="Drag onto another value to merge them" role="presentation" onpointerdown={(e) => down(e, v.value)}>⠿</span></td>
                <td>{v.value}</td>
                <td class="num">{v.count.toLocaleString()}</td>
                <td><input type="text" value={edits.get(v.value) ?? ""} placeholder="(keep)" oninput={(e) => setEdit(v.value, (e.currentTarget as HTMLInputElement).value)} aria-label={`New value for ${v.value}`} /></td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      {#if shown.length >= SHOWN}<p class="note">Showing {SHOWN.toLocaleString()} values; find a value to see others.</p>{/if}
      {#if pairs.length}<p class="good">{pairs.length.toLocaleString()} value{pairs.length === 1 ? "" : "s"} will change ({changing.toLocaleString()} cells).</p>{/if}
      {#if drag?.moving}
        <div class="drag-chip" style:left="{drag.x + 14}px" style:top="{drag.y + 10}px" data-testid="drag-chip">
          {drag.values.length.toLocaleString()} {drag.values.length === 1 ? "value" : "values"} ({drag.cells.toLocaleString()} {drag.cells === 1 ? "cell" : "cells"}) → {drag.over ?? "…"}
        </div>
      {/if}
    {/if}
  {:else}
    <div class="row">
      <label class="row"><input type="radio" name="vfrom" value="paste" bind:group={from} /> Paste the list</label>
      <label class="row"><input type="radio" name="vfrom" value="tab" bind:group={from} /> Two columns of another tab</label>
    </div>
    {#if from === "paste"}
      <label class="col">One pair per line: old value, then the new one (separated by a tab, = or a comma)
        <textarea rows="6" bind:value={pasted} spellcheck="false" placeholder={"NY\tNew York\nCA\tCalifornia"}></textarea>
      </label>
      {#if pairs.length}<p class="good">{pairs.length.toLocaleString()} pair{pairs.length === 1 ? "" : "s"} read.</p>{/if}
    {:else}
      <label class="row">Tab
        <select bind:value={other}>
          {#each others as t (t.id)}<option value={t.id}>{t.name}{t.id === doc ? " (this one)" : ""}</option>{/each}
        </select>
      </label>
      <label class="row"><input type="checkbox" bind:checked={otherHeader} /> Its first row is its header</label>
      <div class="row">
        <label class="row">Old values in
          <select bind:value={oldCol}>{#each otherTitles as t, c (c)}<option value={c}>{t}</option>{/each}</select>
        </label>
        <label class="row">New values in
          <select bind:value={newCol}>{#each otherTitles as t, c (c)}<option value={c}>{t}</option>{/each}</select>
        </label>
      </div>
    {/if}
    <label class="row"><input type="checkbox" bind:checked={loose} /> Ignore case and spaces at the start and end</label>
    <p class="note">A whole cell must equal an old value; cells not on the list stay as they are.</p>
  {/if}
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" class="primary" onclick={run}>Change</button>
    <button type="button" onclick={onClose}>Cancel</button>
  {/snippet}
</FormShell>

<style>
  .values {
    max-height: 300px;
  }
  .values input[type="text"] {
    width: 14em;
  }
  td.num {
    text-align: right;
  }
  tr.edited td {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  tr.over td {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  tr.dragging td {
    opacity: 0.5;
  }
  .grip-cell {
    white-space: nowrap;
    width: 1%;
  }
  .grip {
    cursor: grab;
    padding: 0 4px;
    user-select: none;
    color: var(--muted);
  }
  .drag-chip {
    position: fixed;
    z-index: 1000;
    pointer-events: none;
    background: var(--accent);
    color: white;
    padding: 2px 8px;
    border-radius: 4px;
    font-size: 12px;
  }
  .groups.maybe {
    border-left: 3px solid var(--warn, #e69500);
    padding-left: 6px;
  }
  .groups {
    max-height: 140px;
    overflow: auto;
  }
  .group {
    flex-wrap: wrap;
    gap: 4px 12px;
    padding: 2px 0;
    border-bottom: 1px solid var(--border);
  }
  .members {
    font-size: 12px;
  }
  textarea {
    width: 100%;
    font-family: ui-monospace, monospace;
  }
  input[type="search"] {
    width: 100%;
    margin: 4px 0;
  }
</style>
