// UI test of the repair bar in the browser stand-in: how it opens (note, menu command greyed with its reason on a
// usual row), the raw text, the marks and the live preview, keys, the three repairs it asks the app for, walking the
// ragged rows, and closing. (The stand-in is told which rows are short and long; the changes are the engine's job.)
// Run: (cd ui && npx vite --port 5301) then NODE_PATH=<folder with playwright> node scripts/ui-repair.cjs [screenshot dir] [url]
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
    await p.waitForTimeout(600);
  };
  const text = async (sel) => ((await p.getByTestId(sel).textContent()) || "").replace(/\s+/g, " ").trim();
  const command = () => p.evaluate(() => ({ enabled: window.__nde.reg.get("data.repairRow").enabled(), why: window.__nde.reg.get("data.repairRow").why() }));
  const select = async (row) => {
    await p.evaluate((row) => window.__nde.grid.select({ row, col: 0 }, { row, col: 0 }), row);
    await p.waitForTimeout(500);
  };
  const asked = () => p.evaluate(() => window.__nde.backend.calls.filter((c) => c.startsWith("repairRow:")).pop() || "");

  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => window.__nde.tab && window.__nde.tab.info.indexed && window.__nde.tab.info.rows > 1000, null, { timeout: 30000 });
  await p.waitForTimeout(800);
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.header = true;
    t.update(window.__nde.backend.markRagged(t.id, [3, 7], [5], t.info.cols), true);
    t.bump();
  });
  await p.waitForTimeout(500);

  await p.getByRole("button", { name: "Yes, column names" }).click({ timeout: 1500 }).catch(() => {});
  await select(2);
  let c = await command();
  ok(!c.enabled && /usual number of fields/.test(c.why), `greyed on a usual row, with the reason (${c.why})`);
  await select(5);
  c = await command();
  ok(c.enabled, "enabled on a ragged row");

  await p.getByRole("button", { name: "Repair…" }).click();
  await p.getByTestId("repair-heading").waitFor({ timeout: 5000 });
  ok(/1 of 3 ragged rows/.test(await text("repair-count")), "the note opens the bar at the first ragged row");
  ok(/^Row 4 of the file: \d+ fields, usual \d+\./.test(await text("repair-heading")), `the row is named (${await text("repair-heading")})`);
  ok((await text("repair-raw")).includes(","), "the file's text is shown");
  const apply = p.getByTestId("repair").getByRole("button", { name: "Apply" });
  ok(await apply.isDisabled(), "nothing marked: Apply is greyed");
  const cells = () => p.getByTestId("repair-result").locator("td").count();
  const before = await cells();
  await p.getByRole("button", { name: "Join fields 2 and 3" }).click();
  ok((await cells()) === before - 1, "a click joins two fields in the preview");
  if (out) await p.screenshot({ path: path.join(out, "repair-bar.png") });
  await p.getByRole("button", { name: "Split fields 2 and 3" }).click();
  ok((await cells()) === before, "and a second click splits them again");
  // Keys: Right, Space, Enter.
  await p.getByTestId("repair").focus();
  await p.keyboard.press("ArrowRight");
  await p.keyboard.press("ArrowRight");
  await p.keyboard.press("Space");
  // The cursor was left on the second mark by the clicks.
  ok((await p.getByRole("button", { name: "Split fields 4 and 5" }).count()) === 1, "Left, Right and Space mark a boundary");
  await p.keyboard.press("ArrowDown");
  await p.waitForTimeout(500);
  ok(/2 of 3/.test(await text("repair-count")) && (await apply.isDisabled()), "Down goes to the next ragged row, marks cleared");
  await p.keyboard.press("ArrowUp");
  await p.waitForTimeout(500);
  ok(/1 of 3/.test(await text("repair-count")), "Up goes back");
  // Join, apply: asked for, and the bar stays on the next one.
  await p.getByRole("button", { name: "Join fields 1 and 2" }).click();
  await apply.click();
  await p.waitForTimeout(800);
  ok(/^repairRow:3:\{"kind":"join","groups":\[2,1/.test(await asked()), `a join asks for its groups (${(await asked()).slice(0, 60)})`);
  ok(/1 of 2 ragged rows/.test(await text("repair-count")) && /Row 6 of the file/.test(await text("repair-heading")), `then the next ragged row (${await text("repair-heading")})`);
  // Join with the next row.
  await p.getByRole("combobox", { name: "Put between the two rows" }).selectOption("nothing");
  ok(/Joins row 7 of the file/.test(await text("repair-next")), "the next row is named");
  await apply.click();
  await p.waitForTimeout(800);
  ok(/"kind":"next","between":"nothing"/.test(await asked()), "join with the next row, nothing between");
  ok(/Undo \(Ctrl\+Z\) takes it back/.test((await p.textContent("footer.status")) || ""), "the status line says how to undo");
  // Onto the previous row.
  await p.getByRole("radio", { name: "Join with the previous row" }).check();
  ok(/Joins onto row \d+ of the file/.test(await text("repair-next")), `the previous row is named (${await text("repair-next")})`);
  await apply.click();
  await p.waitForTimeout(800);
  ok(/"kind":"previous","between":"nothing","header":true/.test(await asked()), `join onto the previous row (${await asked()})`);
  await p.getByTestId("repair").focus();
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);
  ok((await p.getByTestId("repair").count()) === 0, "Esc closes the bar");
  // After a column change the command is greyed with the reason.
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    t.update(await window.__nde.backend.change(t.id, { op: "insert_cols", at: 1, count: 1 }), true);
    t.bump();
  });
  await p.waitForTimeout(500);
  c = await command();
  ok(!c.enabled && /added, removed or moved/.test(c.why), "greyed after a column change");
  ok(errors.length === 0, `no page errors (${errors.join("; ").slice(0, 200)})`);
  await b.close();
  process.exit(failed ? 1 : 0);
})();
