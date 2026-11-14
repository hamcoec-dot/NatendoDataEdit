import { describe, expect, it } from "vitest";
import { confidenceText, describeLayout, parseSeparator, separatorName, visible } from "./layout";
import { MockBackend } from "./mock";

const base = { delimiter: ",", quote: '"', escape: null, merge: false, skip_lines: 0 };

describe("layout words", () => {
  it("names separators plainly", () => {
    expect(separatorName({ delimiter: "\t", merge: false })).toBe("Tab");
    expect(separatorName({ delimiter: " ", merge: true })).toBe("Spaces (lined-up columns)");
    expect(separatorName({ delimiter: "~|~", merge: false })).toBe("“~|~”");
    expect(separatorName({ delimiter: "\x1f", merge: false })).toBe("Unit separator (0x1F)");
    expect(visible("a\x02\tb")).toBe("a\\x02\\tb");
  });

  it("describes a whole layout", () => {
    expect(describeLayout(base, 5)).toBe("Comma, double quotes (5 columns)");
    expect(describeLayout({ ...base, delimiter: ";", quote: null, skip_lines: 2 })).toBe(
      "Semicolon, no quotes, 2 lines above the table set aside",
    );
    expect(describeLayout({ ...base, escape: "\\" })).toBe("Comma, double quotes, backslash escapes");
    expect(describeLayout({ ...base, skip_lines: 1 }, 1)).toBe(
      "One column (no separator), 1 line above the table set aside",
    );
  });

  it("reads typed separators", () => {
    expect(parseSeparator("\\t")).toBe("\t");
    expect(parseSeparator("\\x1F")).toBe("\x1f");
    expect(parseSeparator("||")).toBe("||");
  });

  it("puts confidence in words", () => {
    expect(confidenceText(0.95)).toBe("very likely");
    expect(confidenceText(0.2)).toBe("a rough guess");
  });

  it("stub backend offers choices and reopens with one", async () => {
    const b = new MockBackend();
    const d = (await b.openFile())!.doc!;
    const c = await b.layoutChoices(d.id);
    expect(c.choices[0]!.preview[0]![0]).toBe("id");
    const again = await b.reopen(d.id, false, { ...base, delimiter: ";", skip_lines: 1 });
    expect([again.delimiter, again.preamble_lines, again.layout_confidence]).toEqual([";", 1, null]);
    await expect(b.previewLayout(d.id, { ...base, delimiter: "" })).rejects.toMatchObject({ kind: "bad_layout" });
  });
});
