import { describe, expect, it } from "vitest";
import { keyOf, keyOfLoose, watchAlt } from "./commands";

type Handler = (e: KeyboardEvent) => void;

/** A stand-in for the window that records the listeners. */
function fakeWindow() {
  const on = new Map<string, Handler>();
  const target = {
    addEventListener: (t: string, f: Handler) => on.set(t, f),
    removeEventListener: (t: string) => on.delete(t),
  } as unknown as Pick<Window, "addEventListener" | "removeEventListener">;
  const fire = (t: string, e: KeyboardEvent) => on.get(t)?.(e);
  return { target, fire, on };
}

/** A key event at a chosen time. */
const key = (k: string, at: number, mods: { alt?: boolean; ctrl?: boolean; altGr?: boolean } = {}) =>
  ({
    key: k,
    altKey: !!mods.alt,
    ctrlKey: !!mods.ctrl,
    metaKey: false,
    shiftKey: false,
    timeStamp: at,
    getModifierState: (m: string) => m === "AltGraph" && !!mods.altGr,
  }) as unknown as KeyboardEvent;

describe("Alt held while the letter arrives without it", () => {
  it("reads Alt from Alt's own key events, briefly, for shortcuts only", () => {
    const w = fakeWindow();
    const stop = watchAlt(w.target);
    w.fire("keydown", key("Alt", 1000, { alt: true }));
    expect(keyOfLoose(key("q", 1050))).toBe("Alt+Q");
    // Typing and recording shortcuts never guess.
    expect(keyOf(key("q", 1050))).toBe("Q");
    // Too long after the last sign of Alt: not trusted.
    expect(keyOfLoose(key("q", 2000))).toBe("Q");
    // A letter that does say Alt keeps it fresh (Alt held, Q pressed twice).
    w.fire("keydown", key("q", 3000, { alt: true }));
    expect(keyOfLoose(key("q", 3300))).toBe("Alt+Q");
    // Released: plain letters again.
    w.fire("keyup", key("Alt", 3400));
    expect(keyOfLoose(key("q", 3450))).toBe("Q");
    // Leaving the window forgets it too.
    w.fire("keydown", key("Alt", 4000, { alt: true }));
    w.fire("blur", key("", 4010));
    expect(keyOfLoose(key("q", 4050))).toBe("Q");
    stop();
    expect(w.on.size).toBe(0);
  });

  it("never takes AltGr (typing @ or the euro sign) for Alt", () => {
    const w = fakeWindow();
    const stop = watchAlt(w.target);
    // AltGr arrives as Ctrl+Alt; its keyup is "AltGraph".
    w.fire("keydown", key("Control", 1000, { ctrl: true }));
    w.fire("keydown", key("Alt", 1001, { ctrl: true, alt: true, altGr: true }));
    w.fire("keydown", key("@", 1100, { ctrl: true, alt: true, altGr: true }));
    w.fire("keyup", key("AltGraph", 1150));
    expect(keyOfLoose(key("e", 1180))).toBe("E");
    // Even if the keyup is lost, AltGr never started the guess.
    w.fire("keydown", key("AltGraph", 2000, { altGr: true }));
    expect(keyOfLoose(key("e", 2050))).toBe("E");
    stop();
  });
});
