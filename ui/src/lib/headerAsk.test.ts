import { describe, expect, it } from "vitest";
import { headerCertainty } from "./header";

describe("how sure the title-row guess is", () => {
  it("is sure of titles over numbers, or of column words", () => {
    expect(headerCertainty([["id", "name", "amount"], ["1", "Ada", "3.50"], ["2", "Bo", "1.25"]])).toBe("yes");
    expect(headerCertainty([["first_name", "LastName", "City"], ["Ada", "Stone", "Knoxville"], ["Bo", "Wells", "Dayton"]])).toBe("yes");
  });
  it("is sure the first row is data when it has numbers or dates", () => {
    expect(headerCertainty([["1", "Ada", "3.50"], ["2", "Bo", "1.25"]])).toBe("no");
    expect(headerCertainty([["Ada", "2021-03-04"], ["Bo", "2020-01-02"]])).toBe("no");
  });
  it("is unsure of a list of names", () => {
    expect(headerCertainty([["Ada", "Stone"], ["Bo", "Wells"], ["Cy", "Varga"]])).toBe("unsure");
    expect(headerCertainty([["only"]])).toBe("unsure");
    // Surnames written like column names are not titles.
    expect(headerCertainty([["McDonald", "DeLuca"], ["MacArthur", "LeBlanc"], ["Stone", "Wells"]])).toBe("unsure");
    expect(headerCertainty([["iPhone", "eBay"], ["Galaxy", "Etsy"]])).toBe("unsure");
  });
  it("is not sure a row with years is data", () => {
    expect(headerCertainty([["Name", "2023", "2024"], ["Ada", "5", "6"], ["Bo", "7", "8"]])).toBe("yes");
    expect(headerCertainty([["Ada", "1990", "2001"], ["Bo", "1985", "1999"]])).toBe("unsure");
    expect(headerCertainty([["Ada", "2023", "17"], ["Bo", "2024", "3"]])).toBe("no");
  });
});
