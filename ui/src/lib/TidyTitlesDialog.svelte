<script lang="ts">
  // Format › Tidy column titles: trim and collapse spaces, one case style,
  // odd characters out, repeated titles made unique. A preview lists every
  // title that changes before anything does; one undo step.
  import { untrack } from "svelte";
  import FormShell from "./FormShell.svelte";
  import { backend, errorText, type TitleSpec } from "./backend";

  interface Props {
    doc: number;
    last: TitleSpec | null;
    onApply: (spec: TitleSpec) => void;
    onClose: () => void;
  }
  let { doc, last, onApply, onClose }: Props = $props();
  let spec = $state<TitleSpec>(untrack(() => last ?? { spaces: true, case: "keep", odd: false, unique: true }));
  let rows = $state<[number, string, string][] | null>(null);
  let problem = $state("");
  let ask = 0;
  $effect(() => {
    const s = $state.snapshot(spec) as TitleSpec;
    const n = ++ask;
    const id = untrack(() => doc);
    const timer = setTimeout(() => {
      backend()
        .tidyTitlesPreview(id, s)
        .then((r) => {
          if (n !== ask) return;
          rows = r;
          problem = "";
        })
        .catch((e: unknown) => {
          if (n === ask) problem = errorText(e);
        });
    }, 150);
    return () => clearTimeout(timer);
  });
  const SHOWN = 200;
  let done = false;
  function run(): void {
    if (done || !rows?.length) return;
    done = true;
    onApply($state.snapshot(spec) as TitleSpec);
  }
  const show = (s: string) => (s === "" ? "(empty)" : s);
</script>

<FormShell title="Tidy column titles" {onClose} onEnter={run}>
  <div class="opts">
    <label><input type="checkbox" bind:checked={spec.spaces} /> Trim spaces at the ends and make runs of spaces one</label>
    <label
      >Case
      <select bind:value={spec.case}>
        <option value="keep">As written</option>
        <option value="title">Title Case</option>
        <option value="snake">lower_with_underscores</option>
        <option value="upper">UPPER CASE</option>
        <option value="lower">lower case</option>
      </select></label
    >
    <label><input type="checkbox" bind:checked={spec.odd} /> Remove odd characters (keep letters, digits, spaces, _ and -)</label>
    <label><input type="checkbox" bind:checked={spec.unique} /> Make repeated titles unique (Phone, Phone 2) and name empty ones (Column 3)</label>
  </div>
  {#if problem}
    <p class="warn" role="alert">{problem}</p>
  {:else if rows === null}
    <p class="muted">Looking at the titles…</p>
  {:else if !rows.length}
    <p class="muted">Every title is tidy already: nothing would change.</p>
  {:else}
    <p class="muted">{rows.length.toLocaleString()} {rows.length === 1 ? "title changes" : "titles change"}. Data check rules that name a column follow it, as with Rename column; Ctrl+Z undoes it all.</p>
    <div class="preview">
      <table>
        <thead><tr><th>Column</th><th>Now</th><th>After</th></tr></thead>
        <tbody>
          {#each rows.slice(0, SHOWN) as [c, a, b] (c)}
            <tr><td>{c + 1}</td><td>{show(a)}</td><td>{show(b)}</td></tr>
          {/each}
        </tbody>
      </table>
      {#if rows.length > SHOWN}<p class="muted">and {(rows.length - SHOWN).toLocaleString()} more.</p>{/if}
    </div>
  {/if}
  {#snippet buttons()}
    <button type="button" onclick={onClose}>Cancel</button>
    <button type="button" class="primary" onclick={run} disabled={!rows?.length}>Tidy {rows?.length ? rows.length.toLocaleString() : ""} {rows?.length === 1 ? "title" : "titles"}</button>
  {/snippet}
</FormShell>

<style>
  .opts {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-bottom: 10px;
  }
  .preview {
    max-height: 280px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 6px;
  }
  table {
    border-collapse: collapse;
    width: 100%;
    font-size: 13px;
  }
  th,
  td {
    text-align: left;
    padding: 3px 8px;
    border-bottom: 1px solid var(--border);
    overflow-wrap: anywhere;
  }
  .muted {
    color: var(--muted);
    font-size: 12px;
  }
  .warn {
    color: var(--danger, #c0392b);
  }
</style>
