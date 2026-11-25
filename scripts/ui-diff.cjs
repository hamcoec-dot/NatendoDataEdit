// UI test of the key-aware diff in the browser stand-in: Tools > Compare (by key) opens the differences as a
// new tab and marks the first file, the second file and the result (changed cells amber, added rows green,
// removed rows red), Alt+F7 / Alt+Shift+F7 jump between differences and wrap, a sort keeps the marks on the
// right rows (they are looked up by key), an edit makes them "out of date", Clear takes them off.
// Run: (cd ui && npx vite --port 5222) then
// NODE_PATH=<folder with playwright> node scripts/ui-diff.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5222";
const TAB = String.fromCharCode(9);
const CRLF = String.fromCharCode(13, 10);
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1360, height: 860 } });
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
  const body = async () => ((await p.textContent("body")) || "").replace(/\s+/g, " ");
  const A = ["id,first_name,city", ...Array.from({ length: 30 }, (_, i) => `${i + 1},Person ${i + 1},${["Dayton", "Lisbon", "Perth"][i % 3]}`)];
  const B = ["city,id,first_name,zip", ...Array.from({ length: 30 }, (_, i) => `${i === 4 ? "Osaka" : ["Dayton", "Lisbon", "Perth"][i % 3]},${i + 1},${i === 9 ? "Changed name" : "Person " + (i + 1)},${46000 + i}`).filter((_, i) => i !== 14), "Quito,99,New person,1"];
  const open = async (lines) => {
    await p.evaluate(() => window.__nde.reg.run("file.new"));
    await p.waitForTimeout(400);
    await p.evaluate(async ([text]) => {
      const t = window.__nde.tab;
      t.update(await window.__nde.backend.dataOp(t.id, { op: "paste", spec: { row: 0, col: 0, op: "none", skip_blanks: false, transpose: false }, text }).then((r) => r.info), true);
      t.header = true;
      t.headerTouched = true;
      t.bump();
    }, [lines.map((l) => l.split(",").join(String.fromCharCode(9))).join(String.fromCharCode(13, 10))]);
    await p.waitForTimeout(300);
    return p.evaluate(() => window.__nde.tab.id);
  };
  const idA = await open(A);
  const idB = await open(B);
  // back to the first tab, then Tools > Compare by key
  const tabIndex = (id) => p.evaluate((x) => window.__nde.tabs.findIndex((t) => t.id === x), id);
  const goTab = async (id) => {
    await p.locator("[role=tab], .tab").nth(await tabIndex(id)).click({ position: { x: 20, y: 8 } });
    await p.waitForTimeout(600);
  };
  await goTab(idA);
  await p.evaluate(() => window.__nde.reg.run("tools.compare"));
  await p.waitForTimeout(500);
  const dialog = await body();
  ok(/Compare two files/.test(dialog) && /Choose another file/.test(dialog), "the compare screen opens with a way to choose another file");
  await p.locator("fieldset label", { hasText: "id" }).first().locator("input").check();
  await p.getByRole("button", { name: "Run" }).click();
  await p.waitForTimeout(1200);
  const tabs = await p.evaluate(() => window.__nde.tabs.map((t) => ({ id: t.id, on: t.diffOn, name: t.info.name })));
  const res = tabs[tabs.length - 1];
  ok(tabs.length === 3 && /Diff/.test(res.name) && tabs.every((t) => t.on), `the result opened as a new tab and all three tabs are in the diff (${JSON.stringify(tabs.map((t) => t.name))})`);
  const msg = await body();
  ok(/Compared by key: 2 changed rows \(2 cells\), 1 added row, 1 removed row, 27 rows the same/.test(msg), `the status line says what changed (${(msg.match(/Compared by key[^]*?the same/) || [""])[0]})`);
  ok(/columns only in the first file: none; only in the second: zip/.test(msg), "and warns about the column only in the second file");

  // marks on the first file
  await goTab(idA);
  const marksA = await p.evaluate(() => [...window.__nde.tab.diffMarks.entries()].sort());
  ok(marksA.length === 3 && marksA.some(([k, v]) => k === "5:2" && v === 0) && marksA.some(([k, v]) => k === "10:1" && v === 0) && marksA.some(([k, v]) => k === "15:*" && v === 2), `the first file marks the two changed cells and the removed row (${JSON.stringify(marksA)})`);
  if (out) await p.screenshot({ path: path.join(out, "compare-by-key.png") });

  // Alt+F7 walks the differences and wraps
  await p.evaluate(() => window.__nde.grid.select({ row: 0, col: 0 }));
  const rows = [];
  for (let i = 0; i < 4; i++) {
    await p.keyboard.press("Alt+F7");
    await p.waitForTimeout(500);
    rows.push(await p.evaluate(() => window.__nde.tab.cell.row));
  }
  ok(JSON.stringify(rows) === JSON.stringify([5, 10, 15, 5]), `Alt+F7 goes through the differences and wraps (${rows})`);
  ok(/Back at the top/.test(await body()), "and says it wrapped");
  await p.keyboard.press("Alt+Shift+F7");
  await p.waitForTimeout(500);
  ok((await p.evaluate(() => window.__nde.tab.cell.row)) === 15, "Alt+Shift+F7 goes the other way");

  // a sort keeps the marks on the same people
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    t.update(await window.__nde.backend.sortRows(t.id, { keys: [{ col: 1, descending: true, kind: "text", match_case: false }], keep_first: true }), true);
  });
  await p.waitForTimeout(700);
  const sorted = await p.evaluate(async () => {
    const t = window.__nde.tab;
    const out = [];
    for (const [k] of t.diffMarks) {
      const [r, c] = k.split(":");
      out.push([k, t.cache.get(Number(r), 1) ?? "?"]);
    }
    return out.sort();
  });
  ok(sorted.length === 3 && sorted.some(([k, v]) => k.endsWith(":2") && /Person (5)$/.test(v)) && sorted.some(([k, v]) => k.endsWith(":1") && v === "Person 10") && sorted.some(([k, v]) => k.endsWith(":*") && v === "Person 15"), `after a sort the marks are on the same people (${JSON.stringify(sorted)})`);

  // the result tab and the second file
  await goTab(res.id);
  const marksR = await p.evaluate(() => [...window.__nde.tab.diffMarks.entries()].length);
  ok(marksR >= 4, `the result tab is marked too (${marksR})`);
  await goTab(idB);
  const marksB = await p.evaluate(() => [...window.__nde.tab.diffMarks.entries()].sort());
  ok(marksB.filter(([k, v]) => v === 0).length === 2, `the second file marks its changed cells (${marksB.length} on screen)`);
  await p.evaluate(() => window.__nde.grid.select({ row: 10, col: 0 }));
  await p.keyboard.press("Alt+F7");
  await p.waitForTimeout(500);
  ok((await p.evaluate(() => window.__nde.tab.cell.row)) === 30, "Alt+F7 reaches the added row at the bottom of the second file");

  // an edit: out of date
  await goTab(idA);
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    t.update(await window.__nde.backend.setCells(t.id, [[3, 1, "Edited"]]), true);
  });
  await p.waitForTimeout(700);
  const ood = await p.evaluate(() => ({ marks: window.__nde.tab.diffMarks.size, out: window.__nde.tab.diffOutOfDate }));
  ok(ood.out && ood.marks === 0 && /out of date: compare again/.test(await body()), "after an edit the colours say out of date and nothing is drawn");

  // Clear
  await p.getByRole("button", { name: "Clear", exact: true }).first().click();
  await p.waitForTimeout(500);
  ok((await p.evaluate(() => window.__nde.tabs.every((t) => !t.diffOn))), "Clear takes the colours off every tab");

  ok(errors.length === 0, errors.length ? "page errors: " + errors.join(" | ") : "no page errors");
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
