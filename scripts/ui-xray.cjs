// UI test of X-ray gremlins in the browser stand-in: the toggle (Ctrl+Shift+X), what the status bar says about the
// screen, the cell bar note, the report dialog (counts per kind and column), Sanitize with its exact counts, the
// homoglyph count and the pager over every changed cell, one undo step, and that a word of one script, a hyphenated
// pair of scripts and a Cyrillic letter with no twin are never touched.
// Run: (cd ui && npx vite --port 5251) then
// NODE_PATH=<folder with playwright> node scripts/ui-xray.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5251";
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
  const status = async () => ((await p.textContent("footer.status")) || "").replace(/\s+/g, " ");
  // Made-up names: 60 rows, a third with a Cyrillic "а" inside a Latin word, a third with curly quotes, the rest plain;
  // three rows that must NOT be flagged.
  await p.evaluate(() => window.__nde.reg.run("file.new"));
  await p.waitForTimeout(500);
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    const tab = String.fromCharCode(9);
    const rows = [["name", "note"]];
    for (let i = 0; i < 60; i++) {
      if (i % 3 === 0) rows.push([`Pаul${i}`, "ok"]);
      else if (i % 3 === 1) rows.push([`O’Brien${i}`, "ok"]);
      else rows.push([`plain${i}`, "ok"]);
    }
    rows.push(["Москва", "pure Cyrillic"]);
    rows.push(["Smith-Иванова", "hyphenated"]);
    rows.push(["xбy", "no twin"]);
    t.update(await window.__nde.backend.dataOp(t.id, { op: "paste", spec: { row: 0, col: 0, op: "none", skip_blanks: false, transpose: false }, text: rows.map((r) => r.join(tab)).join(String.fromCharCode(13, 10)) }).then((r) => r.info), true);
    t.header = true;
    t.bump();
  });
  await p.waitForTimeout(500);

  // ---- the toggle and the status line ----
  ok(!(await status()).includes("X-ray"), "X-ray is off at first (no status text)");
  await p.keyboard.press("Control+Shift+X");
  await p.waitForTimeout(700);
  const st = await status();
  ok(/X-ray: \d+ cells?, 1 column/.test(st), `the status bar says how many cells and columns on the screen (${st.match(/X-ray[^·]*/)?.[0]?.trim()})`);
  if (out) await p.screenshot({ path: path.join(out, "xray.png") });
  await p.evaluate(() => window.__nde.grid.select({ row: 1, col: 0 }));
  await p.waitForTimeout(300);
  ok((await p.getByText(/X-ray: Cyrillic а \(U\+0430\) inside a Latin word/).count()) >= 1, "the cell bar names the look-alike letter");

  // ---- the report ----
  await p.evaluate(() => window.__nde.reg.run("view.xrayReport"));
  await p.getByTestId("xray-summary").waitFor({ timeout: 5000 });
  const sum = ((await p.getByTestId("xray-summary").textContent()) || "").replace(/\s+/g, " ");
  ok(/40 cells in 40 rows/.test(sum), `the report counts exactly (${sum.slice(0, 80)})`);
  ok((await p.getByText(/20 look-alike letters/).count()) >= 1 && (await p.getByText(/20 smart quotes/).count()) >= 1, "counts per kind: 20 look-alike letters, 20 smart quotes");
  await p.getByRole("button", { name: "Close", exact: true }).click();

  // ---- Sanitize: exact counts, the homoglyph count, a pager over every changed cell ----
  await p.evaluate(() => window.__nde.grid.select({ row: 1, col: 0 }, { row: 63, col: 0 }));
  await p.evaluate(() => window.__nde.reg.run("edit.sanitize"));
  await p.getByTestId("homoglyph-count").waitFor({ timeout: 5000 });
  await p.waitForTimeout(500);
  const hc = ((await p.getByTestId("homoglyph-count").textContent()) || "").replace(/\s+/g, " ");
  ok(/20 cells have a look-alike letter/.test(hc), `the homoglyph count is shown before Apply (${hc.trim()})`);
  ok((await p.getByTestId("pager").count()) === 0, "40 changed cells fit one page: no pager");
  // The pure Cyrillic, hyphenated and no-twin names are not in the list.
  const tableText = (await p.locator(".preview table").textContent()) || "";
  ok(!/Москва|Иванова|xбy/.test(tableText), "words of one script, hyphenated scripts and a letter with no twin are not listed");
  if (out) await p.screenshot({ path: path.join(out, "xray-sanitize.png") });
  await p.getByRole("button", { name: "Run this tool" }).click();
  await p.waitForTimeout(700);
  const after = await p.evaluate(async () => {
    const t = window.__nde.tab;
    const c = async (r) => (await window.__nde.backend.cellText(t.id, r, 0)).text;
    return [await c(1), await c(2), await c(3), await c(61), await c(62), await c(63)];
  });
  ok(after[0] === "Paul0" && after[1] === "O'Brien1" && after[2] === "plain2", `look-alike letters and curly quotes are fixed (${after.slice(0, 3).join(", ")})`);
  ok(after[3] === "Москва" && after[4] === "Smith-Иванова" && after[5] === "xбy", "the three look-alike-free names are untouched");
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(500);
  const back = await p.evaluate(async () => (await window.__nde.backend.cellText(window.__nde.tab.id, 1, 0)).text);
  ok(back === "Pаul0", "Ctrl+Z puts it all back in one step");
  // ---- more than a page of changes: every changed cell can be listed, a page at a time ----
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    const tab = String.fromCharCode(9);
    const rows = [];
    for (let i = 0; i < 120; i++) rows.push([`Q’s${i}`, "more"]);
    t.update(await window.__nde.backend.dataOp(t.id, { op: "paste", spec: { row: 64, col: 0, op: "none", skip_blanks: false, transpose: false }, text: rows.map((r) => r.join(tab)).join(String.fromCharCode(13, 10)) }).then((r) => r.info), true);
  });
  await p.evaluate(() => window.__nde.grid.select({ row: 1, col: 0 }, { row: 183, col: 0 }));
  await p.evaluate(() => window.__nde.reg.run("edit.sanitize"));
  await p.getByTestId("pager").waitFor({ timeout: 5000 });
  const pg = ((await p.getByTestId("pager").textContent()) || "").replace(/\s+/g, " ");
  ok(/Changed cells 1 to 50 of 160/.test(pg), `a pager over all 160 changed cells (${pg.trim()})`);
  await p.getByRole("button", { name: "Next 50" }).click();
  await p.waitForTimeout(600);
  const pg2 = ((await p.getByTestId("pager").textContent()) || "").replace(/\s+/g, " ");
  ok(/Changed cells 51 to 100 of 160/.test(pg2), `Next shows the following page with the same exact count (${pg2.trim()})`);
  await p.getByRole("button", { name: "Cancel" }).click().catch(() => {});
  ok(errors.length === 0, `no page errors ${errors.join(" | ").slice(0, 200)}`);
  await b.close();
  process.exit(failed ? 1 : 0);
})();
