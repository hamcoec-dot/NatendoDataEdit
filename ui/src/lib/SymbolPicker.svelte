<script lang="ts" module>
  /** Characters by kind. Spaces and invisible marks carry names, since they
   *  cannot be seen. */
  export const GROUPS: { name: string; chars: string[] }[] = [
    { name: "Punctuation", chars: [..."–—…•·‘’“”«»‹›¡¿§¶†‡′″©®™°¦¨¬¯´¸"] },
    { name: "Currency", chars: [..."$¢£¤¥€₹₩₽₺₪₫₦₱₴₸₿ƒ"] },
    { name: "Maths", chars: [..."±×÷−≠≈≡≤≥∞√∛∑∏∫∂∆∇∈∉∩∪⊂⊃⊆⊇∧∨∀∃∅∝∠⊥‰‱¼½¾⅓⅔⅛¹²³⁴ⁿ₀₁₂₃µπ"] },
    { name: "Arrows", chars: [..."←↑→↓↔↕⇐⇑⇒⇓⇔↖↗↘↙↩↪↺↻⟵⟶"] },
    {
      name: "Latin letters",
      chars: [..."ÀÁÂÃÄÅÆÇÈÉÊËÌÍÎÏÐÑÒÓÔÕÖØÙÚÛÜÝÞßàáâãäåæçèéêëìíîïðñòóôõöøùúûüýþÿŒœŠšŽžŁłŚśŹźŻżĄąĘęČčŘřĞğİıŞşŐőŰűĀāĒēĪīŌōŪū"],
    },
    { name: "Greek", chars: [..."ΑΒΓΔΕΖΗΘΙΚΛΜΝΞΟΠΡΣΤΥΦΧΨΩαβγδεζηθικλμνξοπρσςτυφχψω"] },
    { name: "Shapes and marks", chars: [..."✓✔✗✘★☆●○◉■□▪▫▲△▼▽◆◇♠♣♥♦♪♫☐☑☒☀☁☂☎✉✂⚠"] },
    { name: "Box drawing", chars: [..."─│┌┐└┘├┤┬┴┼═║╔╗╚╝╠╣╦╩╬░▒▓█▀▄"] },
    { name: "Spaces and hidden marks", chars: [" ", " ", " ", " ", "​", "­", "\t", "‑"] },
  ];
  export const NAMES: Record<string, string> = {
    " ": "Non-breaking space",
    " ": "Thin space",
    " ": "En space",
    " ": "Em space",
    "​": "Zero-width space",
    "­": "Soft hyphen",
    "\t": "Tab",
    "‑": "Non-breaking hyphen",
  };
  /** "U+00E9" for a character. */
  export function codeOf(ch: string): string {
    return `U+${(ch.codePointAt(0) ?? 0).toString(16).toUpperCase().padStart(4, "0")}`;
  }
  /** The character for a typed code ("e9", "U+00E9", "0x1F600"), or null. */
  export function fromCode(s: string): string | null {
    const m = /^(?:u\+|0x|\\u)?([0-9a-f]{1,6})$/i.exec(s.trim());
    if (!m) return null;
    const n = parseInt(m[1]!, 16);
    if (n > 0x10ffff || (n >= 0xd800 && n <= 0xdfff) || n < 0x09 || (n > 0x0d && n < 0x20) || n === 0x7f) return null;
    return String.fromCodePoint(n);
  }
</script>

<script lang="ts">
  // Insert > Symbol: pick a character by kind, by its code, or from the
  // ones picked recently. Inserted at the cell editor's cursor.
  interface Props {
    onPick: (ch: string) => void;
    onClose: () => void;
  }
  let { onPick, onClose }: Props = $props();

  const KEY = "nde.recentSymbols";
  function loadRecent(): string[] {
    try {
      const v = JSON.parse(localStorage.getItem(KEY) ?? "[]");
      return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string" && [...x].length === 1).slice(0, 24) : [];
    } catch {
      return [];
    }
  }
  let recent = $state<string[]>(loadRecent());
  let group = $state(0);
  let code = $state("");
  let box: HTMLDivElement;
  const typed = $derived(fromCode(code));

  function pick(ch: string): void {
    const next = [ch, ...recent.filter((x) => x !== ch)].slice(0, 24);
    try {
      localStorage.setItem(KEY, JSON.stringify(next));
    } catch {
      // Recent picks are a convenience only.
    }
    onPick(ch);
  }

  function label(ch: string): string {
    return NAMES[ch] ?? ch;
  }

  function onKey(e: KeyboardEvent): void {
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "Enter" && e.target instanceof HTMLInputElement && typed) {
      e.preventDefault();
      pick(typed);
    }
  }

  $effect(() => {
    box.querySelector<HTMLElement>("button.ch")?.focus();
  });
</script>

{#snippet chars(list: string[])}
  <div class="chars">
    {#each list as ch (ch)}
      <button type="button" class="ch" class:named={!!NAMES[ch]} title="{NAMES[ch] ?? ch} ({codeOf(ch)})" onclick={() => pick(ch)}>{NAMES[ch] ? label(ch) : ch}</button>
    {/each}
  </div>
{/snippet}

<div class="backdrop" role="presentation">
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="sym-title" tabindex="-1" bind:this={box} onkeydown={onKey}>
    <h2 id="sym-title">Insert a symbol</h2>
    {#if recent.length}
      <h3>Recently used</h3>
      {@render chars(recent)}
    {/if}
    <div class="tabs" role="tablist">
      {#each GROUPS as g, i (g.name)}
        <button type="button" role="tab" aria-selected={group === i} class:on={group === i} onclick={() => (group = i)}>{g.name}</button>
      {/each}
    </div>
    {@render chars(GROUPS[group]!.chars)}
    <label class="code">Or type a character code
      <input type="text" bind:value={code} placeholder="U+00E9" maxlength="12" spellcheck="false" />
      {#if typed}<button type="button" class="ch" onclick={() => pick(typed)}>{label(typed)}</button>{:else if code.trim()}<span class="muted">not a character that can go in a cell</span>{/if}
    </label>
    <div class="buttons"><button type="button" onclick={onClose}>Close</button></div>
  </div>
</div>

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
    width: min(560px, 100%);
    max-height: 90vh;
    overflow: auto;
    box-shadow: 0 12px 40px rgb(0 0 0 / 0.3);
    outline: none;
    font-size: 13px;
  }
  h2 {
    font-size: 16px;
    margin: 0 0 10px;
  }
  h3 {
    font-size: 12px;
    color: var(--muted);
    margin: 6px 0 4px;
    font-weight: 600;
  }
  .tabs {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin: 10px 0 6px;
  }
  .tabs button.on {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-ink);
  }
  .chars {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(34px, 1fr));
    gap: 4px;
  }
  button {
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 3px 8px;
    cursor: pointer;
  }
  button.ch {
    font-size: 17px;
    padding: 4px 0;
    min-height: 34px;
  }
  button.ch.named {
    font-size: 11px;
    grid-column: span 3;
  }
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .code {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 12px 0 4px;
    flex-wrap: wrap;
  }
  .code input {
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 3px 8px;
    width: 110px;
  }
  .code button.ch {
    min-width: 40px;
  }
  .muted {
    color: var(--muted);
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    margin-top: 10px;
  }
</style>
