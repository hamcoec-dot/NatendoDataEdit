// UI test of v0.4.1 (power-user batch 1): show invisible characters, Go to
// special, protect columns, tidy column titles, compare two columns, a zip
// with several tables, JSON Lines export and reopening last time's files.
// Run: (cd ui && npx vite --port 5196) then
// NODE_PATH=<folder with playwright> node scripts/ui-batch1.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5196";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1360, height: 860 }, permissions: ["clipboard-read", "clipboard-write"] });
  const p = await ctx.newPage();
  const errors = [];
  p.on("pageerror", (e) => errors.push(String(e)));
  let failed = 0;
  const ok = (cond, msg) => {
    if (!cond) failed++;
    console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  };
  const wait = (ms) => p.waitForTimeout(ms);
  const clip = (t) => p.evaluate((x) => navigator.clipboard.writeText(x), t);
  const cell = (r, c) => p.evaluate(([a, b2]) => window.__nde.tab.cache.get(a, b2), [r, c]);
  const status = () => p.textContent("footer.status").catch(() => "");
  const sel = (r, c, r2 = r, c2 = c) =>
    p.evaluate(([a, b2, c3, d]) => {
      window.__nde.grid.select({ row: a, col: b2 }, { row: c3, col: d });
      window.__nde.grid.focus();
    }, [r, c, r2, c2]).then(() => wait(200));
  const command = async (q) => {
    await p.keyboard.press("Alt+q");
    await p.keyboard.type(q);
    await wait(300);
    await p.keyboard.press("Enter");
    await wait(500);
  };
  const dialog = (title) => p.locator(`[role=dialog]:has(h2:has-text('${title}'))`);
  const paste = async (rows) => {
    await sel(0, 0);
    await clip(rows.join("\r\n") + "\r\n");
    await p.keyboard.press("Control+v");
    await wait(700);
    await p.evaluate(() => {
      window.__nde.tab.header = true;
      window.__nde.tab.bump();
    });
  };

  await p.goto(url);
  await p.keyboard.press("Control+n");
  await wait(700);
  await paste([
    "voter_id\tname\tmail_address\tres_address\tprecinct",
    "1001\tAnn  Lee\t12 Oak St\t12 OAK ST\t3",
    "1002\tBo Chan \t9 Elm Rd\t44 Pine Ave\t",
    "1003\tCy\u00a0Diaz\t\t7 Main St\t5",
    "1004\tDee Fox\t3 Ash Ct\t\t",
    "1005\tEd Gray\t5 Bay Ln\t5 Bay Ln\t7",
  ]);

  // ---- Show invisible characters ----
  await sel(3, 1);
  await p.keyboard.press("Control+Shift+H");
  await wait(400);
  const note = await p.textContent(".invnote").catch(() => "");
  ok(/1 non-breaking space/.test(note), `the cell bar names a non-breaking space (${note})`);
  await sel(2, 1);
  await wait(300);
  ok(/1 space at the end/.test(await p.textContent(".invnote").catch(() => "")), "a space at the end is named");
  await sel(1, 1);
  await wait(300);
  ok(/1 double space/.test(await p.textContent(".invnote").catch(() => "")), "a double space is named");
  ok((await cell(1, 1)) === "Ann  Lee", "display only: the value is unchanged");
  if (out) await p.screenshot({ path: path.join(out, "invisible.png"), clip: { x: 0, y: 0, width: 900, height: 300 } });
  await p.keyboard.press("Control+Shift+H");
  await wait(300);
  ok(!(await p.isVisible(".invnote")), "switched off, the note goes");

  // ---- Compare two columns ----
  await sel(1, 2);
  await command("Compare two columns");
  const cc = dialog("Compare two columns");
  ok(await cc.isVisible(), "Compare two columns opens");
  ok((await cc.locator("select").nth(0).inputValue()) === "2" && (await cc.locator("select").nth(1).inputValue()) === "3", "the active column and the next are picked");
  await cc.locator("label", { hasText: "Add a column" }).locator("input").check();
  await cc.locator("button:has-text('Compare')").click();
  await wait(800);
  const res = await p.evaluate(() => [0, 1, 2, 3, 4, 5].map((r) => window.__nde.tab.cache.get(r, 4)).join("|"));
  ok(res === "mail_address vs res_address|Same|Different|Only in second|Only in first|Same", `the result column (${res})`);
  await p.evaluate(() => {
    window.__nde.tab.columns.setWidth(4, 210);
    window.__nde.tab.bump();
  });
  await wait(300);
  if (out) await p.screenshot({ path: path.join(out, "compare-columns.png"), clip: { x: 0, y: 0, width: 1060, height: 300 } });
  // The browser stub adds the column and fills it in two undo steps (the
  // engine does it in one).
  // (Undo also steps back over the column width set for the picture.)
  await sel(1, 1);
  for (let i = 0; i < 4 && (await p.evaluate(() => window.__nde.tab.info.cols)) !== 5; i++) {
    await p.keyboard.press("Control+z");
    await wait(400);
  }
  ok((await p.evaluate(() => window.__nde.tab.info.cols)) === 5, "undone, the column is gone");
  await sel(1, 2);
  await command("Compare two columns");
  await cc.locator("label", { hasText: "Show only the rows" }).locator("input").check();
  await cc.locator("button:has-text('Compare')").click();
  await wait(800);
  // (The browser stub can't hide rows pasted into a new tab; the engine's
  // own test checks which rows show.)
  ok(await p.evaluate(() => window.__nde.tab.info.filtered), "a filter is on");
  ok(/Showing 3 rows/.test(await status()), `the status says so (${await status()})`);
  await command("Clear filter");
  await sel(1, 2);
  await command("Compare two columns");
  await cc.locator("button:has-text('Compare')").click();
  await wait(800);
  ok((await p.evaluate(() => window.__nde.tab.picked.size)) === 6, "light up: both cells of each differing row");
  await p.evaluate(() => {
    window.__nde.tab.picked = new Set();
    window.__nde.tab.bump();
  });

  // ---- Go to special ----
  await sel(1, 2, 5, 4);
  await p.keyboard.press("Control+Shift+G");
  await wait(400);
  const gs = dialog("Go to special");
  ok(await gs.isVisible(), "Ctrl+Shift+G opens Go to special");
  ok(await gs.locator("input[name=where][value=true]").isChecked().catch(() => true), "the selection is picked when several cells are selected");
  await gs.locator("label", { hasText: "Fill them with" }).locator("input[type=radio]").check();
  await gs.locator("input[aria-label='Fill them with']").fill("UNKNOWN");
  if (out) await p.screenshot({ path: path.join(out, "go-to-special.png"), clip: { x: 340, y: 90, width: 720, height: 680 } });
  await gs.locator("button:has-text('OK')").click();
  await wait(700);
  ok((await cell(3, 2)) === "UNKNOWN" && (await cell(4, 3)) === "UNKNOWN" && (await cell(2, 4)) === "UNKNOWN", "every blank in the selection is filled");
  ok((await cell(1, 2)) === "12 Oak St", "filled cells are untouched");
  ok(/Filled 4 cells/.test(await status()), `the status says how many (${await status()})`);
  await p.keyboard.press("Control+z");
  await wait(500);
  ok((await cell(3, 2)) === "" && (await cell(2, 4)) === "", "one undo step takes every fill back");
  // Light up the numbers in the whole table.
  await sel(1, 0);
  await p.keyboard.press("Control+Shift+G");
  await wait(400);
  await gs.locator("label", { hasText: "Cells with numbers" }).locator("input").check();
  await gs.locator("label", { hasText: "Light them up" }).locator("input").check();
  await gs.locator("button:has-text('OK')").click();
  await wait(700);
  const lit = await p.evaluate(() => window.__nde.tab.picked.size);
  ok(lit === 8, `numbers lit up in the whole table (${lit})`);
  ok((await p.evaluate(() => [window.__nde.tab.cell.row, window.__nde.tab.cell.col].join(","))) === "1,0", "the first is gone to");

  // ---- Protect a column ----
  await sel(1, 0);
  await command("Protect column");
  ok((await p.evaluate(() => window.__nde.tab.info.protected?.join(","))) === "0", "voter_id is protected");
  ok(/protected/.test(await status()), "the status says so");
  ok((await p.evaluate(() => window.__nde.tab.picked.size)) === 8, "protecting changes no cells (the lights stay)");
  await sel(1, 0, 2, 1);
  await clip("X1\tY1\r\nX2\tY2\r\n");
  await p.keyboard.press("Control+v");
  await wait(700);
  ok((await cell(1, 0)) === "1001" && (await cell(1, 1)) === "Y1", "a paste passes the protected column over");
  ok(/2 cells in protected columns were left as they were/.test(await status()), `the status counts the cells left (${await status()})`);
  await sel(2, 0);
  await p.keyboard.type("999");
  await p.keyboard.press("Enter");
  await wait(500);
  ok((await cell(2, 0)) === "1002", "typing in a protected column changes nothing");
  ok(/is protected, so nothing in it changes/.test(await status()), "and says why");
  ok(!(await p.evaluate(() => document.activeElement?.classList.contains("editor"))), "the cell editor closes (it is not left open)");
  await sel(1, 0);
  await command("Protect column");
  ok(!(await p.evaluate(() => window.__nde.tab.info.protected?.length)), "Protect again switches it off");

  // ---- Tidy column titles ----
  await p.evaluate(() => window.__nde.backend.setCells(window.__nde.tab.id, [[0, 1, "  Full   NAME "], [0, 4, "voter_id"]]).then((d) => window.__nde.tab.update(d, true)));
  await wait(400);
  await command("Tidy column titles");
  const tt = dialog("Tidy column titles");
  ok(await tt.isVisible(), "Tidy column titles opens");
  await tt.locator("select").selectOption("snake");
  await wait(500);
  const rows = await tt.locator("tbody tr").count();
  ok(rows === 2, `the preview lists only the titles that change (${rows})`);
  await tt.locator("button.primary").click();
  await wait(700);
  const titles = await p.evaluate(() => [0, 1, 2, 3, 4].map((c) => window.__nde.tab.cache.get(0, c)).join(","));
  ok(titles === "voter_id,full_name,mail_address,res_address,voter_id_2", `titles tidied and made unique (${titles})`);
  await sel(1, 1);
  await p.keyboard.press("Control+z");
  await wait(500);
  ok((await cell(0, 1)) === "  Full   NAME " && (await cell(0, 4)) === "voter_id", `one undo step (${await cell(0, 1)}|${await cell(0, 4)})`);

  // ---- A zip holding several tables ----
  const before = await p.evaluate(() => window.__nde.tabs.length);
  await p.evaluate(() => window.__nde.showZip({ token: 1, name: "export.zip", entries: [{ name: "data/voters.csv", size: 2048 }, { name: "precincts.tsv", size: 512 }] }));
  await wait(300);
  const zp = dialog("Open from export.zip");
  ok(await zp.isVisible(), "a zip with several tables lists them");
  ok((await zp.locator("input[type=checkbox]:checked").count()) === 1, "the first is ticked");
  if (out) await p.screenshot({ path: path.join(out, "zip-pick.png"), clip: { x: 300, y: 100, width: 760, height: 460 } });
  await zp.locator("button:has-text('Open all 2')").click();
  await wait(700);
  const names = await p.evaluate(() => window.__nde.tabs.map((t) => t.info.name));
  ok(names.length === before + 2 && names.includes("export.zip › voters.csv"), `each table opens in its own tab (${names.join(", ")})`);
  ok(/the zip is never changed/.test(await status()), "and Save as is explained");
  await p.evaluate((n) => {
    window.__nde.activate(n - 1);
  }, before);
  await wait(400);

  // ---- JSON Lines export ----
  await sel(1, 0);
  await command("Export");
  await wait(500);
  // The made-up voter list looks private: the usual check comes first.
  const go = p.locator("button:has-text('Continue, and don')");
  if (await go.isVisible()) {
    await go.click();
    await wait(500);
  }
  const ex = p.locator("[role=dialog]:has(h2:has-text('Export'))");
  if (!(await ex.isVisible())) console.log("dialogs:", await p.$$eval("[role=dialog] h2", (e) => e.map((x) => x.textContent)), await status(), await p.evaluate(() => document.activeElement?.outerHTML.slice(0, 120)));
  if (await ex.isVisible()) {
    await ex.locator("select").first().selectOption("json_lines");
    await wait(700);
    const prev = await ex.locator("pre, textarea").first().textContent().catch(() => "");
    ok(/^\{"voter_id":"[^"]*",/m.test(prev || "") && !/^\[/.test(prev || ""), `the export preview is one record per line (${(prev || "").slice(0, 60)})`);
    await p.keyboard.press("Escape");
    await wait(300);
  } else ok(false, "the Export dialog opens");

  // ---- Pick up where you left off (the stub keeps the list in memory) ----
  const saved = await p.evaluate(async () => {
    const t = window.__nde.tab;
    window.__nde.backend.docs.get(t.id).info.path = "C:/Data/voters.csv";
    return window.__nde.backend.sessionSave([{ doc: t.id, view: { name: "x" }, row: 3, col: 2, active: true }]);
  });
  ok(saved === 1, "a tab with a file is remembered");
  const listed = await p.evaluate(() => window.__nde.backend.sessionList());
  ok(listed.length === 1 && listed[0].name === "voters.csv", "listed by name only");

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
