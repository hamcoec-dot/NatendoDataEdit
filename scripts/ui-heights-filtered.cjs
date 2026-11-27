// UI test: custom row heights while rows are hidden or a filter is on (gap #4).
// Part A, rows hidden by hand (the same path as a filter): heights stay on their
// rows and draw on the shown ones; a real drag on a shown row's line, with a
// title row and frozen rows, sets the right document row; Format > Height,
// Undo and Redo; inserting and deleting while rows are hidden keep every height
// on its data; sorting leaves heights by position, as before.
// Part B, a real filter on the stub's 1M-row sample: the shown rows draw their
// heights, a filtered-out one is left out, and a short burst of filter changes
// asks the engine only a few times.
// Browser stub, made-up data.
// Run: (cd ui && npx vite --port 5398) then
// NODE_PATH=<folder with playwright> node scripts/ui-heights-filtered.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2] && !process.argv[2].startsWith("http") ? process.argv[2] : null;
const url = process.argv.find((a) => a.startsWith("http")) || "http://localhost:5398";
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
  const run = (id) => p.evaluate((x) => window.__nde.reg.run(x), id).then(() => settle(500));
  const clip = (t) => p.evaluate((x) => navigator.clipboard.writeText(x), t);
  const J = (x) => JSON.stringify(x);
  /** Heights by document row: [[row, px], ...]. */
  const docHeights = () => p.evaluate(() => window.__nde.tab.rowHeights.toView());
  /** Heights the grid draws (by shown row while rows are hidden). */
  const drawn = () => p.evaluate(() => window.__nde.tab.heightsInForce()?.toView() ?? null);
  /** [name in column 0, px] for each document row with a height (every row shown). */
  const namedHeights = () =>
    p.evaluate(async () => {
      const t = window.__nde.tab;
      const out = [];
      for (const [r, px] of t.rowHeights.toView()) {
        const v = await window.__nde.backend.viewport(t.id, r, 1, 0, 1, 50);
        out.push([v.cells[0], px]);
      }
      return out;
    });
  /** Pixel height of shown row r on screen. */
  const screenH = (r) => p.evaluate((r) => window.__nde.grid.cellPoint({ row: r, col: 0 }).y - window.__nde.grid.cellPoint({ row: r - 1, col: 0 }).y, r);
  const setH = (rows, px) => p.evaluate(([rows, px]) => window.__nde.grid.setRowsHeight(rows, px), [rows, px]).then(() => settle(400));
  const hide = async (r0, r1) => {
    await sel(r0, 0, r1, 0);
    await run("format.hideRows");
  };
  const showAll = async () => {
    await sel(0, 0, 60, 0);
    await run("format.showRows");
  };

  await p.goto(url);
  await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });

  // ---- A. Rows hidden by hand ----
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
  // Person 03 60 px, Person 04 70 px, Person 06 90 px (document rows 3, 4, 6).
  await setH([3], 60);
  await setH([4], 70);
  await setH([6], 90);
  ok(J(await docHeights()) === "[[3,60],[4,70],[6,90]]", `three heights set (${J(await docHeights())})`);
  // Hide Persons 04-05 and 07-08: shown 0-3, then 6 (shown 4), 9 (shown 5), ...
  await hide(7, 8);
  await hide(4, 5);
  const info1 = await p.evaluate(() => ({ rows: window.__nde.tab.info.rows, filtered: window.__nde.tab.info.filtered }));
  ok(info1.filtered && info1.rows === 27, `4 rows hidden by hand (${J(info1)})`);
  ok(J(await drawn()) === "[[3,60],[4,90]]", `the shown rows draw their own heights; Person 04's (hidden) is left out (${J(await drawn())})`);
  ok((await screenH(3)) === 60 && (await screenH(4)) === 90, `on screen: ${await screenH(3)} px and ${await screenH(4)} px`);
  ok(!/Custom row heights hidden/.test((await p.locator("footer.status").textContent()) || ""), "the old 'hidden while filtered' note is gone");
  await shot("heights-hidden.png");

  // A real drag on a shown row's line, with frozen rows (R3): shown row 5 is Person 09 (document row 9).
  await p.evaluate(() => {
    window.__nde.tab.frozenRows = 2;
    window.__nde.tab.bump();
  });
  await settle(300);
  const edge = await p.evaluate(() => {
    const a = window.__nde.grid.cellPoint({ row: 5, col: 0 });
    return { x: a.x - 30, y: a.y - 1 };
  });
  await p.mouse.move(edge.x, edge.y);
  await p.mouse.down();
  await p.mouse.move(edge.x, edge.y + 25, { steps: 6 });
  await p.mouse.up();
  await settle(500);
  const dh = await docHeights();
  const r9 = dh.find((x) => x[0] === 9);
  ok(!!r9 && r9[1] > 40 && !dh.some((x) => x[0] === 5), `the drag set document row 9 (Person 09), not shown row 5's number (${J(dh)})`);
  ok((await screenH(5)) === (r9 ? r9[1] : -1), `...and it draws at once on the shown row (${await screenH(5)} px)`);
  await p.evaluate(() => {
    window.__nde.tab.frozenRows = 0;
    window.__nde.tab.bump();
  });

  // Format > Height: shown row 6 is Person 10 (document row 10). Undo, Redo.
  await setH([6], 50);
  ok((await docHeights()).some((x) => x[0] === 10 && x[1] === 50), "Height of the selected rows lands on document row 10");
  await run("edit.undo");
  ok(!(await docHeights()).some((x) => x[0] === 10), "Undo takes it off");
  await run("edit.redo");
  ok((await docHeights()).some((x) => x[0] === 10 && x[1] === 50), "Redo puts it back");

  // Insert 2 rows above shown row 4 (Person 06) and delete shown rows 5-6 (Persons 09 and 10: rows 7-8 hidden between).
  await showAll();
  const before = J(await namedHeights());
  await hide(7, 8);
  await hide(4, 5);
  await sel(4, 0, 5, 0);
  await run("insert.rowsAbove");
  await settle(500);
  await sel(7, 0, 8, 0);
  const delWho = await p.evaluate(async () => window.__nde.backend.docRows(window.__nde.tab.id, 7, 2));
  await run("edit.deleteRows");
  await settle(500);
  const note = (await p.locator("footer.status").textContent()) || "";
  ok(!/heights were cleared/.test(note), `no heights were cleared (deleted document rows ${J(delWho)})`);
  await showAll();
  const after = await namedHeights();
  // Person 09 and 10 were deleted (their heights with them); the rest keep theirs.
  const want = J(JSON.parse(before).filter((x) => x[0] !== "Person 09" && x[0] !== "Person 10"));
  ok(J(after) === want, `after insert + delete while rows were hidden, each height is on the same person (${J(after)})`);
  // Undo the show, the delete and the insert, then show every row to compare.
  await run("edit.undo");
  await run("edit.undo");
  await run("edit.undo");
  await showAll();
  await settle(400);
  ok(J(await namedHeights()) === before, `Undo of both: every person keeps their height (${J(await namedHeights())})`);

  // Sorting while rows are hidden: heights stay by row position, as before.
  await hide(4, 5);
  const posBefore = J(await docHeights());
  await sel(1, 2);
  await run("data.sortAsc");
  await settle(600);
  ok(J(await docHeights()) === posBefore, `sorting while rows are hidden leaves each height on its row position (${J(await docHeights())})`);
  await run("edit.undo");
  await showAll();
  ok(J(await drawn()) === J(await docHeights()), "all rows shown again: drawn by document row");

  // ---- B. A real filter, on the 1M-row sample ----
  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => window.__nde.tab && window.__nde.tab.info.indexed && window.__nde.tab.info.rows > 1000, null, { timeout: 30000 });
  await settle(800);
  await p.getByRole("button", { name: "Yes, column names" }).click({ timeout: 1500 }).catch(() => {});
  const spec = { columns: [{ col: 3, rule: { type: "values", values: ["Osaka"], blanks: false } }], keep_first: true };
  const B = await p.evaluate(async (spec) => {
    const t = window.__nde.tab;
    const be = window.__nde.backend;
    // Where the first Osaka rows are, then a height on two of them and on a non-Osaka row.
    t.update(await be.setFilter(t.id, spec), false);
    const pos = await be.docRows(t.id, 1, 2);
    t.update(await be.clearFilter(t.id), false);
    const other = pos[0] + 1;
    t.rowHeights.set([pos[0]], 55);
    t.rowHeights.set([pos[1]], 75);
    t.rowHeights.set([other], 95);
    t.bump();
    const asks0 = t.shownAsks;
    // A burst of filter changes (as typing would), each followed by a redraw.
    for (let k = 0; k < 10; k++) {
      t.update(await be.setFilter(t.id, k % 2 ? { ...spec, keep_first: true } : spec), false);
      t.heightsInForce();
    }
    await new Promise((r) => setTimeout(r, 300));
    t.heightsInForce();
    await new Promise((r) => setTimeout(r, 300));
    return { pos, other, asks: t.shownAsks - asks0, drawn: t.heightsInForce()?.toView() ?? null, rows: t.info.rows };
  }, spec);
  ok(J(B.drawn) === "[[1,55],[2,75]]", `filter on: shown rows 1 and 2 (document rows ${B.pos}) draw 55 and 75 px; document row ${B.other} (not Osaka) is left out (${J(B.drawn)})`);
  ok(B.asks <= 3, `10 filter changes in a row asked the engine ${B.asks} times (at most 3)`);
  await settle(300);
  ok((await screenH(1)) === 55 && (await screenH(2)) === 75, "...and on screen");
  await shot("heights-filtered.png");

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  await b.close();
  console.log(failed ? `${failed} FAILED` : "ALL PASSED");
  process.exit(failed ? 1 : 0);
})();
