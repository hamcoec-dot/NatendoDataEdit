<script lang="ts">
  // Quick totals of the selection in the status bar: Sum and Count side by
  // side unless others are ticked in its list (Average, Min, Max ...), as in
  // Excel. The engine works them out off the UI thread; a newer selection or
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
    const distinct = prefs.data.totalsShown.includes("distinct");
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

  /** The totals shown: with no numbers selected a sum, average, min or max
   *  says nothing, so those drop out (and the count shows instead). */
  function shownKinds(t: Totals): TotalKind[] {
    const numeric = (k: TotalKind) => k === "sum" || k === "average" || k === "min" || k === "max";
    const ks = prefs.data.totalsShown.filter((k) => t.numbers > 0 || !numeric(k));
    return ks.length ? ks : ["count"];
  }

  /** Put one total on the clipboard. */
  function copy(k: TotalKind): void {
    if (!totals) return;
    const v = totalValue(totals, k);
    void navigator.clipboard?.writeText(v === null ? "" : String(v)).catch(() => {});
    open = false;
  }

  /** Tick or untick a total (at least one stays). */
  function pick(k: TotalKind): void {
    const on = prefs.data.totalsShown.includes(k);
    const next = on ? prefs.data.totalsShown.filter((x) => x !== k) : TOTAL_KINDS.filter((x) => x === k || prefs.data.totalsShown.includes(x));
    if (!next.length) return;
    prefs.data.totalsShown = next;
    prefs.save();
  }
</script>

{#if totals || pending}
  <span class="wrap qt-wrap">
    <button
      bind:this={button}
      type="button"
      class="small totals"
      title="Totals of the selected cells. Click to choose which ones show, or to copy one."
      aria-haspopup="menu"
      aria-expanded={open}
      onclick={toggle}
    >
      {#if totals}{#each shownKinds(totals) as k, i (k)}{#if i > 0}<span class="gap"></span>{/if}{totalLabel(k)}: {totalText(totals, k, fmt)}{/each}{:else}Adding up…{/if}
    </button>
    {#if open && totals}
      <div class="pop" role="menu" style:right="{at.right}px" style:bottom="{at.bottom}px">
        {#each TOTAL_KINDS as k (k)}
          {@const on = prefs.data.totalsShown.includes(k)}
          <button type="button" role="menuitemcheckbox" aria-checked={on} class:on onclick={() => pick(k)}>
            <span class="tick" aria-hidden="true">{on ? "✓" : ""}</span><span>{totalLabel(k)}</span><span class="v">{totalText(totals, k, fmt)}</span>
          </button>
        {/each}
        <div class="sep"></div>
        {#each shownKinds(totals) as k (k)}
          <button type="button" role="menuitem" onclick={() => copy(k)}><span class="tick"></span><span>Copy the {totalLabel(k).toLowerCase()}</span></button>
        {/each}
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
  .pop button:hover {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
  }
  .pop button > span:nth-child(2) {
    flex: 1;
  }
  .tick {
    width: 1em;
    flex: none;
  }
  .gap {
    display: inline-block;
    width: 14px;
  }
  .sep {
    height: 1px;
    margin: 4px 0;
    background: var(--border);
  }
  .v {
    color: var(--muted);
  }
</style>
