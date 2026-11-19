import { describe, expect, it } from "vitest";
import { namesFromText } from "./columnNames";

describe("paste as column names", () => {
  it("reads a copied row split by tabs (only the first line)", () => {
    expect(namesFromText("id\tfirst_name\tcity\r\n1\tAda\tDayton\r\n", ",")).toEqual(["id", "first_name", "city"]);
  });
  it("reads a line of the file's delimiter, or commas, with quotes", () => {
    expect(namesFromText('id;"last; first";city', ";")).toEqual(["id", "last; first", "city"]);
    expect(namesFromText('id,"say ""hi""",notes\n', "|")).toEqual(["id", 'say "hi"', "notes"]);
  });
  it("keeps a line break inside quotes", () => {
    expect(namesFromText('"a\nb"\tc\r\nx\ty', ",")).toEqual(["a\nb", "c"]);
  });
  it("reads a list, one name per line", () => {
    expect(namesFromText("id\r\nname\r\ncity\r\n", ",")).toEqual(["id", "name", "city"]);
    expect(namesFromText("only", ",")).toEqual(["only"]);
    expect(namesFromText("", ",")).toEqual([]);
  });
});
