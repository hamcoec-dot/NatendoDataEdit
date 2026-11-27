import { describe, expect, it } from "vitest";
import { focusPick, type Control } from "./focusPick";

const box = (disabled = false): Control => ({ tag: "input", disabled, enter: false });
const btn = (enter = false, disabled = false): Control => ({ tag: "button", disabled, enter });

describe("where a dialog puts the focus", () => {
  it("the first box that can take it", () => {
    expect(focusPick([btn(), box(true), box(), btn(true)])).toBe(2);
    expect(focusPick([{ tag: "select", disabled: false, enter: false }, box()])).toBe(0);
  });
  it("never a later button only because the boxes before it are greyed", () => {
    // A screen still working: its boxes greyed, then Apply (greyed), Delete, Cancel.
    expect(focusPick([box(true), btn(false, true), btn(), btn()])).toBe(-1);
  });
  it("the main button when there is no box", () => {
    expect(focusPick([btn(), btn(true), btn()])).toBe(1);
    expect(focusPick([btn(), btn(true, true)])).toBe(-1);
  });
  it("never a hidden input or one not drawn", () => {
    const hidden: Control = { tag: "input", disabled: false, enter: false, hidden: true };
    expect(focusPick([hidden, box()])).toBe(1);
    expect(focusPick([hidden, { ...btn(true), hidden: true }])).toBe(-1);
  });
  it("else the dialog itself", () => {
    expect(focusPick([])).toBe(-1);
  });
});
