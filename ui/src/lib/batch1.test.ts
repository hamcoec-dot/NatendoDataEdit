import { describe, expect, it } from "vitest";
import { invisibleNote, invisibleRuns, invisibleText } from "./invisible";
import { tidyTitle, tidyTitleList } from "./titles";
import { compareCells } from "./colcompare";
import type { TitleSpec } from "./backend";

describe("show invisible characters", () => {
  it("marks spaces, tabs and line breaks faintly and odd spaces in the warning colour", () => {
    expect(invisibleText("a b\tc\r\nd")).toBe("a·b→c⏎d");
    expect(invisibleRuns("plain")).toEqual([{ text: "plain", kind: "text" }]);
    const runs = invisibleRuns("Ann Lee​");
    expect(runs).toEqual([
      { text: "Ann", kind: "text" },
      { text: "°", kind: "odd" },
      { text: "Lee", kind: "text" },
      { text: "¦", kind: "odd" },
    ]);
    expect(invisibleText("x\u0001")).toBe("x¿");
  });

  it("says what can't be seen", () => {
    expect(invisibleNote("Smith  ")).toBe("2 spaces at the end");
    expect(invisibleNote(" Ann  Lee")).toBe("1 space at the start, 1 double space");
    expect(invisibleNote("Ann Lee ")).toBe("2 non-breaking spaces");
    expect(invisibleNote("   ")).toBe("only spaces (3)");
    expect(invisibleNote("a\tb\nc")).toBe("1 tab, 1 line break");
    expect(invisibleNote("plain")).toBe("");
  });
});

describe("tidy column titles (as the engine)", () => {
  const spec = (p: Partial<TitleSpec> = {}): TitleSpec => ({ spaces: true, case: "keep", odd: false, unique: true, ...p });
  it("styles", () => {
    expect(tidyTitle("  First   NAME ", spec())).toBe("First NAME");
    expect(tidyTitle("first  name", spec({ case: "title" }))).toBe("First Name");
    expect(tidyTitle("First Name-2 ", spec({ case: "snake" }))).toBe("first_name_2");
    expect(tidyTitle("Amount ($)#", spec({ odd: true }))).toBe("Amount");
  });
  it("makes repeats unique and names empty titles", () => {
    expect(tidyTitleList(["Phone", "phone", "", "Phone 2", "Phone"], spec())).toEqual(["Phone", "phone 3", "Column 3", "Phone 2", "Phone 4"]);
    expect(tidyTitleList(["Phone", "PHONE", ""], spec({ case: "snake" }))).toEqual(["phone", "phone_2", "column_3"]);
  });
});

describe("compare two cells (as the engine)", () => {
  const loose = { case: false, spaces: false, numbers: true };
  it("compares", () => {
    expect(compareCells("Main St", "main  st ", loose)).toBe("same");
    expect(compareCells("5", "$5.00", loose)).toBe("same");
    expect(compareCells("x", "", loose)).toBe("first");
    expect(compareCells(" ", "y", loose)).toBe("second");
    expect(compareCells("", "  ", loose)).toBe("same");
    expect(compareCells("Main St", "main st", { case: true, spaces: true, numbers: false })).toBe("different");
  });
});
