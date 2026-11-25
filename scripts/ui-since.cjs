// UI test of "What changed since this checkpoint" in the browser stand-in: the counts first, the changed cells a page
// at a time (was / now), a click going to the cell, the report, and the button greyed with its reason when the
// checkpoint is the table now or ahead of it.
// Run: (cd ui && npx vite --port 5301) then NODE_PATH=<folder with playwright> node scripts/ui-since.cjs [screenshot dir] [url]
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
  await p.evaluate(() => window.__nde.reg.run("file.new"));
  await p.waitForTimeout(500);
  const rows = [["name", "city"]];
  for (let i = 0; i < 200; i++) rows.push([`Person${i}`, `City${i % 7}`]);
  await p.evaluate(
    async (text) => {
      const t = window.__nde.tab;
      t.update(await window.__nde.backend.dataOp(t.id, { op: "paste", spec: { row: 0, col: 0, op: "none", skip_blanks: false, transpose: false }, text }).then((r) => r.info), true);
      t.header = true;
      t.bump();
    },
    rows.map((r) => r.join(TAB)).join(EOL),
  );
  await p.waitForTimeout(400);
  const edit = (r, c, text) => p.evaluate(async ([r, c, text]) => window.__nde.tab.update(await window.__nde.backend.setCells(window.__nde.tab.id, [[r, c, text]]), true), [r, c, text]);
  const name = async (n) => {
    await p.keyboard.press("Control+Alt+k");
    await p.getByRole("dialog").getByRole("textbox").fill(n);
    await p.keyboard.press("Enter");
    await p.waitForTimeout(400);
  };
  const item = (n) => p.getByTestId("checkpoint").filter({ has: p.locator(".n", { hasText: new RegExp(`^${n}$`) }) });

  await name("begin");
  await edit(1, 0, "Alpha");
  await edit(2, 1, "Paris");
  await edit(3, 0, "Gamma");
  await p.getByRole("button", { name: "What changed since begin" }).click();
  await p.getByTestId("since-summary").waitFor({ timeout: 5000 });
  const sum = ((await p.getByTestId("since-summary").textContent()) || "").replace(/\s+/g, " ");
  ok(/3 cells changed in 3 rows/.test(sum), `the counts come first (${sum.slice(0, 70)})`);
  ok(/3 steps since the checkpoint/.test(sum), "and the steps since");
  const lines = await p.locator(".since .line, [data-testid=since] .line").allTextContents();
  const flat = lines.map((l) => l.replace(/\s+/g, " "));
  ok(lines.length === 3 && flat.some((l) => /Person0.*Alpha/.test(l)) && flat.some((l) => /Person2.*Gamma/.test(l)) && flat.some((l) => /Paris/.test(l)), `the cells are listed was and now (${flat.join(" | ")})`);
  if (out) await p.screenshot({ path: path.join(out, "since.png") });
  // A click goes to the cell and closes the dialog.
  await p.locator("[data-testid=since] .line").filter({ hasText: "Paris" }).click();
  await p.waitForTimeout(400);
  const at = await p.evaluate(() => window.__nde.tab.cell);
  ok((await p.getByTestId("since").count()) === 0 && at.row === 2 && at.col === 1, `a click goes to the cell (row ${at.row}, column ${at.col})`);

  // Many changes: a pager over every changed cell, counts exact.
  await p.evaluate(async (text) => {
    const t = window.__nde.tab;
    t.update(await window.__nde.backend.dataOp(t.id, { op: "paste", spec: { row: 10, col: 0, op: "none", skip_blanks: false, transpose: false }, text }).then((r) => r.info), true);
  }, Array.from({ length: 120 }, (_, i) => `Pasted${i}`).join(EOL));
  await p.waitForTimeout(400);
  await p.getByRole("button", { name: "What changed since begin" }).click();
  await p.getByTestId("since-pager").waitFor({ timeout: 5000 });
  const pg = ((await p.getByTestId("since-pager").textContent()) || "").replace(/\s+/g, " ");
  ok(/Lines 1 to 50 of 123/.test(pg), `a pager over all 123 changed cells (${pg.trim().slice(0, 60)})`);
  await p.getByRole("button", { name: "Next 50" }).click();
  await p.waitForTimeout(500);
  const pg2 = ((await p.getByTestId("since-pager").textContent()) || "").replace(/\s+/g, " ");
  ok(/Lines 51 to 100 of 123/.test(pg2), `Next shows the following page with the same exact count (${pg2.trim().slice(0, 60)})`);
  // The report.
  await p.getByRole("button", { name: "Save as a report…" }).click();
  await p.waitForTimeout(500);
  ok(/Saved 123 lines/.test((await p.getByTestId("since").textContent()) || ""), "the report says how many cells it holds");
  await p.getByRole("button", { name: "Close", exact: true }).click();
  await p.waitForTimeout(300);

  // Greyed with the reason: the checkpoint that is the table now, and one ahead of the table.
  await name("second");
  const nowBtn = p.getByRole("button", { name: "What changed since second" });
  ok((await nowBtn.isDisabled()) && /nothing has changed/i.test((await nowBtn.getAttribute("title")) || ""), "the checkpoint that is the table now is greyed: nothing has changed");
  await edit(5, 0, "Later");
  await item("begin").locator("button.go").click();
  await p.waitForTimeout(800);
  const ahead = p.getByRole("button", { name: "What changed since second" });
  ok((await ahead.isDisabled()) && /ahead of the table/.test((await ahead.getAttribute("title")) || ""), "a checkpoint ahead of the table is greyed and says why");
  // The command: greyed with its reason when no checkpoint can be compared with, ready otherwise (the newest one).
  await p.evaluate(() => window.__nde.reg.run("file.new"));
  await p.waitForTimeout(500);
  const menuEntry = () => p.getByRole("menuitem", { name: "What changed since a checkpoint" });
  await p.getByRole("menuitem", { name: /^Edit$/ }).or(p.getByRole("button", { name: /^Edit$/ })).first().click();
  ok((await menuEntry().isDisabled()) && /No checkpoint yet/.test((await menuEntry().getAttribute("title")) || ""), "the Edit menu entry is greyed and says why when there is no checkpoint");
  await p.keyboard.press("Escape");
  await p.evaluate(
    async (text) => {
      const t = window.__nde.tab;
      t.update(await window.__nde.backend.dataOp(t.id, { op: "paste", spec: { row: 0, col: 0, op: "none", skip_blanks: false, transpose: false }, text }).then((r) => r.info), true);
      t.header = true;
      t.bump();
    },
    [["name", "city", "notes"], ["Ann", "Oslo", "n1"], ["Bob", "Rome", "n2"], ["Cat", "Lyon", "n3"]].map((r) => r.join(TAB)).join(EOL),
  );
  await p.waitForTimeout(400);
  await name("lists");
  const change = (c) => p.evaluate(async (c) => window.__nde.tab.update(await window.__nde.backend.change(window.__nde.tab.id, c), true), c);
  await change({ op: "delete_rows", at: 2, count: 1 });
  await change({ op: "delete_cols", at: 1, count: 1 });
  await p.waitForTimeout(500);
  await p.getByRole("menuitem", { name: /^Edit$/ }).or(p.getByRole("button", { name: /^Edit$/ })).first().click();
  ok(await menuEntry().isEnabled(), "after a change the Edit menu entry is ready");
  await menuEntry().click();
  await p.getByTestId("since-summary").waitFor({ timeout: 5000 });
  const s2 = ((await p.getByTestId("since-summary").textContent()) || "").replace(/\s+/g, " ");
  ok(/1 row taken out/.test(s2) && /1 column taken out: city/.test(s2), `the counts name the column taken out (${s2.slice(0, 90)})`);
  const all = (await p.locator("[data-testid=since] .line").allTextContents()).map((l) => l.replace(/\s+/g, " "));
  ok(all.some((l) => /\(whole row\).*Bob \| Rome \| n2.*\(taken out\)/.test(l)), `the row taken out is listed with its values (${all.join(" / ").slice(0, 120)})`);
  ok(all.some((l) => /city.*Oslo.*\(column taken out\)/.test(l)) && all.some((l) => /city.*Lyon.*\(column taken out\)/.test(l)), "and the cells of the column taken out");
  if (out) await p.screenshot({ path: path.join(out, "since-lists.png") });
  await p.getByRole("button", { name: "Close", exact: true }).click();
  // Right-click on a cell and on the tab.
  const box = await p.locator("canvas").first().boundingBox();
  await p.mouse.click(box.x + 90, box.y + 40, { button: "right" });
  ok((await menuEntry().count()) === 1 && (await menuEntry().isEnabled()), "the cell right-click menu has it");
  await p.keyboard.press("Escape");
  await p.locator("[role=tab]").filter({ hasText: "Untitled" }).last().click({ button: "right" });
  ok((await menuEntry().count()) === 1, "the tab right-click menu has it");
  await p.keyboard.press("Escape");
  // Command search.
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("what changed since a checkpoint");
  await p.waitForTimeout(300);
  await p.keyboard.press("Enter");
  await p.getByTestId("since-summary").waitFor({ timeout: 5000 });
  ok(true, "command search opens it");
  await p.getByRole("button", { name: "Close", exact: true }).click();
  ok(errors.length === 0, `no page errors ${errors.join(" | ").slice(0, 200)}`);
  await b.close();
  process.exit(failed ? 1 : 0);
})();
