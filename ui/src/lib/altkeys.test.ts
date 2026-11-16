import { describe, expect, it } from "vitest";
import { keyOf, watchAlt } from "./commands";

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
const key = (k: string, at: number, alt = false) =>
  ({ key: k, altKey: alt, ctrlKey: false, metaKey: false, shiftKey: false, timeStamp: at }) as KeyboardEvent;

describe("Alt held while the letter arrives without it", () => {
  it("reads Alt from Alt's own key events, for a short while", () => {
    const w = fakeWindow();
    const stop = watchAlt(w.target);
    w.fire("keydown", key("Alt", 1000, true));
    expect(keyOf(key("q", 1050))).toBe("Alt+Q");
    // Too long after the last sign of Alt: not trusted.
    expect(keyOf(key("q", 4000))).toBe("Q");
    // A letter that does say Alt keeps it fresh (Alt held, Q pressed twice).
    w.fire("keydown", key("q", 4100, true));
    expect(keyOf(key("q", 5000))).toBe("Alt+Q");
    // Released: plain letters again.
    w.fire("keyup", key("Alt", 5100));
    expect(keyOf(key("q", 5150))).toBe("Q");
    // Leaving the window forgets it too.
    w.fire("keydown", key("Alt", 6000, true));
    w.fire("blur", key("", 6010));
    expect(keyOf(key("q", 6050))).toBe("Q");
    stop();
    expect(w.on.size).toBe(0);
  });
});
