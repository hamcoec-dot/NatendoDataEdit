import { describe, expect, it } from "vitest";
import { MockBackend } from "./mock";
import fixture from "../../../crates/engine/src/testdata/find_blocks.json";

// The cases crates/engine/src/find.rs also runs (find_over_blocks_gives_the_cells_in_the_shared_fixture):
// the browser stand-in must find the same cells as the real engine.
describe("Find over several blocks: the browser stand-in finds what the engine finds", () => {
  for (const c of fixture.cases) {
    it(c.name, async () => {
      const m = new MockBackend();
      const d = await m.newDocument();
      await m.dataOp(d.id, {
        op: "paste",
        spec: { row: 0, col: 0, op: "none", skip_blanks: false, transpose: false },
        text: fixture.grid.map((r) => r.join("\t")).join("\r\n"),
      });
      const spec = {
        pattern: { text: c.pattern, kind: "plain" as const, match_case: false, whole_cell: false },
        scope: { kind: "areas" as const, areas: c.areas },
        skip_first: c.skip_first,
        skip_cols: c.skip_cols,
      };
      const all = await m.findAll(d.id, spec);
      expect(all.hits.map((h) => [h.row, h.col])).toEqual(c.hits);
      expect(all.cells).toBe(c.hits.length);
      expect((await m.findCount(d.id, spec, null)).cells).toBe(c.hits.length);
    });
  }
});
