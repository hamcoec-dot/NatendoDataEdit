<script lang="ts">
  // "File layout" dialog: the layouts detection found (best first, each
  // with a preview), and a form to set the separator, quotes and lines
  // above the table by hand, with a live preview. Choosing one reopens the
  // file with it; the file itself is never changed by this.
  import { backend, errorText, type Layout, type LayoutChoices } from "./backend";
  import { confidenceText, describeLayout, parseSeparator, sameLayout, separatorName, visible } from "./layout";

  interface Props {
    doc: number;
    /** Changes not saved yet will be lost by reopening. */
    modified: boolean;
    onChoose: (layout: Layout) => void;
    onClose: () => void;
  }
  let { doc, modified, onChoose, onClose }: Props = $props();

  let data = $state<LayoutChoices | null>(null);
  let failure = $state("");
  /** Index into data.choices, or -1 for the hand-made layout. */
  let picked = $state(0);

  // Hand-made layout.
  const PRESETS = [",", "\t", ";", "|", " ", "spaces", "other"] as const;
  let sepKind = $state<string>(",");
  let sepOther = $state("");
  let quote = $state<string>('"');
  let escape = $state(false);
  let skip = $state(0);
  let custom = $state<string[][] | null>(null);
  let customError = $state("");
  let box: HTMLDivElement;

  const customLayout = $derived<Layout>({
    delimiter: sepKind === "spaces" ? " " : sepKind === "other" ? parseSeparator(sepOther) : sepKind,
    quote: quote === "none" ? null : quote,
    escape: escape && quote !== "none" ? "\\" : null,
    merge: sepKind === "spaces",
    skip_lines: Math.max(0, Math.min(1000, Math.floor(Number(skip) || 0))),
  });

  const chosen = $derived<Layout | null>(
    picked >= 0 ? (data?.choices[picked]?.layout ?? null) : customError ? null : customLayout,
  );

  $effect(() => {
    void backend()
      .layoutChoices(doc)
      .then((d) => {
        data = d;
        const at = d.choices.findIndex((c) => sameLayout(c.layout, d.current));
        picked = at >= 0 ? at : -1;
        // Start the form from the current layout.
        const cur = d.current;
        const preset = cur.merge ? "spaces" : PRESETS.includes(cur.delimiter as never) ? cur.delimiter : "other";
        sepKind = preset;
        sepOther = preset === "other" ? visible(cur.delimiter) : "";
        quote = cur.quote ?? "none";
        escape = cur.escape !== null;
        skip = cur.skip_lines;
      })
      .catch((e) => (failure = errorText(e)));
  });

  // Live preview of the hand-made layout.
  let timer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const l = customLayout;
    if (picked !== -1) return;
    clearTimeout(timer);
    timer = setTimeout(() => {
      backend()
        .previewLayout(doc, l)
        .then((rows) => {
          custom = rows;
          customError = "";
        })
        .catch((e) => {
          custom = null;
          customError = errorText(e);
        });
    }, 150);
  });

  $effect(() => {
    box?.querySelector<HTMLElement>("[data-autofocus]")?.focus();
  });

  function onKey(e: KeyboardEvent): void {
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    }
  }

  const sameAsNow = $derived(!!(data && chosen && sameLayout(chosen, data.current)));
</script>

<div class="backdrop" role="presentation">
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="lay-title" bind:this={box} tabindex="-1" onkeydown={onKey}>
    <h2 id="lay-title">File layout</h2>
    {#if failure}
      <p class="error">{failure}</p>
    {:else if !data}
      <p class="muted">Looking at the file…</p>
    {:else}
      <p class="lead">
        {#if data.sep_line}
          The file names its own separator on its first line.
        {:else}
          The layout below is {confidenceText(data.confidence)}{data.confidence < 0.6 ? ", so please check the preview" : ""}.
        {/if}
        Pick another reading if the preview looks wrong. The file is not changed by this.
      </p>

      <div class="choices" role="radiogroup" aria-label="Layouts">
        {#each data.choices as c, i (i)}
          <div class="choice" class:on={picked === i}>
            <label class="head">
              <input type="radio" name="layout" checked={picked === i} onchange={() => (picked = i)} data-autofocus={picked === i ? "" : undefined} />
              <span class="title">
                {describeLayout(c.layout, c.columns)}
                {#if i === 0}<em>best match</em>{/if}
                {#if sameLayout(c.layout, data.current)}<em>now</em>{/if}
              </span>
            </label>
            {#if picked === i}{@render table(c.preview)}{/if}
          </div>
        {/each}

        <div class="choice" class:on={picked === -1}>
          <label class="head">
            <input type="radio" name="layout" checked={picked === -1} onchange={() => (picked = -1)} />
            <span class="title">Set it myself</span>
          </label>
          {#if picked === -1}
            <div class="form">
              <label>Separator
                <select bind:value={sepKind}>
                  {#each PRESETS as p (p)}
                    <option value={p}>{p === "spaces" ? separatorName({ delimiter: " ", merge: true }) : p === "other" ? "Other…" : separatorName({ delimiter: p, merge: false })}</option>
                  {/each}
                </select>
              </label>
              {#if sepKind === "other"}
                <label>Characters
                  <input type="text" maxlength="16" bind:value={sepOther} placeholder="e.g. ~|~ or \x1F" spellcheck="false" />
                </label>
              {/if}
              <label>Quotes
                <select bind:value={quote}>
                  <option value={'"'}>Double quote "</option>
                  <option value="'">Single quote '</option>
                  <option value="none">None</option>
                </select>
              </label>
              <label class="check"><input type="checkbox" bind:checked={escape} disabled={quote === "none"} /> Backslash escapes inside quotes</label>
              <label>Lines above the table
                <input type="number" min="0" max="1000" bind:value={skip} />
              </label>
            </div>
            {#if customError}<p class="error">{customError}</p>{:else if custom}{@render table(custom)}{/if}
          {/if}
        </div>
      </div>

      {#if data.preamble}
        <details class="pre">
          <summary>Lines now set aside above the table</summary>
          <pre>{data.preamble}</pre>
        </details>
      {/if}
      {#if modified}<p class="warn">Reopening drops changes you have not saved.</p>{/if}
    {/if}
    <div class="buttons">
      <button type="button" onclick={onClose}>Cancel</button>
      <button type="button" class="primary" disabled={!chosen || sameAsNow} onclick={() => chosen && onChoose(chosen)}>
        Reopen with this layout
      </button>
    </div>
  </div>
</div>

{#snippet table(rows: string[][])}
  {#if rows.length === 0}
    <p class="muted small">No rows.</p>
  {:else}
    <div class="preview">
      <table>
        <tbody>
          {#each rows as r, ri (ri)}
            <tr>
              {#each r as v, ci (ci)}<td title={v}>{v}</td>{/each}
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
{/snippet}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgb(0 0 0 / 0.35);
    display: grid;
    place-items: center;
    z-index: 50;
    padding: 16px;
  }
  .dialog {
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 18px 20px 14px;
    width: min(760px, 100%);
    max-height: calc(100vh - 32px);
    overflow: auto;
    box-shadow: 0 12px 40px rgb(0 0 0 / 0.3);
    outline: none;
  }
  h2 {
    font-size: 16px;
    margin: 0 0 6px;
  }
  .lead {
    margin: 0 0 10px;
    line-height: 1.45;
  }
  .muted {
    color: var(--muted);
  }
  .small {
    font-size: 12px;
  }
  .error {
    color: #c0392b;
    margin: 6px 0;
  }
  .warn {
    color: #b9770e;
    margin: 8px 0 0;
  }
  .choices {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .choice {
    display: block;
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 8px 10px;
  }
  .choice.on {
    border-color: var(--accent);
  }
  .head {
    display: block;
    cursor: pointer;
  }
  .head > input {
    margin: 0 6px 0 0;
    vertical-align: middle;
  }
  .title em {
    font-style: normal;
    font-size: 11px;
    color: var(--accent);
    border: 1px solid var(--accent);
    border-radius: 9px;
    padding: 0 6px;
    margin-left: 6px;
  }
  .preview {
    margin-top: 8px;
    overflow-x: auto;
    border: 1px solid var(--grid-line);
    border-radius: 4px;
  }
  table {
    border-collapse: collapse;
    font-size: 12px;
  }
  td {
    border: 1px solid var(--grid-line);
    padding: 2px 6px;
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: pre;
  }
  tr:first-child td {
    background: var(--header);
  }
  .form {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 14px;
    margin-top: 8px;
    align-items: end;
  }
  .form label {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: 12px;
    color: var(--muted);
  }
  .form label.check {
    flex-direction: row;
    align-items: center;
    gap: 6px;
  }
  .form input[type="text"],
  .form input[type="number"],
  .form select {
    font: inherit;
    font-size: 13px;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 3px 6px;
  }
  .form input[type="number"] {
    width: 80px;
  }
  .pre {
    margin-top: 10px;
    font-size: 12px;
  }
  .pre pre {
    max-height: 120px;
    overflow: auto;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 6px;
    margin: 6px 0 0;
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 14px;
  }
  button {
    font: inherit;
    padding: 6px 14px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--text);
    cursor: pointer;
  }
  button.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-ink);
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
</style>
