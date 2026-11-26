import { describe, expect, it } from "vitest";
import { MockBackend } from "./mock";

describe("processing receipts in the browser preview", () => {
  it("a save with a receipt leaves a note, said once; without one, none", async () => {
    const m = new MockBackend();
    const d = await m.newDocument();
    await m.save(d.id, { saveAs: true, overwriteChanged: false, allowLossy: false, options: null, receipt: null });
    expect(await m.receiptNote(d.id)).toBeNull();
    await m.save(d.id, { saveAs: true, overwriteChanged: false, allowLossy: false, options: null, receipt: { full_paths: false, full_steps: false } });
    expect(await m.receiptNote(d.id)).toMatch(/^Receipt written: .*\.ndreceipt\.json$/);
    expect(await m.receiptNote(d.id)).toBeNull();
  });
  it("verify reports each check", async () => {
    const r = await new MockBackend().receiptVerify(true, true);
    expect(r?.ok).toBe(true);
    expect(r?.lines[0]?.what).toBe("the receipt's seal");
  });
});
