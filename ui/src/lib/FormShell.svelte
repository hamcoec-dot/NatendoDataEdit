<script lang="ts">
  // The frame shared by the Save As options, Export, Import and Join/Append
  // screens: a modal box with a title, the screen's own content and its
  // buttons. Escape closes; keys never reach the grid behind it.
  import type { Snippet } from "svelte";

  interface Props {
    title: string;
    wide?: boolean;
    onClose: () => void;
    /** Enter outside buttons and text boxes runs the main action. */
    onEnter?: () => void;
    children: Snippet;
    buttons: Snippet;
  }
  let { title, wide = false, onClose, onEnter, children, buttons }: Props = $props();
  let box: HTMLDivElement;
  const id = `form-${Math.random().toString(36).slice(2)}`;

  function onKey(e: KeyboardEvent): void {
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (
      e.key === "Enter" &&
      onEnter &&
      !(e.target instanceof HTMLButtonElement) &&
      !(e.target instanceof HTMLTextAreaElement) &&
      !(e.target instanceof HTMLSelectElement)
    ) {
      e.preventDefault();
      onEnter();
    }
  }

  $effect(() => {
    box.querySelector<HTMLElement>("select, input, textarea, button")?.focus();
  });
</script>

<div class="backdrop" role="presentation">
  <div class="shell" class:wide role="dialog" aria-modal="true" aria-labelledby={id} tabindex="-1" bind:this={box} onkeydown={onKey}>
    <h2 {id}>{title}</h2>
    {@render children()}
    <div class="buttons">{@render buttons()}</div>
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
  .shell {
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 18px 20px 14px;
    width: min(600px, 100%);
    max-height: 92vh;
    overflow: auto;
    box-shadow: 0 12px 40px rgb(0 0 0 / 0.3);
    outline: none;
    font-size: 13px;
    box-sizing: border-box;
  }
  .shell.wide {
    width: min(900px, 100%);
  }
  h2 {
    font-size: 16px;
    margin: 0 0 10px;
  }
  .shell :global(.row) {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 6px 0;
    flex-wrap: wrap;
  }
  .shell :global(.block) {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 8px 0;
  }
  .shell :global(.grid2) {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
    gap: 0 16px;
  }
  .shell :global(fieldset) {
    border: 1px solid var(--border);
    border-radius: 6px;
    margin: 8px 0;
    padding: 6px 10px;
    min-width: 0;
  }
  .shell :global(.list) {
    max-height: 160px;
    overflow: auto;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 2px 10px;
  }
  .shell :global(.list label) {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .shell :global(select),
  .shell :global(input[type="text"]),
  .shell :global(input[type="number"]),
  .shell :global(textarea),
  .shell :global(button) {
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 3px 8px;
  }
  .shell :global(select) {
    max-width: 100%;
  }
  .shell :global(textarea) {
    width: 100%;
    box-sizing: border-box;
    font-family: ui-monospace, monospace;
    resize: vertical;
  }
  .shell :global(button) {
    cursor: pointer;
  }
  .shell :global(button:disabled) {
    opacity: 0.5;
    cursor: default;
  }
  .shell :global(button.primary) {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-ink);
    padding: 6px 16px;
  }
  .shell :global(.note) {
    color: var(--muted);
    font-size: 12px;
    margin: 6px 0;
  }
  .shell :global(.sub) {
    flex-basis: 100%;
    margin: -4px 0 2px 24px;
    color: var(--muted);
    font-size: 12px;
  }
  .shell :global(.problem) {
    color: #c0392b;
    margin: 6px 0;
  }
  .shell :global(.good) {
    color: var(--accent);
    font-size: 12px;
    margin: 6px 0;
  }
  .shell :global(pre.preview),
  .shell :global(.table-preview) {
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    max-height: 220px;
    overflow: auto;
    margin: 6px 0;
    font-family: ui-monospace, monospace;
    font-size: 12px;
  }
  .shell :global(pre.preview) {
    padding: 6px 8px;
    white-space: pre;
  }
  .shell :global(.table-preview table) {
    border-collapse: collapse;
  }
  .shell :global(.table-preview td) {
    border: 1px solid var(--border);
    padding: 2px 6px;
    max-width: 240px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: pre;
    vertical-align: top;
  }
  .shell :global(.table-preview tr.head td) {
    font-weight: 600;
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 12px;
    flex-wrap: wrap;
  }
</style>
