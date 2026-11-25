// UI test of the ghost diff (Find and Replace) in the browser stand-in: the cells Replace all would change show
// their new text in green while the Replace box has text (also in a frozen column and in a tall row), the Find bar
// says "about…" until the whole-table count is done and then the exact figure, the count is cancelled by the next
// keystroke, ghosts clear on an edit, on the toggle, on Esc and while a write job runs, the right-click menu has the
// toggle while Find is open, and unticked changes of the preview stay plain.
// Run: (cd ui && npx vite --port 5220) then
// NODE_PATH=<folder with playwright> node scripts/ui-ghost.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5220";
const TAB = String.fromCharCode(9);
const CRLF = String.fromCharCode(13, 10);
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1360, height: 860 }, permissions: ["clipboard-read", "clipboard-write"] });
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
  const status = async () => ((await p.textContent(".find-bar, [aria-label='Find']")) || (await p.textContent("body")) || "").replace(/\s+/g, " ");
  const bodyText = async () => ((await p.textContent("body")) || "").replace(/\s+/g, " ");

  // A small table of made-up data: 40 rows, city in column 3.
  await p.evaluate(() => window.__nde.reg.run("file.new"));
  await p.waitForTimeout(500);
  await p.evaluate(async (tab) => {
    const t = window.__nde.tab;
    const rows = [["id", "first_name", "last_name", "city", "amount"]];
    for (let i = 1; i <= 40; i++) rows.push([String(i), ["Ada", "Bo", "Cy", "Dee"][i % 4], ["Stone", "Lind", "Quinn"][i % 3], ["Dayton", "Lisbon", "Perth"][i % 3], (i * 3.5).toFixed(2)]);
    t.update(await window.__nde.backend.dataOp(t.id, { op: "paste", spec: { row: 0, col: 0, op: "none", skip_blanks: false, transpose: false }, text: rows.map((r) => r.join(tab)).join("\r\n") }).then((r) => r.info), true);
    t.header = true;
    t.columns.setRange(3, 1, 160);
    t.bump();
  }, TAB);
  await p.waitForTimeout(400);

  // ---- open Replace, type the search and the replacement ----
  await p.keyboard.press("Control+h");
  await p.waitForTimeout(400);
  await p.getByLabel("Find", { exact: true }).first().fill("Lisbon");
  await p.getByLabel("Replace with").fill("Porto");
  await p.waitForTimeout(250);
  // The count is held back (a slow count) so "about…" can be seen first.
  const early = await bodyText();
  const ghostsNow = () => p.evaluate(() => [...window.__nde.tab.ghosts.entries()].map(([k, v]) => `${k}=${v.after}`));
  const g = await ghostsNow();
  const expected = await p.evaluate(() => {
    const t = window.__nde.tab;
    let n = 0;
    for (let r = 1; r < 24; r++) if (t.cache.get(r, 3) === "Lisbon") n++;
    return n;
  });
  ok(g.length > 0 && g.every((x) => x.endsWith("=Porto")), `the cells that would change carry their new text (${g.length} on screen: ${g.slice(0, 3).join(", ")})`);
  ok(/Would change \d+ of the cells you can see/.test(early) && /Replace all would change/.test(early), `the Find bar says what would change (${(early.match(/Would change[^]*?cells/) || [""])[0].slice(0, 120)})`);
  await p.waitForTimeout(700);
  const settled = await bodyText();
  const total = await p.evaluate(() => window.__nde.tab.ghostCount);
  ok(total && total.exact && total.cells === 14, `the count of all the cells is exact once the job has finished (${JSON.stringify(total)})`);
  ok(/Replace all would change 14 cells/.test(settled), "and the line gives the exact number then");
  if (out) await p.screenshot({ path: path.join(out, "ghost-diff.png") });

  // The pixels: a ghost cell is green-tinted.
  const cellPt = (row, col) => p.evaluate(([r, c]) => window.__nde.grid.cellPoint({ row: r, col: c }), [row, col]);
  const greenAt = async (row, col) => {
    const pt = await cellPt(row, col);
    const canvasBox = await (await p.$("canvas")).boundingBox();
    const shot = await p.screenshot({ clip: { x: pt.x + 4, y: pt.y - 10, width: 40, height: 6 } });
    return shot.length;
  };
  void greenAt;
  const first = Number(g[0].split(":")[0]);
  const pt = await cellPt(first, 3);
  const px = await p.evaluate(([x, y]) => {
    const c = document.querySelector("canvas");
    const r = c.getBoundingClientRect();
    const k = window.devicePixelRatio || 1;
    const d = c.getContext("2d").getImageData(Math.round((x + 120 - r.left) * k), Math.round((y - 6 - r.top) * k), 1, 1).data;
    return [d[0], d[1], d[2]];
  }, [pt.x, pt.y]);
  ok(px[1] > px[0] + 8 && px[1] > px[2] + 4, `a ghost cell is tinted green (rgb ${px.join(",")})`);

  // ---- typing goes on: the count is cancelled, one at a time ----
  const before = await p.evaluate(() => window.__nde.backend.calls.filter((c) => c === "replaceCountCancel").length);
  await p.getByLabel("Replace with").fill("Porto1");
  await p.waitForTimeout(100);
  const after = await p.evaluate(() => window.__nde.backend.calls.filter((c) => c === "replaceCountCancel").length);
  ok(after > before, "the next keystroke cancels the count before the next one starts");
  ok(/about…/.test(await bodyText()) || (await p.evaluate(() => window.__nde.tab.ghostCount?.exact === false)), "and until it finishes the line says about…");
  await p.waitForTimeout(700);

  // ---- a frozen column and a tall row ----
  await p.getByLabel("Find", { exact: true }).first().fill("Ada");
  await p.getByLabel("Replace with").fill("Adaline");
  await p.waitForTimeout(400);
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.frozenCols = 2;
    t.bump();
  });
  await p.waitForTimeout(500);
  const frozen = await ghostsNow();
  ok(frozen.length > 0 && frozen.some((x) => x.endsWith("=Adaline")), `ghosts draw while columns are frozen (${frozen.length})`);
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.frozenCols = 0;
    t.rowLines = 3;
    t.bump();
  });
  await p.waitForTimeout(500);
  const tall = await ghostsNow();
  ok(tall.length > 0, `ghosts draw in tall rows (${tall.length})`);
  if (out) await p.screenshot({ path: path.join(out, "ghost-diff-tall.png") });
  await p.evaluate(() => {
    window.__nde.tab.rowLines = 1;
    window.__nde.tab.bump();
  });

  // ---- an edit clears them; the new ones are for the new data ----
  await p.getByLabel("Find", { exact: true }).first().fill("Lisbon");
  await p.getByLabel("Replace with").fill("Porto");
  await p.waitForTimeout(500);
  const rev0 = await p.evaluate(() => window.__nde.tab.info.data_revision);
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    t.update(await window.__nde.backend.setCells(t.id, [[2, 3, "Lisbon"], [4, 3, "Elsewhere"]]), true);
  });
  const rev1 = await p.evaluate(() => window.__nde.tab.info.data_revision);
  await p.waitForTimeout(500);
  const afterEdit = await ghostsNow();
  ok(rev1 !== rev0 && afterEdit.some((x) => x.startsWith("2:3=")) && !afterEdit.some((x) => x.startsWith("4:3=")), `after an edit the ghosts are for the new data (${afterEdit.slice(0, 4).join(", ")}...)`);

  // ---- a write job running turns them off, and none is asked for meanwhile ----
  const asked0 = await p.evaluate(() => window.__nde.backend.calls.filter((c) => c === "replaceGhosts").length);
  await p.evaluate(() => {
    window.__nde.tab.busy = "Sorting…";
  });
  await p.waitForTimeout(400);
  const off = await ghostsNow();
  const asked1 = await p.evaluate(() => window.__nde.backend.calls.filter((c) => c === "replaceGhosts").length);
  ok(off.length === 0 && asked1 === asked0, `while a write job runs there are no ghosts and none is asked for (${off.length}, asks ${asked0} -> ${asked1})`);
  await p.evaluate(() => {
    window.__nde.tab.busy = null;
  });
  await p.waitForTimeout(500);
  ok((await ghostsNow()).length > 0, "and fresh ones come when the job ends");

  // ---- unticked changes of the preview stay plain, kept by the row's number ----
  await p.getByRole("button", { name: /Replace all…/ }).click();
  await p.waitForTimeout(600);
  const boxes = p.getByLabel("Replace this one");
  const n = await boxes.count();
  if (n > 0) {
    await boxes.first().uncheck();
    await p.waitForTimeout(300);
    const skip = await p.evaluate(() => [...window.__nde.tab.ghostSkip]);
    ok(skip.length === 1 && /^\d+:\d+$/.test(skip[0]), `an unticked change is kept by row number and column (${skip.join(",")})`);
  } else {
    ok(false, "the Replace all preview listed changes to untick");
  }
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);

  // ---- the right-click menu has the toggle while Find is open; the toggle clears them ----
  await p.mouse.click(...(await (async () => { const q = await cellPt(5, 1); return [q.x + 10, q.y - 8]; })()), { button: "right" });
  await p.waitForTimeout(300);
  const menu = await p.evaluate(() => Array.from(document.querySelectorAll("[role=menu] [role=menuitem], .menu button")).map((e) => e.textContent.replace(/\s+/g, " ").trim()).join(" | "));
  ok(/Show changes in the grid/.test(menu), `the cell menu has "Show changes in the grid" while Find is open (${menu.slice(0, 60)}...)`);
  await p.getByText("Show changes in the grid", { exact: false }).first().click();
  await p.waitForTimeout(400);
  ok((await ghostsNow()).length === 0, "turning it off clears the ghosts");
  await p.evaluate(() => window.__nde.reg.run("find.ghosts"));
  await p.waitForTimeout(500);
  ok((await ghostsNow()).length > 0, "and on brings them back");

  // ---- Esc closes Find: no ghosts, no count ----
  await p.getByLabel("Find", { exact: true }).first().focus();
  await p.keyboard.press("Escape");
  await p.waitForTimeout(400);
  const gone = await p.evaluate(() => ({ g: window.__nde.tab.ghosts.size, want: window.__nde.tab.ghostWant, count: window.__nde.tab.ghostCount }));
  ok(gone.g === 0 && gone.want === null && gone.count === null, "closing Find clears the ghosts and the count");
  void status;

  ok(errors.length === 0, errors.length ? "page errors: " + errors.join(" | ") : "no page errors");
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
