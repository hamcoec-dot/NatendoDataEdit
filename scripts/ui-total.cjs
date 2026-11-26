// UI test of the total-row bar (C4 b) in the browser stand-in: the words when the numbers add up and when a column
// does not (named), the buttons offered (Delete only when they add up), Set aside and Delete as one undo step each,
// Keep, and no bar while rows are hidden. (The stand-in is told the total row; finding it is the engine's job.)
// Run: (cd ui && npx vite --port 5321) then NODE_PATH=<folder with playwright> node scripts/ui-total.cjs [screenshot dir] [url]
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
  const info = () => p.evaluate(() => ({ rows: window.__nde.tab.info.rows, notes: { ...window.__nde.tab.info.notes } }));
  const bar = () => p.locator(".notice, [role=status], div").filter({ hasText: "looks like a total row" }).last();
  /** The stand-in reports a total line on the last row; the tab looks again, as at open. */
  const mark = (addsUp) =>
    p.evaluate(async (addsUp) => {
      const t = window.__nde.tab;
      const last = t.info.rows - 1;
      window.__nde.backend.markTotal(t.id, {
        row: last,
        count: 1,
        label: "Total",
        checks: [
          { col: 4, total: "123.45", sum: addsUp ? "123.45" : "120.00", matches: addsUp, rounded: false },
          { col: 0, total: "9", sum: "9", matches: true, rounded: false },
        ],
        adds_up: addsUp,
        rounded: false,
        subtotals: 0,
      });
      t.totalRow = await window.__nde.backend.totalRow(t.id, t.header, {});
    }, addsUp);
  const button = (name) => p.getByRole("button", { name, exact: true });

  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => window.__nde.tab && window.__nde.tab.info.indexed && window.__nde.tab.info.rows > 1000, null, { timeout: 30000 });
  await p.waitForTimeout(800);
  const start = await info();

  await mark(true);
  await p.waitForTimeout(400);
  let t = await text();
  ok(/Row [\d,]+ looks like a total row \(“Total”\), and the rows above add up to it exactly\./.test(t), "the bar says the row adds up");
  ok((await button("Set aside").count()) === 1 && (await button("Delete").count()) === 1 && (await button("Keep").count()) === 1, "Set aside, Delete and Keep are offered");
  if (out) await p.screenshot({ path: path.join(out, "total-bar.png") });
  await button("Set aside").click();
  await p.waitForTimeout(600);
  let i = await info();
  ok(i.rows === start.rows - 1 && i.notes.below_set === 1, `Set aside: the row is below the table (${i.rows}, ${i.notes.below_set})`);
  ok(!/looks like a total row/.test(await text()), "the bar goes");
  await p.evaluate(() => window.__nde.reg.run("edit.undo"));
  await p.waitForTimeout(600);
  i = await info();
  ok(i.rows === start.rows && i.notes.below_set === 0, "one undo step puts it back");

  await mark(false);
  await p.waitForTimeout(400);
  t = await text();
  ok(/but [^.]*\(the rows above make 120\.00, it says 123\.45\) does not add up\./.test(t), "a column that does not add up is named with both figures");
  ok((await button("Delete").count()) === 0 && (await button("Set aside").count()) === 1, "no Delete when the numbers do not add up");
  await button("Keep").click();
  await p.waitForTimeout(400);
  ok(!/looks like a total row/.test(await text()) && (await info()).rows === start.rows, "Keep leaves the row and closes the bar");

  await mark(true);
  await p.waitForTimeout(400);
  await button("Delete").click();
  await p.waitForTimeout(600);
  ok((await info()).rows === start.rows - 1, "Delete takes the row out");
  await p.evaluate(() => window.__nde.reg.run("edit.undo"));
  await p.waitForTimeout(600);
  ok((await info()).rows === start.rows, "one undo step brings it back");

  // Not while rows are hidden.
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    t.update(await window.__nde.backend.change(t.id, { op: "hide_rows", at: 3, count: 1 }), true);
    t.bump();
  });
  await mark(true);
  await p.waitForTimeout(400);
  ok(!/looks like a total row/.test(await text()), "no bar while rows are hidden");
  ok(errors.length === 0, `no page errors (${errors.join("; ").slice(0, 200)})`);
  void bar;
  await b.close();
  process.exit(failed ? 1 : 0);
})();
