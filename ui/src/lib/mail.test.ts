import { describe, expect, it } from "vitest";
import { fillTemplate, suggestTemplate, unknownFields } from "./mail";
import { runStep } from "./clean";

describe("mailing labels and letters (M14)", () => {
  const titles = ["First Name", "Last Name", "Address", "Address 2", "City", "State", "Zip"];

  it("suggests a template from the titles", () => {
    expect(suggestTemplate(titles)).toBe("{First Name} {Last Name}\n{Address}\n{Address 2}\n{City}, {State} {Zip}");
  });

  it("finds fields that are not columns", () => {
    expect(unknownFields("{City} {nope} {{x}} {3}", titles)).toEqual(["nope"]);
  });

  it("fills a row and leaves out blank lines", () => {
    const row = ["Ann", "Lee", "1 Main St", "", "Reno", "", "89501"];
    const t = "{First Name} {Last Name}\n{Address}\n{Address 2}\n{City}, {State} {Zip}";
    expect(fillTemplate(t, titles, row, true)).toEqual(["Ann Lee", "1 Main St", "Reno, 89501"]);
    expect(fillTemplate(t, titles, row, false)).toHaveLength(4);
  });
});

describe("address tidy steps (M14, browser stand-in)", () => {
  it("tidies ZIP codes, phones and e-mail", () => {
    expect(runStep({ tool: "zip", style: "keep" }, "2134", 0)).toBe("02134");
    expect(runStep({ tool: "zip", style: "five" }, "37402-1234", 0)).toBe("37402");
    expect(runStep({ tool: "phone", style: "brackets" }, "423.555.0123", 0)).toBe("(423) 555-0123");
    expect(runStep({ tool: "email", fix_domains: true }, " Ann@GMIAL.com ", 0)).toBe("ann@gmail.com");
    expect(runStep({ tool: "address", style: "short" }, "1 Main Street", 0)).toBe("1 Main St");
  });
});
