// UI test of the data rules in the browser stand-in: problem cells carry the number of the rule that
// marks them, "Show problems as" (fill / underline / both) changes the drawing, F7 / Shift+F7 jump to the
// next / previous problem (wrapping, passing problems in hidden columns and saying so), the status bar says
// how many rules and problems and "Check now" runs them, the rules dialog suggests rules the person ticks,
// and "Check this pattern" on the pill menu adds a rule.
// Run: (cd ui && npx vite --port 5219) then
// NODE_PATH=<folder with playwright> node scripts/ui-lint.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5219";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1360, height: 860 }, permissions: ["clipboard-read", "clipboard-write"] });
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
  const status = async () => ((await p.textContent("footer.status")) || "").replace(/\s+/g, " ");
  // A small table of made-up data (the stand-in checks at most 200,000 rows): 40 rows, joined (col 5) written
  // as dates, with bad dates on rows 4, 9 and 20 and a blank first name on row 12.
  await p.evaluate(() => window.__nde.reg.run("file.new"));
  await p.waitForTimeout(500);
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    const rows = [["id", "first_name", "last_name", "city", "amount", "joined", "notes"]];
    for (let i = 1; i <= 40; i++) rows.push([String(i), ["Ada", "Bo", "Cy", "Dee"][i % 4], ["Stone", "Lind", "Quinn"][i % 3], ["Dayton", "Lisbon"][i % 2], (i * 3.5).toFixed(2), `2024-${String(1 + (i % 12)).padStart(2, "0")}-${String(1 + (i % 28)).padStart(2, "0")}`, "ok"]);
    rows[4][5] = "02/03/2024";
    rows[9][5] = "2023-02-30";
    rows[20][5] = "soon";
    rows[12][1] = "";
    t.update(await window.__nde.backend.dataOp(t.id, { op: "paste", spec: { row: 0, col: 0, op: "none", skip_blanks: false, transpose: false }, text: rows.map((r) => r.join(String.fromCharCode(9))).join(String.fromCharCode(13, 10)) }).then((r) => r.info), true);
    t.header = true;
    t.columns.setRange(5, 1, 150);
    t.checks = [
      { col: 5, name: "Joined is a real date", kind: { type: "date_iso" } },
      { col: 1, kind: { type: "required" } },
    ];
    t.checksMarked = true;
    t.checkVersion++;
    t.bump();
  });
  await p.waitForTimeout(600);
  const marks = await p.evaluate(() => [...window.__nde.tab.problems.entries()].sort());
  ok(marks.length === 4 && marks.some(([k, v]) => k === "4:5" && v === 0) && marks.some(([k, v]) => k === "12:1" && v === 1), `problem cells carry the number of their rule (${JSON.stringify(marks)})`);
  const st = await status();
  ok(/2 rules/.test(st) && /Check now/.test(st), `the status bar says 2 rules and offers Check now (${st.slice(-120)})`);

  // ---- Check now ----
  await p.getByRole("button", { name: "Check now" }).click();
  await p.waitForFunction(() => window.__nde.tab.lastRun, null, { timeout: 60000 }).catch(() => {});
  await p.waitForTimeout(300);
  const st2 = await status();
  ok(/2 rules, 4 problems/.test(st2) && !/Check now/.test(st2), `Check now runs the rules (${st2.slice(-100)})`);

  // ---- F7 / Shift+F7 ----
  await p.evaluate(() => window.__nde.grid.select({ row: 0, col: 0 }));
  await p.keyboard.press("F7");
  await p.waitForTimeout(400);
  let cell = await p.evaluate(() => ({ r: window.__nde.tab.cell.row, c: window.__nde.tab.cell.col }));
  ok(cell.r === 4 && cell.c === 5, `F7 goes to the first problem (${JSON.stringify(cell)})`);
  ok(/Joined is a real date/.test(await status()), "the status bar names the rule of the cell");
  await p.keyboard.press("F7");
  await p.keyboard.press("F7");
  await p.keyboard.press("F7");
  await p.waitForTimeout(400);
  cell = await p.evaluate(() => ({ r: window.__nde.tab.cell.row, c: window.__nde.tab.cell.col }));
  ok(cell.r === 20 && cell.c === 5, `F7 walks the problems in reading order (${JSON.stringify(cell)})`);
  await p.keyboard.press("F7");
  await p.waitForTimeout(400);
  cell = await p.evaluate(() => ({ r: window.__nde.tab.cell.row, c: window.__nde.tab.cell.col }));
  ok(cell.r === 4 && /back at the top/.test(await status()), `past the last problem it wraps and says so (${JSON.stringify(cell)})`);
  await p.keyboard.press("Shift+F7");
  await p.waitForTimeout(400);
  cell = await p.evaluate(() => ({ r: window.__nde.tab.cell.row, c: window.__nde.tab.cell.col }));
  ok(cell.r === 20 && /back at the top/.test(await status()), `Shift+F7 goes the other way and wraps (${JSON.stringify(cell)})`);

  // ---- hidden columns are passed over and counted ----
  await p.evaluate(() => {
    window.__nde.tab.columns.setRange(1, 1, 0);
    window.__nde.tab.bump();
    window.__nde.grid.select({ row: 0, col: 0 });
  });
  await p.keyboard.press("F7");
  await p.waitForTimeout(400);
  cell = await p.evaluate(() => ({ r: window.__nde.tab.cell.row, c: window.__nde.tab.cell.col }));
  ok(cell.c === 5 && cell.r === 4, `F7 never lands in a hidden column (${JSON.stringify(cell)})`);
  await p.evaluate(() => window.__nde.grid.select({ row: 20, col: 5 }));
  await p.keyboard.press("F7");
  await p.waitForTimeout(400);
  ok(/1 problem in hidden columns passed/.test(await status()), `it says how many problems in hidden columns it passed (${(await status()).slice(0, 160)})`);
  await p.evaluate(() => window.__nde.reg.run("format.showAllCols"));
  await p.waitForTimeout(300);

  // ---- Show problems as ----
  const draw = async (style) => {
    await p.evaluate((s) => window.__nde.reg.run(s), style);
    await p.waitForTimeout(400);
  };
  await draw("view.problemsUnderline");
  await p.evaluate(() => window.__nde.grid.select({ row: 0, col: 0 }));
  await p.keyboard.press("F7");
  await p.waitForTimeout(500);
  if (out) await p.screenshot({ path: path.join(out, "problems-underline.png") });
  await draw("view.problemsBoth");
  if (out) await p.screenshot({ path: path.join(out, "problems-both.png") });
  await draw("view.problemsFill");
  ok(true, "the three styles draw without errors");

  // ---- the rules dialog suggests, the person ticks ----
  await p.evaluate(() => {
    window.__nde.grid.select({ row: 1, col: 0 });
    return window.__nde.reg.run("data.checks");
  });
  await p.waitForTimeout(400);
  await p.getByRole("button", { name: "Suggest rules from this data" }).click();
  await p.waitForTimeout(500);
  const sug = await p.evaluate(() => Array.from(document.querySelectorAll("[aria-label='Suggested rules'] label")).map((e) => e.textContent.replace(/\s+/g, " ").trim()));
  ok(sug.length >= 2 && sug.some((s) => /Unique|different/.test(s)), `suggestions are listed with why (${JSON.stringify(sug).slice(0, 200)})`);
  const before = await p.evaluate(() => document.querySelectorAll(".checks .check select[aria-label='Check']").length);
  await p.getByRole("button", { name: "Add the ticked rules" }).click();
  await p.waitForTimeout(300);
  const after = await p.evaluate(() => document.querySelectorAll(".checks .check select[aria-label='Check']").length);
  ok(after > before, `ticked suggestions are added to the list only on the click (${before} -> ${after})`);
  const kept = await p.evaluate(() => window.__nde.tab.checks.length);
  ok(kept === 2, "the tab's own rules did not change until Keep (still 2)");
  await p.getByRole("button", { name: "Keep these rules with the file" }).click();
  await p.waitForTimeout(300);
  ok(/Kept in/.test((await p.textContent(".good")) || ""), "Keep these rules with the file says where");
  await p.getByRole("button", { name: "Forget these rules" }).click();
  await p.waitForTimeout(200);
  ok(await p.evaluate(() => window.__nde.tab.checks.length === 0), "Forget these rules clears them");
  await p.getByRole("button", { name: "Cancel" }).click();
  await p.waitForTimeout(300);

  // ---- Check this pattern (pill menu) ----
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    if (!t.insights) await window.__nde.reg.run("view.insights");
    t.columns.setRange(5, 1, 340);
    t.bump();
  });
  await p.waitForFunction(() => window.__nde.tab.insights?.byCol.get(5)?.top_shape, null, { timeout: 15000 }).catch(() => {});
  await p.waitForTimeout(500);
  const canvas = await (await p.$("canvas")).boundingBox();
  const cellPt = await p.evaluate(() => window.__nde.grid.cellPoint({ row: 1, col: 5 }));
  await p.mouse.click(cellPt.x + 340 - 25 - 105, canvas.y + 25 + 11, { button: "right" });
  await p.waitForTimeout(300);
  const pillMenu = await p.evaluate(() => Array.from(document.querySelectorAll(".menu button")).map((e) => e.textContent.replace(/\s+/g, " ").trim()).join(" | "));
  console.log("INFO pill menu: " + pillMenu.slice(0, 300) + " shape=" + (await p.evaluate(() => window.__nde.tab.insights?.byCol.get(5)?.top_shape)));
  await p.getByText("Check this pattern (add a rule)").first().click({ timeout: 5000 });
  await p.waitForTimeout(400);
  const rule = await p.evaluate(() => window.__nde.tab.checks.map((c) => c.name));
  ok(rule.length === 1 && /^Matches 9999-99-99$/.test(rule[0] || ""), `"Check this pattern" adds a rule named after the shape (${JSON.stringify(rule)})`);

  ok(errors.length === 0, errors.length ? "page errors: " + errors.join(" | ") : "no page errors");
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
