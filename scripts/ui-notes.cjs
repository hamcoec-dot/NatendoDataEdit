// UI test of report notes (C4) in the browser stand-in: Headers start here and Set aside rows as footer notes,
// greyed with their reasons (a middle row, a sort, hidden rows), the header switch following undo and redo, the
// Report notes panel with its keep switch (it asks first) and Put back, the menus and command search.
// Run: (cd ui && npx vite --port 5321) then NODE_PATH=<folder with playwright> node scripts/ui-notes.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5321";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1280, height: 820 } });
  const p = await ctx.newPage();
  const errors = [];
  p.on("pageerror", (e) => errors.push(String(e)));
  await p.goto(url);
  await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });
  let failed = 0;
  const ok = (cond, msg) => {
    if (!cond) failed++;
    console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  };
  const run = async (id) => {
    await p.evaluate((id) => window.__nde.reg.run(id), id);
    await p.waitForTimeout(500);
  };
  const text = async () => ((await p.locator("body").textContent()) || "").replace(/\s+/g, " ");
  const info = () =>
    p.evaluate(() => {
      const t = window.__nde.tab;
      return { rows: t.info.rows, header: t.header, notes: { ...t.info.notes }, pos: t.info.history_pos };
    });
  const cmd = (id) =>
    p.evaluate((id) => {
      const c = window.__nde.reg.get(id);
      return { enabled: c.enabled(), why: c.why ? c.why() : "" };
    }, id);
  const select = async (row) => {
    await p.evaluate((row) => window.__nde.grid.select({ row, col: 0 }), row);
    // The engine's answer comes shortly after the selection settles.
    await p.waitForTimeout(450);
  };

  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => window.__nde.tab && window.__nde.tab.info.indexed && window.__nde.tab.info.rows > 1000, null, { timeout: 30000 });
  await p.waitForTimeout(800);
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.header = false;
    t.headerTouched = true;
    t.bump();
  });
  const start = await info();

  // Headers start here on row 4: the three rows above go aside, row 4 is the header.
  await select(3);
  let c = await cmd("data.headersStartHere");
  ok(c.enabled, `Headers start here is offered on row 4 (${c.why})`);
  await run("data.headersStartHere");
  let i = await info();
  ok(i.rows === start.rows - 3 && i.notes.above_set === 3, `three rows set aside above (${i.rows}, ${i.notes.above_set})`);
  ok(i.header === true, "and the first row is now the header");
  ok(/Set 3 rows aside above the table as report notes/.test(await text()), "the status says what happened");

  // A middle row is greyed with its reason; the last row can go below.
  await select(10);
  c = await cmd("data.setAsideFoot");
  ok(!c.enabled && /select rows/.test(c.why), `a middle row is greyed: ${c.why}`);
  await select(i.rows - 1);
  c = await cmd("data.setAsideFoot");
  ok(c.enabled, `the last row can be set aside below (${c.why})`);
  await run("data.setAsideFoot");
  i = await info();
  ok(i.notes.below_set === 1 && i.rows === start.rows - 4, `one row set aside below (${i.notes.below_set})`);

  // While sorted, the last row on screen is not the file's last row.
  await select(1);
  await run("data.sortDesc");
  await select((await info()).rows - 1);
  c = await cmd("data.setAsideFoot");
  ok(!c.enabled && /sorted/.test(c.why), `sorted: greyed with the reason (${c.why.slice(0, 80)})`);
  await run("edit.undo");

  // Hidden rows: refused, with the reason.
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    t.update(await window.__nde.backend.change(t.id, { op: "hide_rows", at: 5, count: 1 }), true);
    t.bump();
  });
  await select(2);
  c = await cmd("data.headersStartHere");
  ok(!c.enabled && /show all rows first/.test(c.why), `hidden rows: greyed (${c.why.slice(0, 60)})`);
  await run("edit.undo");

  // The panel.
  await run("view.panelNotes");
  let t = await text();
  ok(/Set aside above the table \(3 lines\)/.test(t) && /Set aside below the table \(1 line\)/.test(t), "the panel shows both parts with their line counts");
  ok(/Save report notes with the file/.test(t) && /Put back into the table/.test(t), "with the keep switch and Put back");
  if (out) await p.screenshot({ path: path.join(out, "notes-panel.png") });

  // The keep switch asks first, with the counts.
  await p.getByLabel("Save report notes with the file").click();
  await p.waitForTimeout(400);
  t = await text();
  ok(/Leave out the report notes when saving: the 4 lines you set aside/.test(t), "turning it off asks, with the real counts");
  await p.getByRole("button", { name: "Leave them out" }).click();
  await p.waitForTimeout(500);
  i = await info();
  ok(i.notes.keep === false, "and leaves them out");
  ok(/Saving leaves them out/.test(await text()), "the panel says so");
  await p.getByLabel("Save report notes with the file").click();
  await p.waitForTimeout(500);
  ok((await info()).notes.keep === true, "back on without a question; nothing lost");
  i = await info();
  ok(i.notes.above_set === 3 && i.notes.below_set === 1, "the notes are all still there");

  // Undo and redo carry the header switch with the rows.
  await run("edit.undo"); // keep on
  await run("edit.undo"); // keep off
  await run("edit.undo"); // foot
  await run("edit.undo"); // head
  i = await info();
  ok(i.rows === start.rows && i.header === false && i.notes.above_set === 0, `undo puts the rows back and the header off (${i.rows}, ${i.header})`);
  await run("edit.redo");
  i = await info();
  ok(i.header === true && i.notes.above_set === 3, "redo sets them aside again with the header on");

  // Put back.
  await run("data.putBackNotes");
  i = await info();
  ok(i.rows === start.rows && i.notes.above_set === 0 && i.header === false, `put back: rows back, header off (${i.rows})`);
  ok(/No report notes|no report notes/.test(await text()), "the panel says there are none");
  c = await cmd("data.putBackNotes");
  ok(!c.enabled && /no report notes/.test(c.why), "Put back is greyed with nothing to put back");

  // Menus and command search.
  await p.getByRole("button", { name: "Data", exact: true }).or(p.getByRole("menuitem", { name: "Data", exact: true })).first().click();
  await p.waitForTimeout(300);
  t = await text();
  ok(/Headers start here/.test(t) && /Set aside rows as footer notes/.test(t), "the Data menu has both");
  await p.keyboard.press("Escape");
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("report notes");
  await p.waitForTimeout(400);
  ok(/Headers start here/.test(await text()), "command search finds them by “report notes”");
  await p.keyboard.press("Escape");
  ok(errors.length === 0, `no page errors (${errors.join("; ").slice(0, 200)})`);
  await b.close();
  process.exit(failed ? 1 : 0);
})();
