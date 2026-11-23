// UI test of filtered paste: a paste taller than the shown rows asks first
// (fill and stop, add new rows, Cancel), hidden rows and hidden columns are
// never written, copy leaves hidden columns out, and the status line says
// how many hidden rows were left alone.
// Run: (cd ui && npx vite --port 5197) then
// NODE_PATH=<folder with playwright> node scripts/ui-paste.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5197";
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
  await p.goto(url);
  await p.keyboard.press("Control+n");
  await p.waitForTimeout(800);
  // A small table typed in by pasting: k,v,w with a's and b's.
  const sel = (r, c, r2 = r, c2 = c) =>
    p.evaluate(([a, b2, c3, d]) => {
      window.__nde.grid.select({ row: a, col: b2 }, { row: c3, col: d });
      window.__nde.grid.focus();
    }, [r, c, r2, c2]).then(() => p.waitForTimeout(200));
  const clip = (t) => p.evaluate((x) => navigator.clipboard.writeText(x), t);
  const cell = (r, c) => p.evaluate(([a, b2]) => window.__nde.tab.cache.get(a, b2), [r, c]);
  const status = () => p.textContent(".status, footer").catch(() => "");
  const cmd = async (t) => {
    await p.evaluate(() => window.__nde.grid.focus());
    await p.keyboard.press("Alt+q");
    await p.keyboard.type(t);
    await p.waitForTimeout(250);
    await p.keyboard.press("Enter");
    await p.waitForTimeout(500);
  };
  await sel(0, 0);
  await clip("k\tv\tw\r\na\t1\tx1\r\nb\t2\tx2\r\na\t3\tx3\r\nb\t4\tx4\r\na\t5\tx5\r\n");
  await p.keyboard.press("Control+v");
  await p.waitForTimeout(600);
  ok((await p.evaluate(() => window.__nde.tab.info.rows)) === 6, "the table is pasted in");
  // Hide the "b" rows (file rows 3 and 5) by hand.
  await sel(2, 0);
  await p.keyboard.press("Control+9");
  await p.waitForTimeout(800);
  await sel(3, 0);
  await p.keyboard.press("Control+9");
  await p.waitForTimeout(800);
  const shown = await p.evaluate(() => window.__nde.tab.info.rows);
  ok(shown === 4, `rows hidden: 4 of 6 shown (${shown})`);

  // 5 rows pasted into 3 shown rows: the question, then Cancel.
  await sel(1, 1);
  await clip("P0\r\nP1\r\nP2\r\nP3\r\nP4\r\n");
  await p.keyboard.press("Control+v");
  await p.waitForTimeout(400);
  const dlg = await p.textContent("[role=dialog], [role=alertdialog]").catch(() => "");
  ok(/pasting 5 rows, but only 3 rows are showing/.test(dlg), `a taller paste asks first (${dlg.slice(0, 80)})`);
  if (out) await p.screenshot({ path: path.join(out, "paste-taller-than-filter.png"), clip: { x: 300, y: 250, width: 760, height: 380 } });
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);
  ok((await cell(1, 1)) === "1", "Cancel changes nothing");

  // Fill the shown rows and stop (the main button, Enter).
  await sel(1, 1);
  await p.keyboard.press("Control+v");
  await p.waitForTimeout(400);
  await p.keyboard.press("Enter");
  await p.waitForTimeout(700);
  ok((await p.evaluate(() => window.__nde.tab.info.total_rows)) === 6, "fill and stop adds no rows");
  ok(/last 2 rows of what you copied were left out/.test(await status()), `the status line says what was left out (${(await status()).slice(0, 120)})`);
  ok(/2 hidden rows between them were left as they were/.test(await status()), "and how many hidden rows were left alone");

  // Hidden column: hide w? Hide v (col 1), then paste two columns from k.
  await sel(1, 1);
  await p.keyboard.press("Control+0");
  await p.waitForTimeout(300);
  ok(await p.evaluate(() => window.__nde.tab.columns.isHidden(1)), "column v hidden");
  await sel(1, 0);
  await clip("a\tQ\r\n");
  await p.keyboard.press("Control+v");
  await p.waitForTimeout(700);
  ok((await cell(1, 2)) === "Q" && (await cell(1, 1)) === "P0", `a paste passes over the hidden column (${await cell(1, 1)}, ${await cell(1, 2)})`);
  // Copy leaves the hidden column out.
  await sel(1, 0, 1, 2);
  await p.keyboard.press("Control+c");
  await p.waitForTimeout(400);
  const copied = await p.evaluate(() => navigator.clipboard.readText());
  ok(copied.trim() === "a\tQ", `copy leaves the hidden column out (${JSON.stringify(copied)})`);
  // Clear leaves it alone too.
  await p.keyboard.press("Delete");
  await p.waitForTimeout(500);
  ok((await cell(1, 1)) === "P0" && (await cell(1, 2)) === "", "clear leaves the hidden column alone");
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(400);
  // A block repeated over a selection with a hidden column fills only the
  // shown columns and never writes past the selection.
  await sel(1, 0, 2, 2);
  await clip("X\r\nY\r\n");
  await p.keyboard.press("Control+v");
  await p.waitForTimeout(700);
  const rep = [await cell(1, 0), await cell(1, 1), await cell(1, 2), await cell(2, 0), await cell(2, 2)];
  ok(rep.join(",") === "X,P0,X,Y,Y" && (await p.evaluate(() => window.__nde.tab.info.cols)) === 3, `a repeated block stays in the selection's shown columns (${rep.join(",")})`);
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(400);
  // Selection to a new tab leaves the hidden column out too.
  await sel(1, 0, 1, 2);
  await cmd("Selection to a new tab");
  await p.waitForTimeout(600);
  const nt = await p.evaluate(() => [window.__nde.tab.info.cols, window.__nde.tab.cache.get(0, 0), window.__nde.tab.cache.get(0, 1)]);
  ok(nt[0] === 2 && nt[1] === "a" && nt[2] === "Q", `selection to a new tab leaves the hidden column out (${nt.join(",")})`);
  ok(/1 hidden column left out/.test(await status()), "and says so");
  await p.keyboard.press("Control+w");
  await p.waitForTimeout(400);
  const dlgW = await p.$("[role=dialog] button:has-text(\"Don't save\"), [role=alertdialog] button:has-text(\"Don't save\")");
  if (dlgW) await dlgW.click();
  await p.waitForTimeout(400);
  // Ctrl+Enter over rows with hidden rows between says they were left alone.
  await sel(1, 2, 3, 2);
  await p.keyboard.type("Z");
  await p.keyboard.press("Control+Enter");
  await p.waitForTimeout(700);
  ok((await cell(3, 2)) === "Z" && /2 hidden rows between them were left as they were/.test(await status()), `Ctrl+Enter says hidden rows were left alone (${(await status()).slice(0, 100)})`);
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(400);
  // A small clear says it too, and counts only the shown cells.
  await sel(1, 0, 3, 2);
  await p.keyboard.press("Delete");
  await p.waitForTimeout(600);
  ok(/Cleared 6 cells\. 2 hidden rows between/.test(await status()), `a clear counts shown cells and notes hidden rows (${(await status()).slice(0, 100)})`);
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(400);
  await p.evaluate(() => {
    window.__nde.tab.columns.showAll();
    window.__nde.tab.bump();
  });

  // Add the rest as new rows at the end.
  await sel(2, 1);
  await clip("N0\r\nN1\r\nN2\r\n");
  await p.keyboard.press("Control+v");
  await p.waitForTimeout(400);
  await p.click("[role=dialog] button:has-text('add the other'), [role=alertdialog] button:has-text('add the other')");
  await p.waitForTimeout(800);
  ok((await p.evaluate(() => window.__nde.tab.info.total_rows)) === 7, "the rest went into one new row at the end");
  ok(/added 1 new row at the end/.test(await status()), "the status line says so");
  const s = await p.evaluate(() => window.__nde.tab.drawRect);
  ok(s.row === (await p.evaluate(() => window.__nde.tab.info.rows)) - 1, "the new row is selected");

  // Hidden rows were never written.
  await cmd("Unhide all rows");
  await p.waitForTimeout(500);
  await sel(0, 0, 6, 1);
  await p.keyboard.press("Control+c");
  await p.waitForTimeout(400);
  const all = (await p.evaluate(() => navigator.clipboard.readText())).trim().split(/\r?\n/).map((l) => l.split("\t").join(","));
  console.log("ROWS", all.join(" | "));
  ok(all.includes("b,2") && all.includes("b,4") && all.length === 7, `hidden rows kept their values (${all.join(" | ")})`);

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
