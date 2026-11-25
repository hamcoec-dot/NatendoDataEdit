<script lang="ts">
  // Change encoding: a list of encodings; the arrow keys scrub, showing the first rows of the file as the highlighted
  // encoding reads them (a throwaway read: nothing changes until Apply). Apply switches an ASCII-compatible file in
  // place (one undo step) or reopens the file from disk for the others. It also looks for rows whose bytes are in
  // another encoding (a job with progress and Cancel) and offers to show them or fix double-encoded text.
  import { backend, errorText, type DocInfo, type EncodingChoice, type EncodingScan } from "./backend";
  import FormShell from "./FormShell.svelte";

  interface Props {
    doc: number;
    /** The encoding the file is read as now (as the status bar names it). */
    current: string;
    /** The title row (left out of the scan). */
    header: boolean;
    onApply: (c: EncodingChoice, now: EncodingChoice | undefined) => void;
    onShow: (info: DocInfo) => void;
    onFixMojibake: () => void;
    onClose: () => void;
  }
  let { doc, current, header, onApply, onShow, onFixMojibake, onClose }: Props = $props();

  let choices = $state<EncodingChoice[]>([]);
  let at = $state(0);
  let preview = $state<string[][]>([]);
  let problem = $state("");
  let busy = $state(false);
  let seq = 0;
  let scan = $state<EncodingScan | null>(null);

  const now = $derived(choices.find((c) => c.label === current.toLowerCase()));
  const picked = $derived(choices[at]);

  $effect(() => {
    let live = true;
    void backend()
      .encodingChoices()
      .then((list) => {
        if (!live) return;
        choices = list;
        const i = list.findIndex((c) => c.label === current.toLowerCase());
        at = Math.max(0, i);
      })
      .catch((e: unknown) => (problem = errorText(e)));
    return () => {
      live = false;
    };
  });

  // The scrub: one read per pause on an encoding, the newest answer wins.
  $effect(() => {
    const c = picked;
    if (!c) return;
    const mine = ++seq;
    const timer = setTimeout(() => {
      backend()
        .encodingPreview(doc, c.label, 8)
        .then((rows) => {
          if (mine !== seq) return;
          preview = rows;
          problem = "";
        })
        .catch((e: unknown) => {
          if (mine !== seq) return;
          preview = [];
          problem = errorText(e);
        });
    }, 90);
    return () => clearTimeout(timer);
  });

  function key(e: KeyboardEvent): void {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      e.stopPropagation();
      at = Math.max(0, Math.min(choices.length - 1, at + (e.key === "ArrowDown" ? 1 : -1)));
    } else if (e.key === "Enter") {
      e.preventDefault();
      e.stopPropagation();
      apply();
    }
  }

  function apply(): void {
    const c = picked;
    if (!c || c.label === current.toLowerCase()) return;
    onApply(c, now);
  }

  async function look(): Promise<void> {
    busy = true;
    scan = null;
    try {
      scan = await backend().scanEncodings(doc, header);
      problem = "";
    } catch (e) {
      problem = errorText(e);
    }
    busy = false;
  }

  async function show(): Promise<void> {
    busy = true;
    try {
      onShow(await backend().encodingShowRows(doc, header));
    } catch (e) {
      problem = errorText(e);
      busy = false;
    }
  }
</script>

<FormShell title="Change encoding" {onClose} onEnter={apply}>
  <div class="body">
    <p class="note">The file is read as <strong>{current}</strong> now. Use the arrow keys to see the first rows in each encoding; nothing changes until Apply.</p>
    <ul class="list" role="listbox" aria-label="Encodings" aria-activedescendant={picked ? `enc-${picked.label}` : undefined} tabindex="0" onkeydown={key}>
      {#each choices as c, i (c.label)}
        <li id={`enc-${c.label}`} role="option" aria-selected={i === at} class:on={i === at} onclick={() => (at = i)} onkeydown={() => {}}>
          {c.name}{c.label === current.toLowerCase() ? " (now)" : ""}
          {#if !c.direct}<span class="tag">reopens the file</span>{/if}
        </li>
      {/each}
    </ul>
    {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
    {#if preview.length}
      <table class="preview" aria-label="First rows read as the highlighted encoding" data-testid="enc-preview">
        <tbody>
          {#each preview as r, i (i)}
            <tr>{#each r as cell, j (j)}<td>{cell}</td>{/each}</tr>
          {/each}
        </tbody>
      </table>
    {/if}
    <p class="note">Apply changes how the file is read, not how it is saved: Save keeps the bytes as they are, and Save As has its own encoding choice (a Save As to another encoding converts every row).</p>
    <div class="scan">
      <button type="button" disabled={busy} onclick={() => void look()}>Look for rows in another encoding</button>
      {#if scan}
        <p data-testid="enc-scan">
          {scan.looked.toLocaleString()} rows: {scan.ascii.toLocaleString()} plain ASCII, {scan.utf8.toLocaleString()} valid UTF-8 with accents or symbols, {scan.invalid.toLocaleString()} not valid UTF-8{scan.double_encoded ? `, ${scan.double_encoded.toLocaleString()} look double-encoded (like Ã©)` : ""}.
          {#if scan.mixed}This file mixes two encodings: the rows that are not valid UTF-8 are probably Windows-1252.{/if}
        </p>
        {#if scan.rows > 0}
          <button type="button" disabled={busy} onclick={() => void show()}>Show only these {scan.rows.toLocaleString()} rows</button>
          {#if scan.double_encoded > 0}<button type="button" onclick={onFixMojibake}>Fix mojibake…</button>{/if}
        {/if}
      {/if}
    </div>
  </div>
  {#snippet buttons()}
    <button type="button" class="primary" data-enter disabled={!picked || picked.label === current.toLowerCase() || busy} onclick={apply}>{picked && !picked.direct ? "Reopen as this" : "Apply"}</button>
    <button type="button" onclick={onClose}>Close</button>
  {/snippet}
</FormShell>

<style>
  .body {
    min-width: 460px;
    max-height: 70vh;
    overflow: auto;
  }
  .list {
    list-style: none;
    margin: 6px 0;
    padding: 2px;
    border: 1px solid var(--border);
    border-radius: 6px;
    max-height: 190px;
    overflow: auto;
  }
  .list:focus {
    outline: 2px solid var(--accent);
  }
  li {
    padding: 3px 8px;
    cursor: pointer;
    border-radius: 4px;
  }
  li.on {
    background: var(--accent);
    color: var(--accent-ink);
  }
  .tag {
    font-size: 11px;
    opacity: 0.75;
    margin-left: 8px;
  }
  .preview {
    border-collapse: collapse;
    width: 100%;
    margin: 6px 0;
    font-size: 12px;
  }
  .preview td {
    border: 1px solid var(--border);
    padding: 2px 6px;
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .note {
    color: var(--muted);
    font-size: 12px;
  }
  .problem {
    color: var(--danger, #b00020);
  }
  .scan {
    display: flex;
    flex-direction: column;
    gap: 6px;
    align-items: flex-start;
    margin-top: 8px;
  }
</style>
