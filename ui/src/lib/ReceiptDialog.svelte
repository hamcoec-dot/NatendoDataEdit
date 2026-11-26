<script lang="ts">
  // Tools > Verify a receipt: pick a receipt, and optionally the files that went in and came
  // out, and see whether each still matches. Only the files picked here are read; paths
  // written inside a receipt are never opened.
  import { backend, errorText, type ReceiptReport } from "./backend";
  import FormShell from "./FormShell.svelte";

  interface Props {
    onClose: () => void;
  }
  let { onClose }: Props = $props();

  let withFiles = $state(true);
  let chain = $state(true);
  let report = $state<ReceiptReport | null>(null);
  let problem = $state("");
  let busy = $state(false);

  async function check(): Promise<void> {
    busy = true;
    problem = "";
    try {
      const r = await backend().receiptVerify(withFiles, chain && withFiles);
      if (r) report = r;
    } catch (e) {
      report = null;
      problem = errorText(e);
    } finally {
      busy = false;
    }
  }
  const list = (v: unknown): string => (Array.isArray(v) ? v.map(String).join(", ") : "");
</script>

<FormShell title="Verify a receipt" {onClose} onEnter={() => void check()}>
  <p class="note">A receipt (name.ndreceipt.json) records the files a run read and wrote, with their SHA-256 hashes. Checking it shows whether the receipt and the files are still as they were. It shows tampering, but it is not a signature: someone who can edit the receipt can also redo its hashes.</p>
  <label class="row"><input type="checkbox" bind:checked={withFiles} /> Then pick the files that went in and came out, to compare</label>
  <label class="row"><input type="checkbox" bind:checked={chain} disabled={!withFiles} /> Also check the receipt beside each file that went in (the chain)</label>
  {#if report}
    <p class={report.ok ? "good" : "problem"}>{report.verdict}</p>
    <ul class="lines" aria-label="Checks">
      {#each report.lines as l, i (i)}
        <li class={l.ok ? "good" : "problem"}><strong>{l.ok ? "OK" : "Not OK"}</strong> {l.what}: {l.why}</li>
      {/each}
    </ul>
    <p class="sub">
      {String(report.summary.action ?? "")} with {String(report.summary.tool ?? "")}, {String(report.summary.started ?? "")} to {String(report.summary.finished ?? "")} (UTC).
      In: {list(report.summary.inputs) || "none"}. Out: {list(report.summary.outputs) || "none"}.{list(report.summary.steps) ? ` Steps: ${list(report.summary.steps)}.` : ""}
      {report.summary.input_changed_during_run ? " An input changed while the run read it, so no hash is claimed for it." : ""}
    </p>
  {/if}
  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {#snippet buttons()}
    <button type="button" class="primary" onclick={() => void check()} disabled={busy}>{busy ? "Checking…" : "Choose a receipt…"}</button>
    <button type="button" onclick={onClose}>Close</button>
  {/snippet}
</FormShell>

<style>
  .lines {
    margin: 4px 0;
    padding-left: 1.2em;
  }
  .sub {
    color: var(--muted);
  }
</style>
