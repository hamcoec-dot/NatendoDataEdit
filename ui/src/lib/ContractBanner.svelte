<script lang="ts">
  // C9: a banner under the tabs (never a window over the table) saying the file looks like one of the
  // saved file contracts, and what drifted. The check runs after the file is open; it never blocks.
  import type { ContractMatch, DriftReport } from "./backend";
  import { driftSummary } from "./contractText";
  import Icon from "./Icon.svelte";

  interface Props {
    match: ContractMatch;
    /** "checking": the count is running; "stopped": an edit or Cancel stopped it. */
    phase: "checking" | "report" | "stopped" | "error";
    report: DriftReport | null;
    error?: string;
    onReview: () => void;
    onNotThis: () => void;
    onClose: () => void;
    onCancel: () => void;
    onAgain: () => void;
    onPick: (name: string) => void;
  }
  let { match, phase, report, error = "", onReview, onNotThis, onClose, onCancel, onAgain, onPick }: Props = $props();
  let others = $state(false);
</script>

<div class="contractbar" role="status" data-testid="contract-banner">
  <span class="text">
    {#if phase === "checking"}
      Checking against <b>{match.name}</b>…
    {:else if phase === "stopped"}
      Looks like <b>{match.name}</b>. The check stopped (the table changed).
    {:else if phase === "error"}
      Looks like <b>{match.name}</b>, but it could not be checked: {error}
    {:else if report}
      Looks like <b>{match.name}</b>: {driftSummary(report)}.
    {/if}
  </span>
  {#if phase === "checking"}
    <button type="button" onclick={onCancel}>Cancel</button>
  {:else if phase === "stopped" || phase === "error"}
    <button type="button" onclick={onAgain}>Check again</button>
  {:else if report?.drifted}
    <button type="button" class="primary" onclick={onReview}>Review</button>
  {/if}
  {#if match.others.length}
    <span class="others">
      <button type="button" aria-haspopup="menu" aria-expanded={others} title={match.ties.length ? `These fit as well: ${match.ties.join(", ")}` : "Other contracts this file also looks like"} onclick={() => (others = !others)}>Other contracts…</button>
      {#if others}
        <div class="menu" role="menu">
          {#each match.others as name (name)}
            <button type="button" role="menuitem" onclick={() => { others = false; onPick(name); }}>{name}{match.ties.includes(name) ? " (fits as well)" : ""}</button>
          {/each}
        </div>
      {/if}
    </span>
  {/if}
  <button type="button" onclick={onNotThis} title="Do not check this file against {match.name} again while it is open">Not this contract</button>
  <button type="button" class="close" aria-label="Close" title="Close" onclick={onClose}><Icon name="close" size={12} /></button>
</div>

<style>
  .contractbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    background: color-mix(in srgb, var(--accent) 10%, var(--bg));
  }
  .contractbar button {
    padding: 3px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    color: inherit;
    font: inherit;
    cursor: pointer;
  }
  .contractbar button.primary {
    border-color: var(--accent);
  }
  .contractbar button:hover {
    border-color: var(--accent);
  }
  .contractbar button.close {
    padding: 3px 6px;
  }
  .text {
    flex: 1 1 260px;
  }
  .others {
    position: relative;
  }
  .menu {
    position: absolute;
    top: 100%;
    left: 0;
    z-index: 20;
    display: flex;
    flex-direction: column;
    min-width: 200px;
    padding: 4px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    box-shadow: 0 4px 16px rgb(0 0 0 / 0.2);
  }
  .menu button {
    border: none;
    text-align: left;
  }
</style>
