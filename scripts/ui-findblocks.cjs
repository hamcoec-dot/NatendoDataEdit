// UI test of Find in several Ctrl+click blocks against the vite dev server (browser
// stub backend): the panel searches all the blocks (Selection), Find next walks them in
// reading order and wraps while all the blocks stay selected, the count agrees, Replace all
// changes only cells of the blocks and one Ctrl+Z puts them back, a cell in two blocks is
// counted once, and the blocks are let go (with a message) when the shown rows change.
// Run: (cd ui && npx vite --port 5217) then
// NODE_PATH=<folder with playwright> node scripts/ui-findblocks.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5217";
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
  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => !/Loading|Indexing|Reading/i.test(document.querySelector("footer.status")?.textContent || ""), null, { timeout: 30000 }).catch(() => {});
  await p.waitForTimeout(800);

  const canvas = await (await p.$("canvas")).boundingBox();
  const HEAD = 25;
  const ROW = 25;
  let left = 0;
  for (let x = 30; x < 120; x += 2) {
    await p.mouse.click(canvas.x + x, canvas.y + HEAD + ROW + 12);
    if ((await p.evaluate(() => window.__nde.tab.areas.length)) === 1 && /C1$/.test(await p.inputValue("input.goto"))) {
      left = x;
      break;
    }
  }
  ok(left > 0, `row numbers end at ${left}px`);
  const W = 130;
  const cx = (c) => canvas.x + left + c * W + W / 2;
  const ry = (r) => canvas.y + HEAD + r * ROW + ROW / 2;
  const areas = () => p.evaluate(() => window.__nde.tab.areas.map((a) => ({ ...a })));
  const drag = async (x0, y0, x1, y1, ctrl) => {
    if (ctrl) await p.keyboard.down("Control");
    await p.mouse.move(x0, y0);
    await p.mouse.down();
    await p.mouse.move(x1, y1, { steps: 6 });
    await p.mouse.up();
    if (ctrl) await p.keyboard.up("Control");
  };
  /** The text of every cell in rows 0..39, columns 0..5 (the first screens). */
  const snapshot = () =>
    p.evaluate(async () => {
      const t = window.__nde.tab;
      const g = [];
      for (let r = 0; r < 40; r++) {
        const row = [];
        for (let c = 0; c < 6; c++) row.push((await window.__nde.backend.cellText(t.id, r, c)).text);
        g.push(row);
      }
      return g;
    });
  const pick = async () => {
    // Three blocks, the last overlapping the first.
    await p.mouse.click(cx(0), ry(14)); // (reset to one block)
    await drag(cx(2), ry(1), cx(3), ry(8), false);
    await drag(cx(1), ry(14), cx(1), ry(20), true);
    await drag(cx(3), ry(5), cx(4), ry(11), true);
    return areas();
  };
  const inUnion = (blocks, r, c) => blocks.some((a) => r >= a.row && r < a.row + a.rows && c >= a.col && c < a.col + a.cols);
  const PAT = /e/i;

  // ---- select blocks and open Find ----
  const blocks = await pick();
  ok(blocks.length === 3, `three blocks selected (${JSON.stringify(blocks)})`);
  const grid0 = await snapshot();
  const cells = [];
  for (let r = 0; r < 40; r++) for (let c = 0; c < 6; c++) if (inUnion(blocks, r, c)) cells.push([r, c]);
  const matches = cells.filter(([r, c]) => PAT.test(grid0[r][c]));
  ok(matches.length >= 4, `the blocks hold ${matches.length} matching cells (${cells.length} cells in the union)`);
  await p.keyboard.press("Control+f");
  await p.waitForSelector('select[aria-label="Search in"]', { timeout: 5000 });
  const scopeText = await p.evaluate(() => {
    const s = document.querySelector('select[aria-label="Search in"]');
    return { value: s.value, text: s.options[s.selectedIndex].textContent };
  });
  ok(scopeText.value === "selection" && /3 blocks/.test(scopeText.text) && new RegExp(`${cells.length.toLocaleString()} cells`).test(scopeText.text), `Find opens on Selection and names the blocks (${scopeText.text})`);
  ok(!(await p.$(".needone")), "no 'works on one block' refusal");
  await p.fill('input[aria-label="Find"], input[placeholder*="ind"]', "e").catch(async () => {
    await p.keyboard.type("e");
  });
  if (!(await p.inputValue('input[aria-label="Find"], input[placeholder*="ind"]').catch(() => "e")).includes("e")) await p.keyboard.type("e");
  await p.waitForTimeout(700);

  // ---- Find next walks the blocks in reading order and wraps, the blocks stay selected ----
  const start = await p.evaluate(() => ({ ...window.__nde.tab.cell }));
  const key = (r, c) => r * 100 + c;
  const order = matches.map(([r, c]) => [r, c]);
  const expected = [];
  let cur = key(start.row, start.col);
  for (let i = 0; i < order.length + 2; i++) {
    const nxt = order.find(([r, c]) => key(r, c) > cur) ?? order[0];
    expected.push(nxt);
    cur = key(nxt[0], nxt[1]);
  }
  const got = [];
  let blocksStayed = true;
  for (let i = 0; i < expected.length; i++) {
    await p.keyboard.press("F3");
    await p.waitForTimeout(250);
    got.push(await p.evaluate(() => [window.__nde.tab.cell.row, window.__nde.tab.cell.col]));
    const now = await areas();
    if (now.length !== 3) blocksStayed = false;
  }
  ok(JSON.stringify(got) === JSON.stringify(expected), `F3 visits the matches of all the blocks in reading order and wraps (${JSON.stringify(got)} vs ${JSON.stringify(expected)})`);
  ok(blocksStayed, "all three blocks stay selected while Find next moves the active cell");
  const countText = await p.evaluate(() => document.querySelector(".count")?.textContent || "");
  ok(countText.includes(String(matches.length)), `the count is ${matches.length} (${countText})`);
  if (out) await p.screenshot({ path: path.join(out, "findblocks.png") });

  // ---- Replace all: only the cells of the blocks, one Undo ----
  await p.keyboard.press("Control+h");
  await p.waitForTimeout(400);
  const rep = p.locator('input[aria-label="Replace with"], input[placeholder*="eplace"]').first();
  await rep.fill("XY");
  await p.getByRole("button", { name: "Replace all…" }).click();
  await p.waitForTimeout(500);
  await p.getByRole("button", { name: "Replace all", exact: true }).click();
  await p.waitForTimeout(900);
  const after = await snapshot();
  let wrong = 0;
  for (let r = 0; r < 40; r++)
    for (let c = 0; c < 6; c++) {
      const want = inUnion(blocks, r, c) ? grid0[r][c].replace(/e/gi, "XY") : grid0[r][c];
      if (after[r][c] !== want) wrong++;
    }
  ok(wrong === 0, `Replace all changed exactly the matching cells of the blocks (${wrong} wrong of 240)`);
  const outside = [];
  for (let r = 0; r < 40; r++) for (let c = 0; c < 6; c++) if (!inUnion(blocks, r, c) && PAT.test(grid0[r][c])) outside.push([r, c]);
  ok(outside.length > 0 && outside.every(([r, c]) => after[r][c] === grid0[r][c]), `${outside.length} matching cells outside the blocks were left alone`);
  await p.keyboard.press("Escape");
  await p.mouse.click(cx(5), ry(25));
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(700);
  const undone = await snapshot();
  ok(JSON.stringify(undone) === JSON.stringify(grid0), "one Ctrl+Z puts every cell back");

  // ---- the blocks are let go when the shown rows change ----
  const blocks2 = await pick();
  await p.keyboard.press("Control+f");
  await p.waitForSelector('select[aria-label="Search in"]', { timeout: 5000 });
  await p.waitForTimeout(300);
  const before = await p.evaluate(() => document.querySelector('select[aria-label="Search in"]').value);
  // (A click makes one block again, as it does in use; Find keeps the blocks it was given.)
  await p.mouse.click(cx(5), ry(25));
  const fe = await p.evaluate(async () => {
    try {
      const t = window.__nde.tab;
      t.sel = { anchor: { row: 20, col: 5 }, focus: { row: 20, col: 5 } };
      const values = Array.from({ length: 30 }, (_, i) => String(i + 1));
      t.update(await window.__nde.backend.setFilter(t.id, { columns: [{ col: 0, rule: { type: "values", values, blanks: false } }], keep_first: true }), true);
      return "";
    } catch (e) {
      return JSON.stringify(e);
    }
  });
  if (fe) console.log("setFilter error: " + fe);
  await p.waitForTimeout(600);
  const scopeAfter = await p.evaluate(() => document.querySelector('select[aria-label="Search in"]').value);
  const st = await p.evaluate(() => document.querySelector(".findbar, .find, [role=search]")?.textContent || document.body.textContent || "");
  ok(before === "selection" && scopeAfter === "all", `a filter while Find is open lets the blocks go and searches the whole file (${before} -> ${scopeAfter})`);
  ok(/selected blocks were let go/.test(st), "and says so");
  void blocks2;

  // ---- one block still works as before ----
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    t.update(await window.__nde.backend.clearFilter(t.id), true);
  });
  await p.waitForTimeout(400);
  await p.keyboard.press("Escape");
  await p.evaluate(() => {
    window.__nde.tab.sel = { anchor: { row: 2, col: 2 }, focus: { row: 8, col: 3 } };
  });
  await p.keyboard.press("Control+f");
  await p.waitForSelector('select[aria-label="Search in"]', { timeout: 5000 });
  await p.waitForTimeout(300);
  const one = await p.evaluate(() => {
    const s = document.querySelector('select[aria-label="Search in"]');
    return { value: s.value, text: s.options[s.selectedIndex].textContent };
  });
  ok(one.value === "selection" && !/blocks/.test(one.text) && /Selection C\d+:D\d+/.test(one.text), `one block: Selection names its cells as before (${one.text})`);

  ok(errors.length === 0, errors.length ? "page errors: " + errors.join(" | ") : "no page errors");
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
