// UI test of the clean matrix in the browser stand-in: ticking, the header tick, preview counts and examples, a
// protected column greyed, hidden columns listed apart and left out of Select all, a saved cleaner, the right-click
// entry, apply in one undo step, and command search.
// Run: (cd ui && npx vite --port 5362) then NODE_PATH=<folder with playwright> node scripts/ui-matrix.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5362";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1360, height: 900 } });
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
    await p.evaluate((id) => void window.__nde.reg.run(id), id);
    await p.waitForTimeout(500);
  };
  const text = async () => ((await p.locator("body").textContent()) || "").replace(/\s+/g, " ");
  const cell = (r, c) => p.evaluate(async ([r, c]) => (await window.__nde.backend.cellText(window.__nde.tab.id, r, c)).text, [r, c]);

  await p.keyboard.press("Control+n");
  await p.waitForFunction(() => window.__nde.tab, null, { timeout: 10000 });
  await p.waitForTimeout(500);
  // Made-up people with messy cells; column 4 protected, column 5 hidden.
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    const rows = [
      ["Name", "Town", "Phone", "Code", "Note"],
      ["  ada   LOVE ", " York ", "(555) 010-0001", " a1 ", " n "],
      ["grace  HOPPER", "Leeds", "555.010.0002", " b2 ", "m"],
      [" alan turing ", " Hull", "555 010 0003", "c3", "k"],
      ["mary   shelley", "Bath ", "5550100004", "d4", "j"],
    ];
    const cells = rows.flatMap((line, r) => line.map((v, c) => [r, c, v]));
    t.update(await window.__nde.backend.change(t.id, { op: "insert_rows", at: 1, count: rows.length - 1 }), true);
    t.update(await window.__nde.backend.setCells(t.id, cells), true);
    t.header = true;
    t.update(await window.__nde.backend.setProtected(t.id, [3]), true);
    t.bump();
  });
  await p.waitForTimeout(500);
  await p.evaluate(() => window.__nde.grid.select({ row: 1, col: 4 }, { row: 1, col: 4 }));
  await run("format.hideCols");

  // Everything the app logs from here on, and the clipboard as it stands.
  await p.evaluate(() => {
    window.__log = [];
    window.__nde.backend.logEvent = async (_level, message) => void window.__log.push(message);
  });
  await ctx.grantPermissions(["clipboard-read", "clipboard-write"]);
  await p.evaluate(() => navigator.clipboard.writeText("untouched"));

  await p.evaluate(() => window.__nde.grid.select({ row: 1, col: 0 }, { row: 1, col: 0 }));
  await run("clean.matrix");
  const dlg = p.getByRole("dialog", { name: "Clean matrix" });
  await dlg.waitFor({ timeout: 5000 });
  const box = (name) => dlg.getByRole("checkbox", { name, exact: true });
  ok(await box("Code (protected)").isDisabled(), "a protected column is greyed");
  ok(/Hidden columns/.test(await text()), "hidden columns are listed apart");
  await dlg.getByRole("button", { name: "Select all", exact: true }).click();
  await p.waitForTimeout(300);
  ok((await box("Name").isChecked()) && !(await box("Note").isChecked()) && !(await box("Code (protected)").isChecked()), "Select all leaves hidden and protected columns out");
  ok(await dlg.getByRole("button", { name: "Preview" }).isDisabled(), "nothing to preview until a cleaner is ticked");
  await box("Trim for every ticked column").check();
  ok((await box("Dates as yyyy-mm-dd for every ticked column").isEnabled()) && !(await box("Dates as yyyy-mm-dd: Name").isChecked()) && (await box("Dates as yyyy-mm-dd: Code (protected)").count()) + (await dlg.getByRole("checkbox", { name: /^Dates as yyyy-mm-dd: Code/ }).count()) >= 1, "a dates tick for each column, unticked");
  await box("Collapse spaces: Name").check();
  await box("Digits only: Phone").check();
  await dlg.getByRole("combobox", { name: "Case: Name" }).selectOption("title");
  ok(await dlg.getByRole("button", { name: "Apply" }).isDisabled(), "Apply waits for a preview");
  await dlg.getByRole("button", { name: "Preview" }).click();
  await p.waitForTimeout(700);
  const count = async (name) => ((await dlg.locator("tbody tr", { has: p.getByRole("checkbox", { name, exact: true }) }).first().locator("td.count").textContent()) || "").trim();
  ok((await count("Name")) === "4" && (await count("Town")) === "3" && (await count("Phone")) === "3", `counts per column (${await count("Name")}, ${await count("Town")}, ${await count("Phone")})`);
  ok(/Before\s*After/.test(await text()) && /Ada Love/.test(await text()), "examples before and after");
  if (out) await p.screenshot({ path: path.join(out, "clean-matrix.png") });
  await box("Trim: Town").uncheck();
  ok(/press Preview again/.test(await text()) && (await dlg.getByRole("button", { name: "Apply" }).isDisabled()), "a change makes the preview stale");
  await box("Trim: Town").check();
  await dlg.getByRole("button", { name: "Choose a saved cleaner for Town" }).click();
  await p.waitForTimeout(400);
  ok(/tidy-names\.ndclean/.test(await text()), "the saved cleaner shows its file name");
  await dlg.getByRole("button", { name: "Preview" }).click();
  await p.waitForTimeout(600);
  await dlg.getByRole("button", { name: "Apply" }).click();
  await p.waitForTimeout(800);
  ok((await cell(1, 0)) === "Ada Love" && (await cell(1, 2)) === "5550100001" && (await cell(1, 1)) === "York", "applied to every ticked column");
  ok((await cell(1, 3)) === " a1 " && (await cell(1, 4)) === " n ", "protected and hidden columns untouched");
  ok(/Cleaned \d+ cells in 3 columns/.test(await text()), "and says what it did");
  await run("edit.undo");
  ok((await cell(1, 0)) === "  ada   LOVE " && (await cell(1, 2)) === "(555) 010-0001", "one undo puts everything back");

  await p.evaluate(() => window.__nde.grid.select({ row: 1, col: 1 }, { row: 2, col: 2 }));
  await run("clean.matrixHere");
  await dlg.waitFor({ timeout: 5000 });
  ok((await box("Town").isChecked()) && (await box("Phone").isChecked()) && !(await box("Name").isChecked()), "from a selection its columns start ticked");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);
  ok((await p.getByRole("dialog").count()) === 0, "Escape closes it");
  await p.evaluate(() => window.__nde.grid.select({ row: 1, col: 0 }, { row: 1, col: 0 }));
  const point = await p.evaluate(() => window.__nde.grid.cellPoint({ row: 1, col: 0 }));
  await p.mouse.click(point.x + 20, point.y - 8, { button: "right" });
  await p.waitForTimeout(400);
  ok(/Clean matrix \(these columns\)/.test(await text()), "the cell right-click lists it");
  await p.keyboard.press("Escape");
  await p.keyboard.press("Alt+q");
  await p.waitForTimeout(300);
  await p.keyboard.type("clean many columns at once");
  await p.waitForTimeout(500);
  ok(/Clean matrix/.test(await text()), "command search finds it");
  const logged = await p.evaluate(() => window.__log.join(" | "));
  ok(/Cleaned \d+ cells/.test(logged) && !/ada|hopper|turing|york|5550100/i.test(logged), `the log holds counts, never values (${logged.slice(0, 120)})`);
  ok((await p.evaluate(() => navigator.clipboard.readText())) === "untouched", "nothing is put on the clipboard");
  ok(errors.length === 0, `no page errors (${errors.slice(0, 3).join("; ")})`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
