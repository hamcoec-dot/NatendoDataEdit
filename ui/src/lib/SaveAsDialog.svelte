<script lang="ts">
  // Save As options: how values are quoted, the qualifier character,
  // separator, encoding, byte-order mark and line breaks. The live preview
  // shows the first rows as they will be written. The file itself is
  // chosen next, in the system's Save dialog.
  import { untrack } from "svelte";
  import type { DocInfo, ReceiptAsk, SaveAsOptions } from "./backend";
  import FormShell from "./FormShell.svelte";
  import ReceiptChoice from "./ReceiptChoice.svelte";
  import { ENCODINGS, QUOTINGS, SEPARATORS, keepsBytes, saveAsDefaults, writeRow } from "./transfer";

  interface Props {
    info: DocInfo;
    /** The first rows, for the preview. */
    sample: string[][];
    onSave: (o: SaveAsOptions, receipt: ReceiptAsk | null) => void;
    onClose: () => void;
    /** The tab is private (its receipt keeps file names only). */
    privateFile?: boolean;
  }
  let { info, sample, onSave, onClose, privateFile = false }: Props = $props();
  let receipt = $state<ReceiptAsk | null>(null);

  const start = untrack(() => saveAsDefaults(info));
  let o = $state<SaveAsOptions>(start);
  let sepChoice = $state(untrack(() => (SEPARATORS.some((s) => s[0] === start.delimiter) ? start.delimiter : "other")));
  let sepOther = $state(untrack(() => (SEPARATORS.some((s) => s[0] === start.delimiter) ? "" : start.delimiter)));
  let qualifier = $state(untrack(() => (start.quote === null ? "none" : start.quote === "'" ? "'" : '"')));
  const encodings = untrack(() =>
    ENCODINGS.some((e) => e[0].toLowerCase() === start.encoding.toLowerCase()) ? ENCODINGS : [[start.encoding, start.encoding] as [string, string], ...ENCODINGS],
  );
  const unicode = $derived(/^utf-(8|16|32)/i.test(o.encoding));

  $effect(() => {
    o.delimiter = sepChoice === "other" ? sepOther : sepChoice;
  });
  $effect(() => {
    o.quote = qualifier === "none" ? null : qualifier;
    if (qualifier === "none" && (o.quoting === "all" || o.quoting === "text")) o.quoting = "needed";
  });

  const problem = $derived.by(() => {
    if (o.delimiter === "") return "Choose a separator.";
    if ([...o.delimiter].some((c) => c.charCodeAt(0) > 126)) return "The separator must be plain characters (letters, digits, punctuation, tab or space).";
    if (o.delimiter.length > 4) return "The separator can be at most 4 characters.";
    if (o.quote !== null && o.delimiter.includes(o.quote)) return "The separator cannot contain the qualifier character.";
    return "";
  });
  const keeps = $derived(keepsBytes(o, info));
  const preview = $derived(sample.slice(0, 6).map((r) => writeRow(r, o).replace(/\t/g, "→")));

  function save(): void {
    if (problem) return;
    onSave({ ...$state.snapshot(o), bom: unicode && o.bom }, receipt);
  }
</script>

<FormShell title="Save As options" {onClose} onEnter={save}>
  <div class="grid2">
    <label class="row">Separator
      <select bind:value={sepChoice}>
        {#each SEPARATORS as [v, label] (v)}<option value={v}>{label}</option>{/each}
        <option value="other">Other…</option>
      </select>
      {#if sepChoice === "other"}<input type="text" bind:value={sepOther} maxlength="4" size="5" aria-label="Separator characters" />{/if}
    </label>
    <label class="row">Text qualifier
      <select bind:value={qualifier}>
        <option value={'"'}>Double quote "</option>
        <option value="'">Single quote '</option>
        <option value="none">None</option>
      </select>
    </label>
  </div>
  <fieldset>
    <legend>Put values in quotes</legend>
    {#each QUOTINGS as [v, label, about] (v)}
      <label class="row" title={about}>
        <input type="radio" name="quoting" value={v} bind:group={o.quoting} disabled={o.quote === null && (v === "all" || v === "text")} />
        {label}<span class="sub">{about}</span>
      </label>
    {/each}
  </fieldset>
  <div class="grid2">
    <label class="row">Encoding
      <select bind:value={o.encoding}>
        {#each encodings as [v, label] (v)}<option value={v}>{label}</option>{/each}
      </select>
    </label>
    <label class="row"><input type="checkbox" bind:checked={o.bom} disabled={!unicode} /> Byte-order mark (BOM)</label>
    <label class="row">Line breaks
      <select bind:value={o.line_break}>
        <option value="CRLF">Windows (CR LF)</option>
        <option value="LF">Unix, macOS (LF)</option>
        <option value="CR">Old Mac (CR)</option>
      </select>
    </label>
    <label class="row"><input type="checkbox" bind:checked={o.convert_line_breaks} /> Use these line breaks on every row</label>
  </div>
  <p class="note">Preview of the first rows as changed rows are written:</p>
  <pre class="preview">{preview.join("\n")}</pre>
  {#if keeps}
    <p class="good">Rows you did not change are saved exactly as they were, byte for byte.</p>
  {:else}
    <p class="note">Every row is written again in the new format. The saved file is read back and checked before it replaces anything.</p>
  {/if}
  <ReceiptChoice value={receipt} onChange={(v) => (receipt = v)} {privateFile} />
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" class="primary" onclick={save} disabled={!!problem}>Choose file and save…</button>
    <button type="button" onclick={onClose}>Cancel</button>
  {/snippet}
</FormShell>
