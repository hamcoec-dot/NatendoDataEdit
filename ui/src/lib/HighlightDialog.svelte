<script lang="ts">
  // Highlight rules: colour cells (or whole rows) whose value meets a
  // condition. Saved with views; a delimited file cannot store colours.
  import { untrack } from "svelte";
  import type { Condition, Rule } from "./backend";
  import FormShell from "./FormShell.svelte";
  import { MAX_HIGHLIGHTS, PALETTE, type Highlight } from "./views";

  interface Props {
    titles: string[];
    col: number;
    rules: Highlight[];
    onSave: (rules: Highlight[]) => void;
    onClose: () => void;
  }
  let { titles, col, rules, onSave, onClose }: Props = $props();

  type Kind = "contains" | "equals" | "gt" | "lt" | "between" | "blank" | "not_blank" | "pattern";
  interface Row {
    col: number;
    kind: Kind;
    a: string;
    b: string;
    color: number;
    whole_row: boolean;
  }
  function toRow(h: Highlight): Row {
    const base = { col: h.col, color: h.color, whole_row: h.whole_row, a: "", b: "" };
    if (h.rule.type === "values") return { ...base, kind: "equals", a: h.rule.values[0] ?? "" };
    const c = h.rule.first;
    switch (c.type) {
      case "text":
        return { ...base, kind: c.op === "equals" ? "equals" : "contains", a: c.text };
      case "number":
        return { ...base, kind: c.op === "lt" || c.op === "le" ? "lt" : "gt", a: String(c.value) };
      case "number_between":
        return { ...base, kind: "between", a: String(c.low), b: String(c.high) };
      case "blank":
        return { ...base, kind: "blank" };
      case "not_blank":
        return { ...base, kind: "not_blank" };
      case "pattern":
        return { ...base, kind: "pattern", a: c.pattern.text };
      default:
        return { ...base, kind: "contains" };
    }
  }
  let list = $state<Row[]>(untrack(() => rules.map(toRow)));
  let problem = $state("");

  function add(): void {
    if (list.length >= MAX_HIGHLIGHTS) return;
    list = [...list, { col, kind: "contains", a: "", b: "", color: list.length % PALETTE.length, whole_row: false }];
  }

  function toRule(r: Row): Rule | string {
    const cond = (first: Condition): Rule => ({ type: "conditions", first, second: null, or: false });
    const num = (s: string) => Number(s.replace(",", "."));
    switch (r.kind) {
      case "contains":
        return r.a ? cond({ type: "text", op: "contains", text: r.a }) : "Type the text to look for.";
      case "equals":
        return cond({ type: "text", op: "equals", text: r.a });
      case "gt":
      case "lt":
        return Number.isFinite(num(r.a)) && r.a.trim() ? cond({ type: "number", op: r.kind, value: num(r.a) }) : "Type a number.";
      case "between":
        return Number.isFinite(num(r.a)) && Number.isFinite(num(r.b)) && r.a.trim() && r.b.trim()
          ? cond({ type: "number_between", low: Math.min(num(r.a), num(r.b)), high: Math.max(num(r.a), num(r.b)) })
          : "Type two numbers.";
      case "blank":
        return cond({ type: "blank" });
      case "not_blank":
        return cond({ type: "not_blank" });
      case "pattern":
        return r.a ? cond({ type: "pattern", pattern: { text: r.a, kind: "regex", match_case: false, whole_cell: false }, negate: false }) : "Type a pattern.";
    }
  }

  function save(): void {
    const out: Highlight[] = [];
    for (const r of list) {
      const rule = toRule(r);
      if (typeof rule === "string") {
        problem = rule;
        return;
      }
      out.push({ col: r.col, rule, color: r.color, whole_row: r.whole_row });
    }
    onSave(out);
  }
</script>

<FormShell title="Highlight rules" wide {onClose} onEnter={save}>
  <p class="note">The first rule that matches a cell colours it. Rules are kept with the view, not in the file.</p>
  <div class="rules">
    {#each list as r, i (i)}
      <div class="rule">
        <select bind:value={r.col} aria-label="Column">{#each titles as t, c (c)}<option value={c}>{t}</option>{/each}</select>
        <select bind:value={r.kind} aria-label="Condition">
          <option value="contains">contains</option>
          <option value="equals">is exactly</option>
          <option value="gt">is more than</option>
          <option value="lt">is less than</option>
          <option value="between">is between</option>
          <option value="blank">is blank</option>
          <option value="not_blank">is not blank</option>
          <option value="pattern">matches the pattern</option>
        </select>
        {#if r.kind !== "blank" && r.kind !== "not_blank"}<input type="text" bind:value={r.a} aria-label="Value" />{/if}
        {#if r.kind === "between"}and <input type="text" bind:value={r.b} aria-label="Second value" />{/if}
        <span class="colors" role="radiogroup" aria-label="Colour">
          {#each PALETTE as p, k (k)}
            <button type="button" class="sw" class:on={r.color === k} style:background={p.color} aria-label={p.name} aria-checked={r.color === k} role="radio" onclick={() => (r.color = k)}></button>
          {/each}
        </span>
        <label><input type="checkbox" bind:checked={r.whole_row} /> Whole row</label>
        <button type="button" aria-label="Remove rule" onclick={() => (list = list.filter((_, k) => k !== i))}>✕</button>
      </div>
    {:else}
      <p class="note">No rules yet.</p>
    {/each}
  </div>
  <button type="button" onclick={add} disabled={list.length >= MAX_HIGHLIGHTS}>Add a rule</button>
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" class="primary" onclick={save}>Apply</button>
    <button type="button" onclick={onClose}>Cancel</button>
  {/snippet}
</FormShell>

<style>
  .rules {
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 24rem;
    overflow: auto;
    margin-bottom: 6px;
  }
  .rule {
    display: flex;
    gap: 6px;
    align-items: center;
    flex-wrap: wrap;
  }
  .rule input[type="text"] {
    width: 8rem;
  }
  .colors {
    display: inline-flex;
    gap: 2px;
  }
  .sw {
    width: 16px;
    height: 16px;
    border-radius: 3px;
    border: 2px solid transparent;
    padding: 0;
  }
  .sw.on {
    border-color: var(--fg);
  }
  .note {
    color: var(--muted);
    font-size: 0.85rem;
  }
  .problem {
    color: var(--danger, #c00);
  }
</style>
