// UI test of "ragged rows" in the browser stand-in: the open note with its Review button, showing all, short and long
// rows, the menus and command search, Go to special's entry, and the commands greyed with their reason after a column
// change. (The stand-in is told which rows are short and long; finding them is the engine's job.)
// Run: (cd ui && npx vite --port 5301) then NODE_PATH=<folder with playwright> node scripts/ui-ragged.cjs [screenshot dir] [url]
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
  const TAB = String.fromCharCode(9);
  const EOL = String.fromCharCode(13, 10);
  const run = async (id) => {
    await p.evaluate((id) => window.__nde.reg.run(id), id);
    await p.waitForTimeout(500);
  };
  const text = async () => ((await p.locator("body").textContent()) || "").replace(/\s+/g, " ");
  const info = () => p.evaluate(() => ({ rows: window.__nde.tab.info.rows, filtered: window.__nde.tab.info.filtered }));

  await run("file.new");
  await p.evaluate(
    async (t0) => {
      const t = window.__nde.tab;
      t.update(await window.__nde.backend.dataOp(t.id, { op: "paste", spec: { row: 0, col: 0, op: "none", skip_blanks: false, transpose: false }, text: t0 }).then((r) => r.info), true);
      t.header = true;
      t.bump();
    },
    [["id", "name", "city"], ["1", "ann", "Oslo"], ["2", "bob", ""], ["3", "cat", "Rome"], ["4", "Smith", "John"], ["5", "eve", "Lyon"]].map((r) => r.join(TAB)).join(EOL),
  );
  await p.waitForTimeout(400);
  ok(!/different number of fields/.test(await text()), "no note for a file without ragged rows");
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.update(window.__nde.backend.markRagged(t.id, [2], [4], 3), true);
    t.bump();
  });
  await p.waitForTimeout(400);
  ok(/2 rows have a different number of fields than most \(usual: 3\)\./.test(await text()), "the note counts the rows and names the usual width");
  if (out) await p.screenshot({ path: path.join(out, "ragged-note.png") });
  await p.getByRole("button", { name: "Review" }).click();
  await p.waitForTimeout(600);
  let i = await info();
  ok(i.filtered && i.rows === 3, `Review shows the title row and the two ragged rows (${i.rows})`);
  ok(/Showing the 2 rows with a different number of fields \(1 short, 1 long\)/.test(await text()), "and says how many are short and long");
  if (out) await p.screenshot({ path: path.join(out, "ragged-shown.png") });
  await run("data.clearFilter");
  await run("data.raggedShort");
  i = await info();
  ok(i.rows === 2 && /Showing the 1 row with too few fields/.test(await text()), "short rows alone");
  await run("data.clearFilter");
  await run("data.raggedLong");
  i = await info();
  ok(i.rows === 2 && /Showing the 1 row with too many fields/.test(await text()), "long rows alone");
  await run("data.clearFilter");
  // The menus and command search.
  await p.getByRole("button", { name: "Data", exact: true }).or(p.getByRole("menuitem", { name: "Data", exact: true })).first().click();
  await p.waitForTimeout(300);
  ok(/Show only rows with the wrong number of fields/.test(await text()), "the Data menu has it");
  await p.keyboard.press("Escape");
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("ragged rows");
  await p.waitForTimeout(400);
  ok(/wrong number of fields/.test(await text()), "command search finds it by “ragged rows”");
  await p.keyboard.press("Escape");
  await run("edit.gotoSpecial");
  ok(/Rows with the wrong number of fields/.test(await text()), "Go to special offers it");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);
  // After a column change: no note, greyed with the reason.
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    t.update(await window.__nde.backend.change(t.id, { op: "insert_cols", at: 1, count: 1 }), true);
    t.bump();
  });
  await p.waitForTimeout(400);
  ok(!/different number of fields than most/.test(await text()), "the note goes after a column is added");
  const state = await p.evaluate(() => {
    const c = window.__nde.reg.get("data.ragged");
    return { enabled: c.enabled(), why: c.why() };
  });
  ok(!state.enabled && /added, removed or moved/.test(state.why), `the command is greyed with the reason (${state.why.slice(0, 40)})`);
  ok(errors.length === 0, `no page errors (${errors.join("; ").slice(0, 200)})`);
  await b.close();
  process.exit(failed ? 1 : 0);
})();
