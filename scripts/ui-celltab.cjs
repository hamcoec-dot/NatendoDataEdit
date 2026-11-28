// UI test of dragging selected cells onto another tab (copy, Shift: move), the
// "Copy/Move selection to another file" commands, and the refusals, against the
// vite dev server (browser stub backend).
// Run: (cd ui && npx vite --port 5213) then
// NODE_PATH=<folder with playwright> node scripts/ui-celltab.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5213";
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
  const status = async () => ((await p.textContent("footer.status .msg").catch(() => "")) || "").replace(/\s+/g, " ");
  const open = async () => {
    await p.keyboard.press("Control+o");
    await p.waitForFunction(() => !/Loading|Indexing|Reading/i.test(document.querySelector("footer.status")?.textContent || ""), null, { timeout: 30000 }).catch(() => {});
    await p.waitForTimeout(700);
  };
  await open();
  await open();
  const ids = await p.evaluate(() => window.__nde.tabs.map((t) => t.id));
  ok(ids.length === 2, `two tabs open (${ids})`);
  const tabBox = async (i) => p.locator(`[data-tab-index="${i}"]`).boundingBox();
  const showTab = async (i) => {
    const bx = await tabBox(i);
    await p.mouse.click(bx.x + bx.width / 2 - 8, bx.y + bx.height / 2);
    await p.waitForTimeout(400);
  };
  // Text of a block: line breaks as LF, none at the end.
  const block = async (id, row, rows, col, cols) =>
    (await p.evaluate(([id, row, rows, col, cols]) => window.__nde.backend.copyCells(id, row, rows, col, cols), [id, row, rows, col, cols])).replace(/\r/g, "").replace(/\n$/, "");
  const rectOf = (i) => p.evaluate((i) => ({ ...window.__nde.tabs[i].drawRect }), i);
  const undo = async () => {
    await p.keyboard.press("Control+z");
    await p.waitForTimeout(400);
  };

  // Make the two tabs easy to tell apart: one marked block in tab 1, and give tab 2 its own selected cell.
  await p.evaluate(async (id) => {
    await window.__nde.backend.setCells(id, [[2, 1, "AAA"], [2, 2, "BBB"], [3, 1, "CCC"], [3, 2, "DDD"]]);
  }, ids[0]);
  await showTab(1);
  await p.locator("canvas").click({ position: { x: 300, y: 130 } });
  const dstRect = await rectOf(1);
  console.log("INFO destination cell", JSON.stringify(dstRect));
  await showTab(0);

  // ---- geometry: where the grid's cells are ----
  const canvas = await (await p.$("canvas")).boundingBox();
  const HEAD = 25;
  const ROW = 25;
  let left = 0;
  for (let x = 30; x < 120; x += 2) {
    await p.mouse.click(canvas.x + x, canvas.y + HEAD + ROW + 12);
    if ((await p.evaluate(() => window.__nde.tab.drawRect.cols)) === 1 && /C1$/.test(await p.inputValue("input.goto"))) {
      left = x;
      break;
    }
  }
  ok(left > 0, `row numbers end at ${left}px`);
  const W = 130;
  const cx = (c) => canvas.x + left + c * W + W / 2;
  const ry = (r) => canvas.y + HEAD + r * ROW + ROW / 2;
  const selectBlock = async () => {
    await p.mouse.click(cx(1), ry(1));
    await p.keyboard.down("Shift");
    await p.mouse.click(cx(2), ry(2));
    await p.keyboard.up("Shift");
  };
  const edgeX = canvas.x + left + W + 1; // the block's left outline
  const dragTo = async (i, opts = {}) => {
    const bx = await tabBox(i);
    await p.mouse.move(edgeX, ry(1));
    await p.mouse.down();
    await p.mouse.move(edgeX + 8, ry(1), { steps: 2 });
    await p.mouse.move(bx.x + bx.width / 2 - 8, bx.y + bx.height / 2, { steps: 8 });
    if (opts.shot) await p.screenshot({ path: path.join(out, opts.shot) });
    const over = await p.locator(".tab.drop").count();
    if (opts.back) {
      await p.mouse.move(cx(4), ry(8), { steps: 6 });
      opts.overBack = await p.locator(".tab.drop").count();
    }
    if (opts.esc) await p.keyboard.press("Escape");
    if (opts.shift) await p.keyboard.down("Shift");
    await p.mouse.up();
    if (opts.shift) await p.keyboard.up("Shift");
    await p.waitForTimeout(500);
    return over;
  };

  const src = await block(ids[0], 2, 2, 1, 2);
  ok(src === "AAA\tBBB\nCCC\tDDD" || src.replace(/\r/g, "") === "AAA\tBBB\nCCC\tDDD", `source block is the marked one (${JSON.stringify(src)})`);

  // ---- copy ----
  await selectBlock();
  let over = await dragTo(1, { shot: "celltab-over.png" });
  ok(over === 1, "the tab under the pointer is outlined while dragging");
  const got = await block(ids[1], dstRect.row, 2, dstRect.col, 2);
  ok(got.replace(/\r/g, "") === src.replace(/\r/g, ""), `the block landed at the other tab's selected cell (${JSON.stringify(got)})`);
  ok((await block(ids[0], 2, 2, 1, 2)).replace(/\r/g, "") === src.replace(/\r/g, ""), "and the source still has it (a copy)");
  ok(/Copied 4 cells to .*Undo there with Ctrl\+Z/.test(await status()), `toast says so (${await status()})`);
  ok((await p.getByRole("button", { name: /^Go to / }).count()) === 1, "with a Go to link");
  ok((await p.evaluate(() => window.__nde.tabs.indexOf(window.__nde.tab))) === 0, "and the person stays on the source tab");
  ok((await p.locator(".tab.drop").count()) === 0, "the outline is gone after the drop");
  await p.getByRole("button", { name: /^Go to / }).click();
  await p.waitForTimeout(400);
  ok((await p.evaluate(() => window.__nde.tabs.indexOf(window.__nde.tab))) === 1, "Go to takes the person to the other tab");
  await undo();
  ok((await block(ids[1], dstRect.row, 2, dstRect.col, 2)).replace(/\r/g, "") !== src.replace(/\r/g, ""), "Ctrl+Z there undoes it");
  await showTab(0);

  // ---- back over the grid clears the outline; Esc cancels ----
  await selectBlock();
  const opts = { back: true, esc: true };
  await dragTo(1, opts);
  ok(opts.overBack === 0, "moving back over the grid takes the outline off the tab");
  const before = await block(ids[1], dstRect.row, 2, dstRect.col, 2);
  await selectBlock();
  await dragTo(1, { esc: true });
  ok((await block(ids[1], dstRect.row, 2, dstRect.col, 2)) === before && (await p.locator(".tab.drop").count()) === 0, "Esc mid-drag drops nothing");
  // Dropping on the source tab does nothing.
  await selectBlock();
  const said = await status();
  const histBefore = await p.evaluate(() => window.__nde.tab.info.history_pos);
  await dragTo(0);
  ok((await status()) === said && (await p.evaluate(() => window.__nde.tab.info.history_pos)) === histBefore, "dropping on the source tab does nothing special");

  // ---- Shift: move ----
  await showTab(0);
  await selectBlock();
  const d2 = await rectOf(1);
  await dragTo(1, { shift: true });
  ok(/Moved 4 cells to .*Ctrl\+Z in each file undoes its half/.test(await status()), `Shift-drop moves (${await status()})`);
  ok((await block(ids[1], d2.row, 2, d2.col, 2)).replace(/\r/g, "") === src.replace(/\r/g, ""), "the other tab has the cells");
  ok(/^\s*\t\s*\n\s*\t\s*$|^\t\n\t$/.test((await block(ids[0], 2, 2, 1, 2)).replace(/\r/g, "")), "and the source is cleared");
  await undo();
  ok((await block(ids[0], 2, 2, 1, 2)).replace(/\r/g, "") === src.replace(/\r/g, ""), "Ctrl+Z in the source puts its cells back (step one)");
  await showTab(1);
  await undo();
  ok((await block(ids[1], d2.row, 2, d2.col, 2)).replace(/\r/g, "") !== src.replace(/\r/g, ""), "Ctrl+Z in the other file takes them out (step two)");
  await showTab(0);

  // ---- a source that changed after the copy is not cleared ----
  await p.evaluate((sid) => {
    const bk = window.__nde.backend;
    const orig = bk.copyCells.bind(bk);
    bk.copyCells = async (...a) => {
      const text = await orig(...a);
      if (a[0] === sid) {
        bk.copyCells = orig;
        const info = await bk.setCells(sid, [[9, 5, "changed meanwhile"]]);
        window.__nde.tabs.find((t) => t.id === sid).update(info, true);
      }
      return text;
    };
  }, ids[0]);
  await selectBlock();
  await dragTo(1, { shift: true });
  ok(/Copied, not moved: .*not removed from .*changed meanwhile/.test(await status()), `a source changed after the copy keeps its cells (${await status()})`);
  ok((await block(ids[0], 2, 2, 1, 2)).replace(/\r/g, "") === src.replace(/\r/g, ""), "the source cells are still there");
  await p.evaluate(() => {});

  // ---- edit, undo, a different edit while the move runs: same undo position, other data: not cleared ----
  await p.evaluate(async (sid) => {
    const t = window.__nde.tabs.find((x) => x.id === sid);
    t.update(await window.__nde.backend.setCells(sid, [[11, 5, "x0"]]), true);
  }, ids[0]);
  await p.evaluate((sid) => {
    const bk = window.__nde.backend;
    const orig = bk.copyCells.bind(bk);
    bk.copyCells = async (...a) => {
      const text = await orig(...a);
      if (a[0] === sid) {
        bk.copyCells = orig;
        const t = window.__nde.tabs.find((x) => x.id === sid);
        window.__pos = t.info.history_pos;
        t.update(await bk.change(sid, { op: "undo" }), true);
        t.update(await bk.setCells(sid, [[12, 5, "y"]]), true);
        window.__posAfter = t.info.history_pos;
      }
      return text;
    };
  }, ids[0]);
  await selectBlock();
  await dragTo(1, { shift: true });
  const same = await p.evaluate(() => window.__pos === window.__posAfter);
  ok(same, "(set up: the undo position is the same again after edit, undo, edit)");
  ok(/Copied, not moved: .*not removed from .*changed meanwhile/.test(await status()), `a different edit made during the move keeps the source cells (${await status()})`);
  ok((await block(ids[0], 2, 2, 1, 2)) === src, "the source cells were not cleared");

  // ---- a clear that fails says copied, not removed ----
  await p.evaluate((sid) => {
    const bk = window.__nde.backend;
    const orig = bk.fillCells.bind(bk);
    bk.fillCells = async (...a) => {
      if (a[0] === sid) {
        bk.fillCells = orig;
        throw { kind: "x", message: "disk said no" };
      }
      return orig(...a);
    };
  }, ids[0]);
  await selectBlock();
  await dragTo(1, { shift: true });
  ok(/Copied, not moved: 4 cells went to .*not removed from .*could not be cleared/.test(await status()), `a failed clear says copied, not removed (${await status()})`);

  // ---- the commands (keyboard and menu path) and the refusals ----
  await showTab(0);
  await selectBlock();
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("copy selection to another");
  await p.waitForTimeout(300);
  await p.keyboard.press("Enter");
  await p.waitForTimeout(500);
  const dlg = p.locator(".dialog[role=alertdialog]");
  ok(/Copy the selected cells to which file/.test((await dlg.textContent().catch(() => "")) || ""), "the command asks which file");
  if (out) await p.screenshot({ path: path.join(out, "celltab-picker.png") });
  await dlg.getByRole("button", { name: /sample-1M/ }).first().click();
  await p.waitForTimeout(500);
  ok(/Copied 4 cells to/.test(await status()), "choosing a file copies there");
  await p.evaluate(() => (window.__nde.tab.private = { file: true, folder: false }));
  await selectBlock();
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("copy selection to another");
  await p.waitForTimeout(300);
  await p.keyboard.press("Enter");
  await p.waitForTimeout(500);
  await dlg.getByRole("button", { name: /sample-1M/ }).first().click();
  await p.waitForTimeout(500);
  ok(/is private .*not copied into/.test(await status()), `a private source is refused into a file that is not (${await status()})`);
  await p.evaluate(() => (window.__nde.tabs[1].private = { file: true, folder: false }));
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("copy selection to another");
  await p.waitForTimeout(300);
  await p.keyboard.press("Enter");
  await p.waitForTimeout(500);
  await dlg.getByRole("button", { name: /sample-1M/ }).first().click();
  await p.waitForTimeout(500);
  ok(/Copied 4 cells to/.test(await status()), "private into private is allowed");
  await p.evaluate(() => {
    window.__nde.tab.private = { file: false, folder: false };
    window.__nde.tabs[1].private = { file: false, folder: false };
    window.__nde.tab.sel = { anchor: { row: 1, col: 0 }, focus: { row: 100000, col: 1 } };
  });
  await p.waitForTimeout(300);
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("copy selection to another");
  await p.waitForTimeout(300);
  await p.keyboard.press("Enter");
  await p.waitForTimeout(500);
  await dlg.getByRole("button", { name: /sample-1M/ }).first().click();
  await p.waitForTimeout(500);
  ok(/up to 100,000 cells/.test(await status()), `over the cap is refused (${await status()})`);

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
