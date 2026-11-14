<script lang="ts" module>
  export interface DialogButton {
    id: string;
    label: string;
    primary?: boolean;
    danger?: boolean;
  }
  export interface DialogRequest {
    title: string;
    message: string;
    detail?: string;
    buttons: DialogButton[];
    /** Button id returned by Escape. */
    cancel: string;
    resolve: (id: string) => void;
  }
</script>

<script lang="ts">
  // Modal question box. Focus starts on the primary button; Escape cancels.
  interface Props {
    request: DialogRequest;
    onDone: () => void;
  }
  let { request, onDone }: Props = $props();
  let box: HTMLDivElement;

  function choose(id: string): void {
    request.resolve(id);
    onDone();
  }

  function onKey(e: KeyboardEvent): void {
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      choose(request.cancel);
    } else if (e.key === "Tab") {
      // Keep focus inside the dialog.
      const items = [...box.querySelectorAll<HTMLButtonElement>("button")];
      const i = items.indexOf(document.activeElement as HTMLButtonElement);
      const next = items[(i + (e.shiftKey ? items.length - 1 : 1)) % items.length];
      e.preventDefault();
      next?.focus();
    }
  }

  $effect(() => {
    const primary = box.querySelector<HTMLButtonElement>("button.primary") ?? box.querySelector("button");
    primary?.focus();
  });
</script>

<div class="backdrop" role="presentation">
  <div class="dialog" role="alertdialog" aria-modal="true" aria-labelledby="dlg-title" bind:this={box} tabindex="-1" onkeydown={onKey}>
    <h2 id="dlg-title">{request.title}</h2>
    <p>{request.message}</p>
    {#if request.detail}<p class="detail">{request.detail}</p>{/if}
    <div class="buttons">
      {#each request.buttons as b (b.id)}
        <button type="button" class:primary={b.primary} class:danger={b.danger} onclick={() => choose(b.id)}>{b.label}</button>
      {/each}
    </div>
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
    padding: 20px 22px 16px;
    max-width: 480px;
    width: 100%;
    box-shadow: 0 12px 40px rgb(0 0 0 / 0.3);
    outline: none;
  }
  h2 {
    font-size: 16px;
    margin: 0 0 8px;
  }
  p {
    margin: 0 0 8px;
    line-height: 1.45;
  }
  .detail {
    color: var(--muted);
    font-size: 13px;
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 14px;
    flex-wrap: wrap;
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
  button.danger {
    border-color: #c0392b;
    color: #c0392b;
  }
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
</style>
