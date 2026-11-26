import { describe, expect, it } from "vitest";
import type { DiskEvent } from "./backend";
import { diskBanner, diskText, takeDisk, type DiskTab } from "./disk";

const tab = (): DiskTab => ({ diskNote: null, diskIgnored: "", diskMuted: false });
const changed = (token: string): DiskEvent => ({ doc: 1, state: "changed", token, modified: 1000 });
const gone: DiskEvent = { doc: 1, state: "gone", token: "gone", modified: null };
const same: DiskEvent = { doc: 1, state: "same", token: "", modified: null };

describe("the outside-change banner", () => {
  it("shows a change, stays ignored until the file changes again", () => {
    const t = tab();
    takeDisk(t, changed("a"));
    expect(diskBanner(t)?.state).toBe("changed");
    t.diskIgnored = "a";
    takeDisk(t, changed("a"));
    expect(diskBanner(t)).toBeNull();
    takeDisk(t, changed("b"));
    expect(diskBanner(t)?.token).toBe("b");
    takeDisk(t, gone);
    expect(diskBanner(t)?.state).toBe("gone");
  });
  it("goes when the file is as before, and a muted file never warns", () => {
    const t = tab();
    takeDisk(t, gone);
    takeDisk(t, same);
    expect(diskBanner(t)).toBeNull();
    t.diskMuted = true;
    takeDisk(t, changed("c"));
    expect(diskBanner(t)).toBeNull();
  });
  it("says what happened", () => {
    expect(diskText("made up.csv", { state: "changed", token: "a", modified: 5 }, () => "3:42 PM")).toBe("made up.csv changed on disk (another program saved it at 3:42 PM).");
    expect(diskText("made up.csv", { state: "gone", token: "gone", modified: null })).toBe("made up.csv was deleted or moved. Keep working, then use Save As.");
  });
});
