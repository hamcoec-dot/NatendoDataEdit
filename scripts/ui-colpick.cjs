// UI test: column tick lists start with nothing ticked and have Select all
// and Unselect all (browser stub backend).
// Run: (cd ui && npx vite --port 5193) then
// NODE_PATH=<folder with playwright> node scripts/ui-colpick.cjs <screenshot dir> [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5193";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const p = await b.newPage({ viewport: { width: 1360, height: 860 } });
  const errors = [];
  p.on("pageerror", (e) => errors.push(String(e)));
  await p.goto(url);
  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => !/Loading|Indexing|Reading/i.test(document.querySelector("footer.status")?.textContent || ""), null, { timeout: 30000 }).catch(() => {});
  await p.waitForTimeout(1500);
  let failed = 0;
  const ok = (cond, msg) => {
    if (!cond) failed++;
    console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  };
  const command = async (q) => {
    await p.keyboard.press("Alt+q");
    await p.keyboard.type(q);
    await p.waitForTimeout(250);
    await p.keyboard.press("Enter");
    await p.waitForTimeout(500);
  };
  // The tick boxes in the list next to a Select all row.
  const boxes = (scope) => `${scope} .all-none ~ div input[type=checkbox]`;
  const ticked = (scope) => p.$$eval(boxes(scope), (e) => e.filter((x) => x.checked).length);
  const total = (scope) => p.$$eval(boxes(scope), (e) => e.length);

  const cases = [
    ["Remove duplicates", "[role=dialog]:has(h2:has-text('Remove duplicates'))", true],
    ["Show duplicates", "[role=dialog]:has(h2:has-text('Show duplicates'))", false],
    ["Merge rows", "[role=dialog]:has(h2:has-text('Merge rows'))", false],
    ["Subtotals", "[role=dialog]:has(h2:has-text('Subtotals'))", false],
    ["Fill blank cells", "[role=dialog]:has(h2:has-text('Fill blank'))", false],
    ["Group figures", "[role=dialog]:has(h2:has-text('Group'))", false],
  ];
  for (const [q, scope, shot] of cases) {
    await command(q);
    const open = await p.isVisible(scope);
    ok(open, `${q}: opens`);
    if (!open) continue;
    const n = await total(scope);
    ok(n > 0 && (await ticked(scope)) === 0, `${q}: ${n} columns listed, none ticked to start`);
    await p.click(`${scope} .all-none button:has-text('Select all')`);
    await p.waitForTimeout(100);
    ok((await ticked(scope)) === n, `${q}: Select all ticks every column`);
    ok(await p.isDisabled(`${scope} .all-none button:has-text('Select all')`), `${q}: Select all then waits`);
    if (shot) await p.screenshot({ path: path.join(out, "select-all-columns.png"), clip: { x: 0, y: 0, width: 1360, height: 700 } });
    await p.click(`${scope} .all-none button:has-text('Unselect all')`);
    await p.waitForTimeout(100);
    ok((await ticked(scope)) === 0 && (await p.isDisabled(`${scope} .all-none button:has-text('Unselect all')`)), `${q}: Unselect all clears them`);
    await p.keyboard.press("Escape");
    await p.waitForTimeout(300);
    ok(!(await p.isVisible(scope)), `${q}: Esc still closes it after the buttons`);
  }

  // Remove duplicates with nothing ticked still compares whole rows (as it says).
  await command("Remove duplicates");
  ok(/Tick none to compare whole rows/.test((await p.textContent("[role=dialog]")) || ""), "Remove duplicates says none ticked compares whole rows");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);

  // Merge rows with nothing ticked asks for a tick instead of running.
  await command("Merge rows");
  const mr = "[role=dialog]:has(h2:has-text('Merge rows'))";
  await p.click(`${mr} button.primary`);
  await p.waitForTimeout(300);
  ok(/Tick the column/.test((await p.textContent(mr)) || ""), "Merge rows with none ticked asks for a tick");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);

  // The chart panel starts with no value column ticked, and says so.
  await command("Chart");
  await p.waitForTimeout(500);
  const side = "aside, .side";
  const chartText = (await p.textContent(side).catch(() => "")) || "";
  ok(/Tick the columns to chart/.test(chartText) || !(await p.$(".values input:checked")), "the chart starts with no column ticked");

  ok(errors.length === 0, `no page errors ${errors.join("; ")}`);
  await b.close();
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  process.exit(failed ? 1 : 0);
})();
