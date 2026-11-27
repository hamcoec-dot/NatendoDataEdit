// UI test: moving rows while rows are hidden or a filter is on (gap #5).
// Rows hidden by hand: Alt+Shift+Down on a block with hidden rows inside
// gathers it, says so, and the hidden rows stay where they are; a custom
// height goes with its row; held Alt+Shift+Down x20 then Undo x20 puts every
// row back; moving to the top or the bottom of the shown rows says the hidden
// rows above or below stayed; dragging row numbers works while rows are
// hidden. A real filter on the stub's 1M-row sample: a shown row moves down
// past hidden rows and Undo puts it back.
// Browser stub, made-up data.
// Run: (cd ui && npx vite --port 5381) then
// NODE_PATH=<folder with playwright> node scripts/ui-move-filtered.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2] && !process.argv[2].startsWith("http") ? process.argv[2] : null;
const url = process.argv.find((a) => a.startsWith("http")) || "http://localhost:5381";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1360, height: 900 }, permissions: ["clipboard-read", "clipboard-write"] });
  const p = await ctx.newPage();
  const errors = [];
  p.on("pageerror", (e) => errors.push(String(e)));
  let failed = 0;
  const ok = (cond, msg) => {
    if (!cond) failed++;
    console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  };
  const shot = (n) => (out ? p.screenshot({ path: path.join(out, n) }) : null);
  const settle = (ms = 250) => p.waitForTimeout(ms);
  const sel = (r, c, r2 = r, c2 = c) =>
    p.evaluate(([a, b2, c3, d]) => {
      window.__nde.grid.select({ row: a, col: b2 }, { row: c3, col: d });
      window.__nde.grid.focus();
    }, [r, c, r2, c2]).then(() => settle(150));
  const run = (id, ms = 400) => p.evaluate((x) => window.__nde.reg.run(x), id).then(() => settle(ms));
  const clip = (t) => p.evaluate((x) => navigator.clipboard.writeText(x), t);
  const J = (x) => JSON.stringify(x);
  const status = async () => ((await p.locator("footer.status").textContent()) || "").replace(/\s+/g, " ");
  /** Column 0 of every row in document order (hidden ones too). */
  const all = () =>
    p.evaluate(() => {
      const be = window.__nde.backend;
      const t = window.__nde.tab;
      const d = be.get(t.id);
      const n = be.totalRows(d);
      const ph = be.physCol(d, 0);
      return Array.from({ length: n }, (_, i) => d.edits.get(`${d.order ? d.order[i] : i}:${ph}`) ?? "");
    });
  /** Column 0 of the shown rows. */
  const shownNames = () =>
    p.evaluate(async () => {
      const t = window.__nde.tab;
      const v = await window.__nde.backend.viewport(t.id, 0, t.info.rows, 0, 1, 50);
      return v.cells;
    });
  const hide = async (r0, r1) => {
    await sel(r0, 0, r1, 0);
    await run("format.hideRows");
  };
  const selection = () => p.evaluate(() => [window.__nde.tab.sel.anchor.row, window.__nde.tab.sel.focus.row]);

  await p.goto(url);
  await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });

  const lines = ["name\tk\tnum"];
  for (let i = 1; i <= 30; i++) lines.push(`Person ${String(i).padStart(2, "0")}\t${i % 3 === 0 ? "x" : "y"}\t${(i * 7) % 31}`);
  await run("file.new");
  await sel(0, 0);
  await clip(lines.join("\r\n") + "\r\n");
  await p.keyboard.press("Control+v");
  await settle(800);
  const yes = p.getByRole("button", { name: /Yes, column names/ });
  if (await yes.count()) await yes.first().click();
  if (!(await p.evaluate(() => window.__nde.tab.header))) await run("format.header");
  await settle(300);
  await p.evaluate(() => window.__nde.grid.setRowsHeight([6], 60));
  await settle(300);
  const start = await all();

  // Hide Persons 04-05 and 10-11. Shown: titles, 01, 02, 03, 06 (shown 4), 07, 08, 09, 12 (shown 8), ...
  await hide(10, 11);
  await hide(4, 5);
  const hidden0 = (await all()).filter((n) => ["Person 04", "Person 05", "Person 10", "Person 11"].includes(n));
  // Persons 03 and 06 (Persons 04-05 hidden between) move down one shown row.
  await sel(3, 0, 4, 2);
  await run("edit.moveRowsDown");
  const a1 = await all();
  ok(J(a1.slice(1, 8)) === J(["Person 01", "Person 02", "Person 04", "Person 05", "Person 07", "Person 03", "Person 06"]), `gathered after Person 07; Persons 04-05 stayed where they were (${J(a1.slice(1, 8))})`);
  const s1 = await status();
  ok(/Moved 2 rows\. 2 hidden rows between them stayed in place\./.test(s1), `the status line says the hidden rows stayed (${s1.slice(0, 120)})`);
  const tip = await p.locator("footer.status .msg").getAttribute("title");
  ok(tip === (await p.locator("footer.status .msg").textContent()), "the whole message is its tooltip, so a cut-off line can be read");
  ok(J(await selection()) === "[4,5]", `the selection follows the rows (${J(await selection())})`);
  const h1 = await p.evaluate(async () => {
    const t = window.__nde.tab;
    const be = window.__nde.backend;
    const out = [];
    for (const [r, px] of t.rowHeights.toView()) {
        const d = be.get(t.id);
        out.push([d.edits.get(`${d.order ? d.order[r] : r}:${be.physCol(d, 0)}`), px]);
      }
    return out;
  });
  ok(J(h1) === J([["Person 06", 60]]), `Person 06's height went with it (${J(h1)})`);
  ok(J((await all()).filter((n) => hidden0.includes(n))) === J(hidden0), "hidden rows keep their order");
  await shot("move-filtered.png");

  // V3: held Alt+Shift+Down x19 more (one shown row each), then Undo x20 puts every row back.
  const before20 = await shownNames();
  for (let k = 0; k < 19; k++) await run("edit.moveRowsDown", 60);
  await settle(300);
  const after20 = await shownNames();
  const at3 = after20.indexOf("Person 03");
  ok(at3 === Math.min(4 + 19, after20.length - 2) && after20[at3 + 1] === "Person 06", `20 presses moved the pair one shown row each (Person 03 now shown row ${at3})`);
  ok(before20.length === after20.length, "no shown row came or went");
  for (let k = 0; k < 20; k++) await run("edit.undo", 60);
  await settle(300);
  const back = await all();
  ok(J(back) === J(start), `Undo x20: every row is back in its place (${J(back.slice(0, 8))})`);

  // Moving to the top of the shown rows: Persons 01-02 hidden above, Person 03 then Person 06 moves up.
  await hide(1, 2);
  // Shown: titles, 07?.. find Person 07 and move it up to the first data row.
  let names = await shownNames();
  const r7 = names.indexOf("Person 07");
  await sel(r7, 0);
  for (let k = r7; k > 1; k--) await run("edit.moveRowsUp", 120);
  names = await shownNames();
  const s2 = await status();
  ok(names[1] === "Person 07", `Person 07 is the first shown row (${J(names.slice(0, 4))})`);
  ok(/Moved 1 row\. It is at the top of the shown rows; 2 hidden rows above stayed in place\./.test(s2), `the status line says the hidden rows above stayed (${s2.slice(0, 140)})`);
  ok(J((await all()).slice(1, 3)) === J(["Person 01", "Person 02"]), "Persons 01-02 are still the first rows of the file");

  // Moving to the bottom: Persons 29-30 hidden below.
  await hide(names.indexOf("Person 29"), names.indexOf("Person 30"));
  names = await shownNames();
  const r8 = names.indexOf("Person 08");
  await sel(r8, 0);
  const last = names.length - 1;
  for (let k = r8; k < last; k++) await run("edit.moveRowsDown", 60);
  await settle(300);
  names = await shownNames();
  const s3 = await status();
  ok(names[names.length - 1] === "Person 08", `Person 08 is the last shown row (${J(names.slice(-3))})`);
  ok(/Moved 1 row\. It is at the bottom of the shown rows; 2 hidden rows below stayed in place\./.test(s3), `the status line says the hidden rows below stayed (${s3.slice(0, 140)})`);
  ok(J((await all()).slice(-2)) === J(["Person 29", "Person 30"]), "Persons 29-30 are still the last rows of the file");

  // Dragging row numbers while rows are hidden: shown row 2 to after shown row 5.
  names = await shownNames();
  const who = names[2];
  // Find each row number's place on screen by clicking down the row numbers.
  const cv = await (await p.$("canvas")).boundingBox();
  const rx = cv.x + 18;
  const ys = {};
  for (let y = cv.y + 30; y < cv.y + 400; y += 4) {
    await p.mouse.click(rx, y);
    const r = await p.evaluate(() => window.__nde.tab.cell.row);
    if (ys[r] === undefined) ys[r] = y + 6;
  }
  await p.mouse.click(rx, ys[2]);
  await settle(200);
  await p.mouse.move(rx, ys[2]);
  await p.mouse.down();
  await p.mouse.move(rx + 8, ys[2] + 8, { steps: 2 });
  await p.mouse.move(rx, ys[6] + 4, { steps: 8 });
  await shot("move-drag.png");
  await p.mouse.up();
  await settle(500);
  const dragged = await shownNames();
  ok(dragged.indexOf(who) > 2, `dragging the row number moved ${who} down (now shown row ${dragged.indexOf(who)})`);
  ok(/Moved 1 row/.test(await status()), `the status line says so (${(await status()).slice(0, 100)})`);

  // ---- A real filter on the 1M-row sample ----
  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => window.__nde.tab && window.__nde.tab.info.indexed && window.__nde.tab.info.rows > 1000, null, { timeout: 30000 });
  await settle(800);
  await p.getByRole("button", { name: "Yes, column names" }).click({ timeout: 1500 }).catch(() => {});
  const spec = { columns: [{ col: 3, rule: { type: "values", values: ["Osaka"], blanks: false } }], keep_first: true };
  const B = await p.evaluate(async (spec) => {
    const t = window.__nde.tab;
    const be = window.__nde.backend;
    t.update(await be.setFilter(t.id, spec), false);
    t.bump();
    return { pos: await be.docRows(t.id, 1, 3), rows: t.info.rows };
  }, spec);
  await sel(1, 0);
  await run("edit.moveRowsDown", 600);
  const B2 = await p.evaluate(async () => window.__nde.backend.docRows(window.__nde.tab.id, 1, 3));
  ok(B2[0] === B.pos[1] - 1 && B2[1] === B.pos[1] && B2[2] === B.pos[2], `filtered: the first Osaka row moved down to straight after the second, the row it passed (${J(B.pos)} -> ${J(B2)})`);
  await run("edit.undo", 600);
  ok(J(await p.evaluate(async () => window.__nde.backend.docRows(window.__nde.tab.id, 1, 3))) === J(B.pos), "Undo puts it back");

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  await b.close();
  console.log(failed ? `${failed} FAILED` : "ALL PASSED");
  process.exit(failed ? 1 : 0);
})();
