<script lang="ts">
  // "Write a processing receipt" (off unless ticked): a small file beside each file written,
  // with the names, sizes and SHA-256 of the files read and written, the time, and the kinds
  // of steps run. Full paths and full step settings are separate ticks, off by default.
  import type { ReceiptAsk } from "./backend";

  interface Props {
    /** The choice (null: no receipt). */
    value: ReceiptAsk | null;
    onChange: (v: ReceiptAsk | null) => void;
    /** Recipe runs can keep the steps' settings too; a save has no steps. */
    steps?: boolean;
    /** The file is private: the receipt keeps file names only. */
    privateFile?: boolean;
    disabled?: boolean;
  }
  let { value, onChange, steps = false, privateFile = false, disabled = false }: Props = $props();
  const set = (p: Partial<ReceiptAsk>): void => onChange({ full_paths: value?.full_paths ?? false, full_steps: value?.full_steps ?? false, ...p });
</script>

<div class="receipt">
  <label class="row" title="Writes name.ndreceipt.json beside the file: file names, sizes and SHA-256 hashes, times, the tool, and the kinds of steps. No cell values. It shows tampering, but it is not a signature.">
    <input type="checkbox" checked={value !== null} {disabled} onchange={(e) => onChange(e.currentTarget.checked ? { full_paths: false, full_steps: false } : null)} />
    Write a processing receipt beside {steps ? "each result" : "the file"}
  </label>
  {#if value}
    {#if privateFile}
      <p class="sub">This file is private: the receipt keeps file names only{steps ? ", without column titles or the steps' hash" : ""}.</p>
    {:else}
      <label class="row sub"><input type="checkbox" checked={value.full_paths} {disabled} onchange={(e) => set({ full_paths: e.currentTarget.checked })} /> Include full folder paths (a path can name a person)</label>
      {#if steps}
        <label class="row sub"><input type="checkbox" checked={value.full_steps} {disabled} onchange={(e) => set({ full_steps: e.currentTarget.checked })} /> Include the steps' settings (they can hold data, such as a replaced name)</label>
      {/if}
    {/if}
  {/if}
</div>

<style>
  .receipt {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .sub {
    color: var(--muted);
    margin-left: 1.6em;
  }
  p.sub {
    margin: 0 0 0 1.6em;
  }
</style>
