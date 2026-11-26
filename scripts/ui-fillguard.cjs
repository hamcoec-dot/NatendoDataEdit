// UI test of Fill blank cells with stops (C5) in the browser stand-in: the "Stop the fill at" group (greyed with its
// reason when filling with a text, the guard picker greying a ticked column), the count line before Run, the status
// after it, the amber marks of the cells left blank (F7 visits them, Esc and Ctrl+Z take them away), one undo step,
// and the guard-is-a-filled-column problem. Made-up data in the first rows of the stand-in's table.
// Run: (cd ui && npx vite --port 5321) then NODE_PATH=<folder with playwright> node scripts/ui-fillguard.cjs [screenshot dir] [url]
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
  const text = async () => ((await p.locator("body").textContent()) || "").replace(/\s+/g, " ");
  const dlg = "[role=dialog]:has(h2:has-text('Fill blank'))";
  const cmd = (id) => p.evaluate((id) => window.__nde.reg.run(id), id);
  const state = (id) =>
    p.evaluate((id) => {
      const c = window.__nde.reg.get(id);
      return { enabled: c.enabled(), why: c.why ? c.why() : "" };
    }, id);
  const col0 = () => p.evaluate(() => Array.from({ length: 10 }, (_, r) => window.__nde.tab.cache.get(r, 0) ?? "?"));
  const tickCol = (name) => p.locator(`${dlg} .all-none ~ div label`).filter({ hasText: new RegExp(`^\\s*${name}\\s*$`) }).first().locator("input").click();
  const marks = () => p.evaluate(() => ({ n: window.__nde.tab.fillMarked, on: [...window.__nde.tab.fillMarks].sort() }));

  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => window.__nde.tab && window.__nde.tab.info.indexed && window.__nde.tab.info.rows > 1000, null, { timeout: 30000 });
  await p.waitForTimeout(800);
  // A grouped report in rows 1-8 (made-up): region, id, amount; row 4 a spacer (every cell blank).
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    const rows = [
      ["Region", "ID", "Amount"],
      ["North", "101", "5"],
      ["", "102", "7"],
      ["Subtotal", "", "12"],
      ["", "", ""],
      ["", "103", "2"],
      ["South", "104", "9"],
      ["", "105", "1"],
      ["", "", "10"],
    ];
    const cells = [];
    rows.forEach((r, i) => r.forEach((v, c) => cells.push([i, c, v])));
    for (let c = 3; c < t.info.cols; c++) cells.push([4, c, ""]);
    t.update(await window.__nde.backend.setCells(t.id, cells), true);
    t.header = true;
  });
  await p.waitForTimeout(500);
  ok((await state("data.nextLeftBlank")).why.includes("Fill blank cells with a stop first"), "Next cell the fill left blank is greyed with its reason before a fill");

  await cmd("tools.fillBlanks");
  await p.waitForSelector(dlg);
  await tickCol("Region");
  const fs = `${dlg} [data-testid=fill-stops]`;
  ok(await p.isVisible(fs), "the Stop the fill at group is there");
  ok(await p.$eval(`${fs} select option:has-text('Region')`, (o) => o.disabled && /cannot both stop the fill and be filled/.test(o.title)), "the guard picker greys the column ticked for filling, with the reason");
  await p.selectOption(`${fs} select`, { label: "ID" });
  await p.waitForSelector(`${dlg} [data-testid=fill-count]:has-text('Will fill')`, { timeout: 15000 });
  const count = await p.textContent(`${dlg} [data-testid=fill-count]`);
  ok(/Will fill [\d,]+ blank cells?; [\d,]+ left blank by the stops\./.test(count), `the count line before Run: ${count}`);
  ok(/Counted in the first 200,000 of [\d,]+ shown rows/.test(count), "a capped count says both numbers");
  await p.click(`${dlg} label:has-text('With this text') input[type=radio]`);
  await p.waitForTimeout(200);
  ok(await p.$eval(fs, (f) => f.disabled && /Stops apply when copying/.test(f.title)), "filling with a text greys the stops, with the reason");
  ok(!(await p.isVisible(`${dlg} [data-testid=fill-count]`)), "no count line without stops in use");
  await p.click(`${dlg} label:has-text('From the cell above') input[type=radio]`);
  await p.click(`${fs} label:has-text('Blank rows') input`);
  await p.click(`${fs} label:has-text('Total rows') input`);
  await p.waitForSelector(`${dlg} [data-testid=fill-count]:has-text('Will fill')`, { timeout: 15000 });
  if (out) await p.screenshot({ path: path.join(out, "fill-stops-dialog.png") });
  await p.click(`${dlg} button.primary`);
  await p.waitForTimeout(1500);
  const t1 = await text();
  ok(/Filled [\d,]+ blank cells? in [\d,]+ rows?\. [\d,]+ blank cells? left blank on purpose by the stops \(marked amber/.test(t1), "the status says how many were left blank on purpose");
  let c0 = await col0();
  ok(JSON.stringify(c0.slice(0, 9)) === JSON.stringify(["Region", "North", "North", "Subtotal", "", "", "South", "South", ""]), `the label stops at the subtotal, the spacer and the row with no ID (${c0.slice(0, 9).join("|")})`);
  await p.waitForTimeout(400);
  let m = await marks();
  ok(m.n >= 3 && ["4:0", "5:0", "8:0"].every((k) => m.on.includes(k)), `the cells left blank are marked (${m.on.slice(0, 5).join(" ")})`);
  if (out) await p.screenshot({ path: path.join(out, "fill-stops-marks.png"), clip: { x: 0, y: 0, width: 900, height: 420 } });
  await p.evaluate(() => window.__nde.grid?.select?.({ row: 1, col: 0 }, { row: 1, col: 0 }));
  await p.evaluate(() => window.__nde.reg.run("data.nextProblem"));
  await p.waitForTimeout(400);
  let cell = await p.evaluate(() => ({ ...window.__nde.tab.cell }));
  ok(cell.row === 4 && cell.col === 0, `F7 goes to the next cell left blank (${cell.row}:${cell.col})`);
  await p.evaluate(() => window.__nde.reg.run("data.nextProblem"));
  await p.waitForTimeout(400);
  cell = await p.evaluate(() => ({ ...window.__nde.tab.cell }));
  ok(cell.row === 5 && cell.col === 0, `and on to the one after (${cell.row}:${cell.col})`);
  ok((await state("data.fillMarksOff")).enabled, "Remove the left-blank marks is offered");
  await p.click("canvas", { position: { x: 300, y: 300 } }).catch(() => {});
  await p.keyboard.press("Escape");
  await p.waitForTimeout(400);
  m = await marks();
  ok(m.n === 0 && m.on.length === 0, "Esc takes the marks away");
  await cmd("edit.undo");
  await p.waitForTimeout(800);
  c0 = await col0();
  ok(c0[2] === "" && c0[7] === "", "one Ctrl+Z puts the blanks back");

  // Run again, then undo: the marks go with it.
  await cmd("tools.fillBlanks");
  await p.waitForSelector(dlg);
  await tickCol("Region");
  await p.selectOption(`${fs} select`, { label: "ID" });
  await p.click(`${dlg} button.primary`);
  await p.waitForTimeout(1500);
  ok((await marks()).n > 0, "a second fill marks again");
  await cmd("edit.undo");
  await p.waitForTimeout(800);
  ok((await marks()).n === 0, "Ctrl+Z takes the marks away too");

  // The guard cannot also be a filled column.
  await cmd("tools.fillBlanks");
  await p.waitForSelector(dlg);
  await p.selectOption(`${fs} select`, { label: "ID" });
  await tickCol("ID");
  await p.waitForTimeout(200);
  ok(/cannot also be filled/.test(await p.textContent(dlg)), "ticking the guard column for filling says it cannot be both");
  await p.click(`${dlg} button.primary`);
  await p.waitForTimeout(300);
  ok(await p.isVisible(dlg), "Run does not go ahead");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);
  ok(!(await p.isVisible(dlg)), "Esc closes the dialog");

  ok(errors.length === 0, `no page errors (${errors.join("; ")})`);
  console.log(failed ? `${failed} FAILED` : "ALL PASSED");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
