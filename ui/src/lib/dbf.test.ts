import { describe, expect, it } from "vitest";
import dialog from "./ImportDialog.svelte?raw";
import { headerChoice, importHeader, previewLine } from "./dbf";

describe("the import screen for a dBase table", () => {
  it("offers no title-row choice: the field names are the titles", () => {
    expect(headerChoice("dbf")).toBe(false);
    expect(headerChoice("json")).toBe(true);
    expect(importHeader("dbf", false)).toBe(true);
    expect(importHeader("delimited", false)).toBe(false);
    expect(dialog).toContain("{#if headerChoice(info.format)}");
    expect(dialog).toContain("importHeader(info.format, header)");
  });
  it("counts records, never the title row", () => {
    expect(previewLine("dbf", 3, "2 records (1 marked as deleted, left out)")).toBe("2 records (1 marked as deleted, left out).");
    expect(previewLine("dbf", 3, undefined)).toBe("Counting the records\u2026");
    expect(previewLine("json", 3, undefined)).toBe("3 rows.");
    expect(previewLine("json", 100, undefined)).toBe("Showing the first 100 rows.");
    expect(dialog).toContain("previewLine(info.format, rows.length, notes?.records)");
  });
});
