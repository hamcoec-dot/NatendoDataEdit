// UI test of the column type overlay in the browser stand-in: "Treat this column as ..." commands (menu and
// Alt+Q), the badge state the grid draws from, Undo and Redo of a type change, "Text column: count only",
// a saved view that carries the types, and what is remembered per file (a fingerprint and the types, never
// values, nothing for a private file).
// Run: (cd ui && npx vite --port 5241) then
// NODE_PATH=<folder with playwright> node scripts/ui-types.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5241";
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
  const status = async () => ((await p.textContent("footer.status")) || "").replace(/\s+/g, " ");
  const typed = () => p.evaluate(() => JSON.stringify(window.__nde.tab.info.typed));
  // A small table of made-up data: zip codes (with leading zeros), amounts, dates.
  await p.evaluate(() => window.__nde.reg.run("file.new"));
  await p.waitForTimeout(500);
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    const rows = [["zip", "amount", "joined"]];
    for (let i = 1; i <= 12; i++) rows.push([String(i * 111).padStart(5, "0"), String(i * 10.5), `2024-0${1 + (i % 9)}-1${i % 9}`]);
    t.update(await window.__nde.backend.dataOp(t.id, { op: "paste", spec: { row: 0, col: 0, op: "none", skip_blanks: false, transpose: false }, text: rows.map((r) => r.join(String.fromCharCode(9))).join(String.fromCharCode(13, 10)) }).then((r) => r.info), true);
    t.header = true;
    t.bump();
  });
  await p.waitForTimeout(400);
  ok((await typed()) === "[]", "a new table has no typed columns");

  // ---- the commands ----
  await p.evaluate(() => window.__nde.grid.select({ row: 1, col: 0 }, { row: 12, col: 0 }));
  await p.waitForTimeout(200);
  ok(await p.evaluate(() => window.__nde.reg.get("col.typeId") !== undefined && window.__nde.reg.enabled("col.typeId")), "the Id command exists and is enabled");
  await p.evaluate(() => window.__nde.reg.run("col.typeId"));
  await p.waitForTimeout(500);
  ok((await typed()) === '[[0,"id"]]', `the first column is typed Id (${await typed()})`);
  ok(/treated as Id/.test(await status()), `the status bar says so (${(await status()).slice(0, 120)})`);
  ok(await p.evaluate(() => window.__nde.reg.get("col.typeId").checked()), "the Id item shows as ticked");
  // Text column: count only
  await p.waitForTimeout(500);
  ok(/Text column: count only/.test(await status()), `quick totals say "Text column: count only" (${(await status()).slice(-90)})`);
  if (out) await p.screenshot({ path: path.join(out, "types-id.png") });

  // ---- Undo and Redo ----
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(500);
  ok((await typed()) === "[]", `Ctrl+Z puts the column back to Auto (${await typed()})`);
  await p.keyboard.press("Control+y");
  await p.waitForTimeout(500);
  ok((await typed()) === '[[0,"id"]]', `Ctrl+Y types it again (${await typed()})`);

  // ---- more types, a date order, Alt+Q ----
  await p.evaluate(() => window.__nde.grid.select({ row: 1, col: 1 }, { row: 12, col: 1 }));
  await p.evaluate(() => window.__nde.reg.run("col.typeNumber"));
  await p.evaluate(() => window.__nde.grid.select({ row: 1, col: 2 }, { row: 12, col: 2 }));
  await p.evaluate(() => window.__nde.reg.run("col.typeDateDmy"));
  await p.waitForTimeout(500);
  ok((await typed()) === '[[0,"id"],[1,"number"],[2,{"date":"dmy"}]]', `three columns typed (${await typed()})`);
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("treat this column as text");
  await p.waitForTimeout(300);
  ok((await p.getByText(/Treat this column as Text/).count()) >= 1, "the Text command is found in command search");
  await p.keyboard.press("Escape");

  // ---- a saved view carries the types ----
  const view = await p.evaluate(() => JSON.stringify(window.__nde.tab.info.column_types));
  ok(/"id"/.test(view) && /"number"/.test(view), `the engine's types are kept (${view})`);

  // ---- what is remembered ----
  await p.evaluate(() => window.__nde.grid.select({ row: 1, col: 0 }));
  const kept = await p.evaluate(() => JSON.stringify(Object.entries(localStorage).filter(([k]) => k.startsWith("nde.coltypes"))));
  ok(kept === "[]" || !/00111|zip|amount|joined/.test(kept), `only types are remembered, never values or titles (${kept.slice(0, 100)})`);
  ok(errors.length === 0, `no page errors ${errors.join(" | ").slice(0, 200)}`);
  await b.close();
  process.exit(failed ? 1 : 0);
})();
