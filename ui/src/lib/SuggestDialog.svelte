<script lang="ts">
  // Clean-up suggestions: what a scan found, each with a one-click fix.
  // Every fix is one undo step; after a fix the list is worked out again.
  import FormShell from "./FormShell.svelte";
  import { describeSuggestion, type Suggestion, type Suggestions } from "./m9";

  interface Props {
    found: Suggestions | null;
    busy: boolean;
    title: (col: number) => string;
    onFix: (s: Suggestion) => void;
    onFixAll: () => void;
    onClose: () => void;
  }
  let { found, busy, title, onFix, onFixAll, onClose }: Props = $props();
</script>

<FormShell title="Clean-up suggestions" wide {onClose}>
  {#if !found}
    <p class="note">Looking through the file…</p>
  {:else if !found.items.length}
    <p>Nothing to suggest: no extra spaces, numbers written as text, mixed date styles, repeated rows or blank rows were found{found.partial ? ` in the first ${found.rows.toLocaleString()} rows` : ""}.</p>
  {:else}
    <p class="note">
      Found by looking at {found.partial ? `the first ${found.rows.toLocaleString()} rows` : `all ${found.rows.toLocaleString()} rows`}.
      Each fix works on every row and can be undone (Ctrl+Z).
    </p>
    <ul class="items">
      {#each found.items as s, i (i)}
        {@const d = describeSuggestion(s, title)}
        <li>
          <div class="text">
            <div class="found">{d.found}</div>
            {#if s.examples.length}
              <div class="ex">For example: {#each s.examples as e, k (k)}{#if k > 0}, {/if}<code>{e.replace(/ /g, "·")}</code>{/each}</div>
            {/if}
            <div class="fix">{d.fix}</div>
          </div>
          <button type="button" disabled={busy} onclick={() => onFix(s)}>Fix</button>
        </li>
      {/each}
    </ul>
  {/if}
  {#snippet buttons()}
    {#if found && found.items.length > 1}<button type="button" class="primary" disabled={busy} onclick={onFixAll}>Fix all</button>{/if}
    <button type="button" onclick={onClose}>Close</button>
  {/snippet}
</FormShell>

<style>
  .note {
    color: var(--muted);
    font-size: 0.85rem;
  }
  .items {
    width: min(44rem, 80vw);
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 55vh;
    overflow-y: auto;
  }
  li {
    display: flex;
    gap: 12px;
    align-items: center;
    padding: 8px 0;
    border-bottom: 1px solid var(--border);
  }
  .text {
    flex: 1;
    min-width: 0;
  }
  .found {
    font-weight: 600;
  }
  .ex {
    font-size: 0.85rem;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .fix {
    font-size: 0.85rem;
  }
  code {
    background: var(--bg);
    padding: 0 3px;
    border-radius: 3px;
  }
</style>
