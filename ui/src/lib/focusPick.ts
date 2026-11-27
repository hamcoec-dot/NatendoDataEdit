// Where a dialog puts the focus when it opens (FormShell): the first box that can take it, else
// its main button (one marked data-enter, never a destructive one), else the dialog itself. Never
// a later button picked only because the boxes before it are greyed: Enter would press it.

export interface Control {
  /** "select", "input", "textarea" or "button". */
  tag: string;
  disabled: boolean;
  /** A button marked data-enter. */
  enter: boolean;
  /** Not on screen (an input of type hidden, or not drawn): never focused. */
  hidden?: boolean;
}

/** The index of the control to focus, or -1 for the dialog itself. */
export function focusPick(controls: readonly Control[]): number {
  const box = controls.findIndex((c) => c.tag !== "button" && !c.disabled && !c.hidden);
  if (box >= 0) return box;
  return controls.findIndex((c) => c.tag === "button" && c.enter && !c.disabled && !c.hidden);
}
