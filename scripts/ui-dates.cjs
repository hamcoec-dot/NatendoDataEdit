// UI test of the date radar in the browser stand-in: the command (menu, greyed on a Text column with its reason), the
// amber marks on the right cells only, their tooltip, the pill with next and previous, and switching it off.
// (Telling the kinds of date apart is the engine's job.)
// Run: (cd ui && npx vite --port 5301) then NODE_PATH=<folder with playwright> node scripts/ui-dates.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5301";
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
    await p.waitForTimeout(700);
  };
  const select = async (row, col) => {
    await p.evaluate(([row, col]) => window.__nde.grid.select({ row, col }, { row, col }), [row, col]);
    await p.waitForTimeout(300);
  };
  const cell = () => p.evaluate(() => ({ ...window.__nde.tab.cell }));
  const marks = () => p.evaluate(() => [...window.__nde.tab.dateMarks].sort((a, b) => a - b));
  const command = (id) => p.evaluate((id) => ({ enabled: window.__nde.reg.get(id).enabled(), why: window.__nde.reg.get(id).why?.() ?? "" }), id);

  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => window.__nde.tab && window.__nde.tab.info.indexed && window.__nde.tab.info.rows > 1000, null, { timeout: 30000 });
  await p.waitForTimeout(800);
  await p.getByRole("button", { name: "Yes, column names" }).click({ timeout: 1500 }).catch(() => {});
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    t.update(await window.__nde.backend.setCells(t.id, [[1, 5, "03/04/2026"], [2, 5, "25/04/2026"], [3, 5, "05/05/2026"], [6, 5, "04/07/2026"]]), true);
    t.bump();
  });
  await p.waitForTimeout(500);
  await select(1, 5);
  let c = await command("data.nextDate");
  ok(!c.enabled && /Check dates in this column first/.test(c.why), "next is greyed until the check is on");
  await run("data.checkDates");
  const status = ((await p.textContent("footer.status")) || "").replace(/\s+/g, " ");
  ok(/could be read two ways/.test(status), `the note says what the column holds (${status.slice(0, 90)})`);
  ok(/Ambiguous dates: 2 \(first rows\)/.test((await p.getByTestId("date-pill").textContent()) || ""), "the pill counts the first rows and says so");
  const lit = await marks();
  ok(JSON.stringify(lit) === "[1,6]", `only the ambiguous cells are lit (${JSON.stringify(lit)})`);
  if (out) await p.screenshot({ path: path.join(out, "date-radar.png") });
  // The tooltip over the first lit cell.
  // (Row 7 of the table, the "joined" column, on this window size.)
  await p.mouse.move(790, 336);
  await p.waitForTimeout(500);
  const tip = await p.evaluate(() => [...document.querySelectorAll("[title], [role=tooltip]")].map((e) => e.getAttribute("title") || e.textContent).find((t) => /could be/.test(t || "")) || "");
  ok(/04\/07\/2026 could be 7 April or 4 July/.test(tip), `the tooltip gives both readings (${tip})`);
  await select(1, 5);
  await run("data.nextDate");
  ok((await cell()).row === 6, "next goes to the next ambiguous date");
  await p.getByRole("button", { name: "Previous ambiguous date" }).click();
  await p.waitForTimeout(600);
  ok((await cell()).row === 1, "and previous back");
  await p.getByRole("button", { name: "Count all rows" }).click();
  await p.waitForTimeout(800);
  ok((await p.getByRole("button", { name: "Count all rows" }).count()) === 0, "counting all rows is offered once, then done");
  // An edit takes the mark away.
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    t.update(await window.__nde.backend.setCells(t.id, [[6, 5, "2026-04-07"]]), true);
    t.bump();
  });
  await p.waitForTimeout(800);
  ok(JSON.stringify(await marks()) === "[1]", "an edited cell loses its mark");
  await p.getByRole("button", { name: "Remove the ambiguous date marks" }).click();
  await p.waitForTimeout(500);
  ok((await p.getByTestId("date-pill").count()) === 0 && (await marks()).length === 0, "the cross removes the pill and the marks");
  // Greyed on a Text column.
  await select(1, 1);
  await run("col.typeText");
  c = await command("data.checkDates");
  ok(!c.enabled && /set to Text/.test(c.why), `greyed on a Text column (${c.why})`);
  ok(errors.length === 0, `no page errors (${errors.join("; ").slice(0, 200)})`);
  await b.close();
  process.exit(failed ? 1 : 0);
})();
