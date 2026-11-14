<script lang="ts">
  // Quick totals of the selection in the status bar (Sum, Average, Count,
  // ...). The engine works them out off the UI thread; a newer selection or
  // any change cancels older requests, so a huge selection never blocks.
  import type { Tab } from "./tab.svelte";
  import { backend, type Totals } from "./backend";
  import { prefs, TOTAL_KINDS, type TotalKind } from "./prefs.svelte";
  import { fmt } from "./views";
  import { totalLabel, totalText, totalValue } from "./m9";

  let { tab }: { tab: Tab } = $props();

  let totals = $state<Totals | null>(null);
  let pending = $state(false);
  let open = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let asked = 0;

  const READ = { decimal_comma: false, date_order: "mdy" as const };

  $effect(() => {
    const t = tab;
    const r = t.rect;
    void t.version;
    void t.info;
    const distinct = prefs.data.totals === "distinct";
    clearTimeout(timer);
    const n = ++asked;
    // Never show the last selection's totals for this one.
    totals = null;
    open = false;
    // One cell: nothing to total (as in Sheets).
    if (r.rows * r.cols <= 1 || !t.info.indexed) {
      totals = null;
      pending = false;
      return;
    }
    // A short pause so dragging a selection does not ask on every step.
    timer = setTimeout(() => {
      const slow = setTimeout(() => {
        if (n === asked) pending = true;
      }, 250);
      void backend()
        .selectionTotals(t.id, r.row, r.rows, r.col, Math.min(r.cols, 16_384), READ, distinct)
        .then((x) => {
          if (n === asked) totals = x;
        })
        .catch(() => {
          // Cancelled by a newer selection, or the file was busy: the next
          // change asks again.
          if (n === asked) totals = null;
        })
        .finally(() => {
          clearTimeout(slow);
          if (n === asked) pending = false;
        });
    }, 120);
    return () => clearTimeout(timer);
  });

  let button = $state<HTMLButtonElement>();
  /** Where the list opens (fixed, as the status bar clips what overflows). */
  let at = $state({ right: 0, bottom: 0 });
  function toggle(): void {
    if (!open && button) {
      const r = button.getBoundingClientRect();
      at = { right: Math.max(4, window.innerWidth - r.right), bottom: window.innerHeight - r.top + 4 };
    }
    open = !open;
  }
  $effect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!(e.target as HTMLElement).closest(".qt-wrap")) open = false;
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  });

  /** With no numbers selected, a sum or average says nothing: show the count. */
  function shownKind(t: Totals, k: TotalKind): TotalKind {
    return t.numbers === 0 && k !== "count" && k !== "numbers" && k !== "distinct" && k !== "chars" ? "count" : k;
  }

  /** Put the shown total on the clipboard. */
  function copy(): void {
    if (!totals) return;
    const k = shownKind(totals, prefs.data.totals);
    const v = totalValue(totals, k);
    void navigator.clipboard?.writeText(v === null ? "" : String(v)).catch(() => {});
    open = false;
  }

  function pick(k: TotalKind): void {
    prefs.data.totals = k;
    prefs.save();
    open = false;
  }
</script>

{#if totals || pending}
  <span class="wrap qt-wrap">
    <button
      bind:this={button}
      type="button"
      class="small totals"
      title="Totals of the selected cells. Click to choose which one shows."
      aria-haspopup="menu"
      aria-expanded={open}
      onclick={toggle}
    >
      {#if totals}{@const k = shownKind(totals, prefs.data.totals)}{totalLabel(k)}: {totalText(totals, k, fmt)}{:else}Adding up…{/if}
    </button>
    {#if open && totals}
      <div class="pop" role="menu" style:right="{at.right}px" style:bottom="{at.bottom}px">
        {#each TOTAL_KINDS as k (k)}
          <button type="button" role="menuitemradio" aria-checked={prefs.data.totals === k} class:on={prefs.data.totals === k} onclick={() => pick(k)}>
            <span>{totalLabel(k)}</span><span class="v">{totalText(totals, k, fmt)}</span>
          </button>
        {/each}
        <button type="button" role="menuitem" onclick={copy}><span>Copy the total</span></button>
      </div>
    {/if}
  </span>
{/if}

<style>
  .wrap {
    position: relative;
  }
  .totals {
    font-variant-numeric: tabular-nums;
  }
  .pop {
    position: fixed;
    z-index: 30;
    min-width: 220px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    box-shadow: 0 6px 20px rgb(0 0 0 / 0.18);
    padding: 4px;
  }
  .pop button {
    display: flex;
    width: 100%;
    justify-content: space-between;
    gap: 16px;
    border: 0;
    background: none;
    color: var(--text);
    padding: 4px 8px;
    border-radius: 4px;
    text-align: left;
    font-variant-numeric: tabular-nums;
  }
  .pop button:hover,
  .pop button.on {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
  }
  .v {
    color: var(--muted);
  }
</style>
