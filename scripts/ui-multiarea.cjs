// UI test of Ctrl+click multi-area selection against the vite dev server
// (browser stub backend): the gestures with the real mouse, the fail-closed
// registry test (every command, with 2 areas and with 10,000), the write guard
// behind the commands, and the actions that work on several blocks.
// Run: (cd ui && npx vite --port 5214) then
// NODE_PATH=<folder with playwright> node scripts/ui-multiarea.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5214";
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
  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => !/Loading|Indexing|Reading/i.test(document.querySelector("footer.status")?.textContent || ""), null, { timeout: 30000 }).catch(() => {});
  await p.waitForTimeout(800);

  // ---- geometry ----
  const canvas = await (await p.$("canvas")).boundingBox();
  const HEAD = 25;
  const ROW = 25;
  let left = 0;
  for (let x = 30; x < 120; x += 2) {
    await p.mouse.click(canvas.x + x, canvas.y + HEAD + ROW + 12);
    if ((await p.evaluate(() => window.__nde.tab.areas.length)) === 1 && (await p.evaluate(() => window.__nde.tab.drawRect.cols)) === 1 && /C1$/.test(await p.inputValue("input.goto"))) {
      left = x;
      break;
    }
  }
  ok(left > 0, `row numbers end at ${left}px`);
  const W = 130;
  const cx = (c) => canvas.x + left + c * W + W / 2;
  const ry = (r) => canvas.y + HEAD + r * ROW + ROW / 2;
  const areas = () => p.evaluate(() => window.__nde.tab.areas.map((a) => ({ ...a })));
  const ctrlClick = async (x, y) => {
    await p.keyboard.down("Control");
    await p.mouse.click(x, y);
    await p.keyboard.up("Control");
  };
  const ctrlDrag = async (x0, y0, x1, y1) => {
    await p.keyboard.down("Control");
    await p.mouse.move(x0, y0);
    await p.mouse.down();
    await p.mouse.move(x1, y1, { steps: 6 });
    await p.mouse.up();
    await p.keyboard.up("Control");
  };

  // ---- gestures ----
  await p.mouse.click(cx(0), ry(1));
  await ctrlClick(cx(2), ry(3));
  let a = await areas();
  ok(a.length === 2 && a[0].row === 2 && a[0].col === 0 && a[1].row === 4 && a[1].col === 2, `Ctrl+click adds a block (${JSON.stringify(a)})`);
  ok((await p.evaluate(() => window.__nde.tab.cell.col)) === 2, "the active cell is the last one picked");
  await ctrlDrag(cx(0), ry(6), cx(1), ry(7));
  a = await areas();
  ok(a.length === 3 && a[2].rows === 2 && a[2].cols === 2, `Ctrl+drag adds a block (${JSON.stringify(a[2])})`);
  // The header: Ctrl+click on a column heading adds the whole column.
  await ctrlClick(cx(4), canvas.y + HEAD / 2);
  a = await areas();
  ok(a.length === 4 && a[3].col === 4 && a[3].rows > 1000, "Ctrl+click on a column heading adds the whole column");
  // Shift+click extends the last block only.
  await p.keyboard.down("Shift");
  await p.mouse.click(cx(6), ry(2));
  await p.keyboard.up("Shift");
  a = await areas();
  ok(a.length === 4 && a[0].cols === 1 && a[3].col >= 4, "Shift+click moves the last block's far corner and keeps the others");
  if (out) await p.screenshot({ path: path.join(out, "multiarea.png") });
  // The earlier blocks are drawn (their tint is in the canvas) and totals cover all.
  await p.waitForTimeout(800);
  const totals = await p.locator("footer.status").textContent();
  ok(/Count/.test(totals ?? ""), `the status bar shows totals for all blocks (${(totals ?? "").replace(/\s+/g, " ").slice(0, 80)})`);
  // Plain click: one block again.
  await p.mouse.click(cx(1), ry(1));
  ok((await areas()).length === 1, "a plain click goes back to one block");
  // Esc clears the earlier blocks.
  await p.mouse.click(cx(0), ry(1));
  await ctrlClick(cx(2), ry(3));
  await p.keyboard.press("Escape");
  ok((await areas()).length === 1, "Esc goes back to the last block alone");
  // Selecting again by keys clears them.
  await p.mouse.click(cx(0), ry(1));
  await ctrlClick(cx(2), ry(3));
  await p.keyboard.press("ArrowDown");
  ok((await areas()).length === 1, "a plain arrow key picks one block again");

  // ---- refused gestures and actions (message, nothing changes) ----
  const snap = () =>
    p.evaluate(async () => {
      const t = window.__nde.tab;
      return { text: await window.__nde.backend.copyCells(t.id, 0, 60, 0, 7), pos: t.info.history_pos, rev: t.info.data_revision ?? 0 };
    });
  await p.mouse.click(cx(0), ry(1));
  await ctrlClick(cx(2), ry(3));
  const before = await snap();
  // Fill handle is not offered; drag-to-move of the outline is refused (nothing grabs).
  await p.mouse.move(cx(2) + W / 2 - 1, ry(3) + ROW / 2 - 1);
  const cur = await p.$eval("[aria-label='Data grid'], .grid", (e) => getComputedStyle(e).cursor).catch(() => "");
  ok(cur !== "crosshair", `no fill handle while several blocks are selected (${cur})`);
  await p.keyboard.press("Control+d");
  await p.waitForTimeout(300);
  ok(/works on one block/i.test(await status()), `Fill down is refused with the one-block message (${await status()})`);
  const after = await snap();
  ok(JSON.stringify(after) === JSON.stringify(before), "and nothing changed");

  // ---- the actions that work on several blocks ----
  // Clear (Delete) over two blocks is one undo step.
  await p.mouse.click(cx(0), ry(1)); // one block again: the test's own writes would be refused otherwise
  const pos0 = (await snap()).pos;
  const wrote = await p.evaluate(async () => {
    const t = window.__nde.tab;
    await window.__nde.backend.setCells(t.id, [[2, 0, "aa"], [4, 2, "bb"], [3, 1, "keep"]]);
    t.update(await window.__nde.backend.docInfo(t.id), true);
    return t.info.history_pos;
  });
  await p.mouse.click(cx(0), ry(1));
  await ctrlClick(cx(2), ry(3));
  await p.keyboard.press("Delete");
  await p.waitForTimeout(500);
  const cells = await p.evaluate(async () => {
    const t = window.__nde.tab;
    const g = async (r, c) => (await window.__nde.backend.copyCells(t.id, r, 1, c, 1)).replace(/[\r\n]+$/, "");
    return [await g(2, 0), await g(4, 2), await g(3, 1), t.info.history_pos];
  });
  ok(cells[0] === "" && cells[1] === "" && cells[2] === "keep", `Delete clears both blocks and nothing else (${cells.slice(0, 3)})`);
  ok(cells[3] === wrote + 1, "as one undo step");
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(400);
  const undone = await p.evaluate(async () => {
    const t = window.__nde.tab;
    const g = async (r, c) => (await window.__nde.backend.copyCells(t.id, r, 1, c, 1)).replace(/[\r\n]+$/, "");
    return [await g(2, 0), await g(4, 2)];
  });
  ok(undone[0] === "aa" && undone[1] === "bb", "one Ctrl+Z puts both blocks back");
  void pos0;
  // Ctrl+Enter fills one value into both blocks (typing writes the active cell only).
  await p.mouse.click(cx(0), ry(1));
  await ctrlClick(cx(2), ry(3));
  await p.keyboard.type("zz");
  await p.keyboard.press("Control+Enter");
  await p.waitForTimeout(500);
  const filled = await p.evaluate(async () => {
    const t = window.__nde.tab;
    const g = async (r, c) => (await window.__nde.backend.copyCells(t.id, r, 1, c, 1)).replace(/[\r\n]+$/, "");
    return [await g(2, 0), await g(4, 2), await g(3, 1)];
  });
  ok(filled[0] === "zz" && filled[1] === "zz" && filled[2] === "keep", `Ctrl+Enter fills the value into every block (${filled})`);
  // Copy: blocks that do not line up are refused with a message.
  await p.mouse.click(cx(0), ry(1));
  await ctrlClick(cx(2), ry(3));
  await p.keyboard.press("Control+c");
  await p.waitForTimeout(300);
  ok(/line up/i.test(await status()), `Copy of blocks that do not line up says so (${await status()})`);
  // Blocks with the same rows line up: copied side by side.
  await p.mouse.click(cx(0), ry(1));
  await ctrlClick(cx(2), ry(1));
  await p.keyboard.press("Control+c");
  await p.waitForTimeout(400);
  const clip = await p.evaluate(() => navigator.clipboard.readText());
  ok(/Copied 2 cells from 2 blocks/.test(await status()) && clip.split("\t").length === 2, `blocks with the same rows copy side by side (${JSON.stringify(clip)})`);

  // ---- paste: one value goes into every block; more than one value is refused ----
  await p.mouse.click(cx(0), ry(1));
  await ctrlClick(cx(2), ry(3));
  const valueAt = (r, c) => p.evaluate(async ([r, c]) => (await window.__nde.backend.copyCells(window.__nde.tab.id, r, 1, c, 1)).replace(/[\r\n]+$/, ""), [r, c]);
  const beforePaste = [await valueAt(2, 0), await valueAt(4, 2)];
  await p.evaluate(() => navigator.clipboard.writeText("one\ttwo"));
  await p.keyboard.press("Control+v");
  await p.waitForTimeout(500);
  ok(/more than one value works on one block/.test(await status()), `pasting more than one value is refused (${await status()})`);
  ok(JSON.stringify([await valueAt(2, 0), await valueAt(4, 2)]) === JSON.stringify(beforePaste) && (await areas()).length === 2, "and nothing changed, the blocks stay selected");
  await p.evaluate(() => navigator.clipboard.writeText("vv\nww"));
  await p.keyboard.press("Control+v");
  await p.waitForTimeout(500);
  ok(/more than one value works on one block/.test(await status()) && JSON.stringify([await valueAt(2, 0), await valueAt(4, 2)]) === JSON.stringify(beforePaste), "two lines are refused too");
  await p.evaluate(() => navigator.clipboard.writeText("single"));
  await p.keyboard.press("Control+v");
  await p.waitForTimeout(600);
  ok((await valueAt(2, 0)) === "single" && (await valueAt(4, 2)) === "single", "one value pastes into every block");

  // ---- Find in a selection with several blocks searches all of them ----
  await p.mouse.click(cx(0), ry(1));
  await p.keyboard.down("Shift");
  await p.mouse.click(cx(1), ry(2));
  await p.keyboard.up("Shift");
  await p.keyboard.press("Control+f");
  await p.waitForTimeout(300);
  await p.keyboard.type("single");
  await p.locator("select[aria-label='Search in']").selectOption("selection");
  await ctrlClick(cx(3), ry(4));
  await p.waitForTimeout(300);
  const sel1 = JSON.stringify(await areas());
  // Blocks picked after Find opened are not searched until Ctrl+F takes them: nothing is quietly widened.
  await p.getByRole("button", { name: "Find next" }).click();
  await p.waitForTimeout(400);
  const needText = await p.locator(".find").first().textContent();
  ok(/selected after Find opened/.test(needText ?? "") && JSON.stringify(await areas()) === sel1, "blocks picked after Find opened are not searched, and it says so");
  await p.keyboard.press("Control+f");
  await p.waitForTimeout(300);
  const scopeText = await p.evaluate(() => {
    const s = document.querySelector("select[aria-label='Search in']");
    return s.options[s.selectedIndex].textContent;
  });
  ok(/2 blocks/.test(scopeText), `Ctrl+F takes the blocks: Selection names them (${scopeText})`);
  const optDisabled = await p.locator("select[aria-label='Search in'] option[value='column']").isDisabled();
  ok(!optDisabled, "and 'Selected columns' can be picked");
  await p.getByRole("button", { name: "Find next" }).click();
  await p.waitForTimeout(500);
  const hit = await p.evaluate(() => [window.__nde.tab.cell.row, window.__nde.tab.cell.col, window.__nde.tab.areas.length]);
  ok(hit[0] === 2 && hit[1] === 0 && hit[2] === 2, `Find next finds the hit inside the blocks and keeps both selected (${JSON.stringify(hit)})`);
  await p.locator("select[aria-label='Search in']").selectOption("all");
  await p.getByRole("button", { name: "Find next" }).click();
  await p.waitForTimeout(500);
  ok((await areas()).length === 1, "Whole file searches, and the hit selects one cell (the earlier blocks go)");
  await p.keyboard.press("Escape");

  // ---- Ctrl+F with several blocks searches them (the scope is Selection) ----
  await p.mouse.click(cx(0), ry(5));
  await ctrlClick(cx(2), ry(6));
  await p.keyboard.press("Control+f");
  await p.waitForTimeout(400);
  const scopeNow = await p.locator("select[aria-label='Search in']").inputValue();
  const noteNow = (await p.locator(".find").first().textContent()) ?? "";
  ok(scopeNow === "selection" && !/works on one block/.test(noteNow), `Ctrl+F with several blocks gives the Selection scope and no refusal (scope ${scopeNow})`);
  await p.locator("select[aria-label='Search in']").selectOption("all");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(200);

  // ---- the earlier blocks go when the selection is replaced or the rows change ----
  const several = async () => {
    await p.mouse.click(cx(0), ry(1));
    await ctrlClick(cx(2), ry(3));
    return (await areas()).length;
  };
  ok((await several()) === 2, "(two blocks selected)");
  await p.locator("input.goto").fill("R6 C2");
  await p.keyboard.press("Enter");
  await p.waitForTimeout(300);
  ok((await areas()).length === 1, "Name box (Go To): the earlier blocks go");
  await several();
  await p.keyboard.press("Control+f");
  await p.waitForTimeout(300);
  await p.keyboard.type("Ada");
  await p.locator("select[aria-label='Search in']").selectOption("all");
  await p.waitForTimeout(500);
  await p.getByRole("button", { name: "Find next" }).click();
  await p.waitForTimeout(900);
  const afterFind = (await areas()).length;
  await p.keyboard.press("Escape");
  ok(afterFind === 1, `Find next selects a hit: the earlier blocks go (${afterFind})`);
  await several();
  const shape = await p.evaluate(() => {
    const t = window.__nde.tab;
    t.update({ ...t.info, has_filter: !t.info.has_filter, filtered: !t.info.filtered }, false);
    return t.areas.length;
  });
  ok(shape === 1, "a filter turning on or off (other rows) drops the earlier blocks");
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.update({ ...t.info, has_filter: !t.info.has_filter, filtered: !t.info.filtered }, false);
  });
  await several();
  const rowsChanged = await p.evaluate(() => {
    const t = window.__nde.tab;
    t.update({ ...t.info, rows: t.info.rows + 1 }, false);
    return t.areas.length;
  });
  ok(rowsChanged === 1, "rows added or removed drop the earlier blocks");
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.update({ ...t.info, rows: t.info.rows - 1 }, false);
  });
  await several();
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.update({ ...t.info, history_pos: t.info.history_pos - 1 }, false);
  });
  ok((await areas()).length === 1, "undo or redo (a step back) drops the earlier blocks");
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.update({ ...t.info, history_pos: t.info.history_pos + 1 }, false);
  });

  // ---- the registry test: every command, two blocks, then 10,000 ----
  async function registryRun(label, makeAreas) {
    await p.mouse.click(cx(0), ry(1));
    await makeAreas();
    const n = await p.evaluate(() => window.__nde.tab.areas.length);
    const result = await p.evaluate(async () => {
      const reg = window.__nde.reg;
      const t = window.__nde.tab;
      const bk = window.__nde.backend;
      const snap = async () => ({ text: await bk.copyCells(t.id, 0, 200, 0, 7), pos: t.info.history_pos, rev: t.info.data_revision ?? 0, rows: t.info.rows, cols: t.info.cols, tabs: window.__nde.tabs.length });
      const FREE = /^(file|view|help|window)\./;
      const unflagged = [];
      const flagged = [];
      const free = [];
      for (const c of reg.all()) {
        if (c.multiArea) flagged.push(c.id);
        else if (FREE.test(c.id)) free.push(c.id);
        else unflagged.push(c.id);
      }
      const base = await snap();
      const bad = [];
      const noMessage = [];
      for (const id of unflagged) {
        const before = JSON.stringify(base);
        const dialogsBefore = document.querySelectorAll("[role=dialog],[role=alertdialog]").length;
        const ran = await reg.run(id).catch((e) => "threw " + e);
        const now = JSON.stringify(await snap());
        const dialogsAfter = document.querySelectorAll("[role=dialog],[role=alertdialog]").length;
        if (ran !== false || now !== before || dialogsAfter !== dialogsBefore || t.areas.length !== window.__nde.tab.areas.length) bad.push(`${id}: ran=${ran} same=${now === before} dialogs=${dialogsBefore}->${dialogsAfter}`);
        if (window.__nde.tab.extra.length === 0) bad.push(`${id}: the blocks were dropped`);
      }
      return { unflagged: unflagged.length, flagged, free: free.length, bad, noMessage };
    });
    ok(result.bad.length === 0, `${label}: all ${result.unflagged} commands without the several-blocks flag are refused and change nothing${result.bad.length ? " | " + result.bad.slice(0, 5).join(" ; ") : ""}`);
    console.log(`INFO ${label}: ${n} blocks selected; ${result.flagged.length} flagged (${result.flagged.join(", ")}); ${result.free} selection-free commands (file, view, help, window)`);
    return result;
  }
  const r2 = await registryRun("two blocks", async () => ctrlClick(cx(2), ry(3)));
  ok(r2.flagged.length > 0 && r2.flagged.every((id) => /^(edit|find|format)\./.test(id)), "only edit, find and format commands carry the flag");
  // 10,000 blocks: add them in one go (the model's cap is 10,000).
  const big = await registryRun("10,000 blocks", async () => {
    await p.evaluate(() => {
      const t = window.__nde.tab;
      const r = t.drawRect;
      t.extra = Array.from({ length: 9_999 }, (_, i) => ({ row: 1 + (i % 3000), rows: 1, col: Math.floor(i / 3000) % 6, cols: 1 }));
      void r;
    });
  });
  void big;
  ok(await p.evaluate(() => window.__nde.tab.areas.length) >= 9_999, "the 10,000 blocks stayed selected through the whole run");
  // At the cap another Ctrl+click is declined with a message.
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.extra = Array.from({ length: 9_999 }, (_, i) => ({ row: 10 + i, rows: 1, col: 0, cols: 1 }));
  });
  await ctrlClick(cx(3), ry(12));
  ok(/10,000/.test(await status()), `at 10,000 blocks another Ctrl+click is declined (${await status()})`);

  // ---- the backend refuses writes too (second guard) ----
  await p.mouse.click(cx(0), ry(1));
  await ctrlClick(cx(2), ry(3));
  const refused = await p.evaluate(async () => {
    const t = window.__nde.tab;
    const bk = window.__nde.backend;
    const res = [];
    const tries = {
      setCells: () => bk.setCells(t.id, [[1, 1, "x"]]),
      fillCells: () => bk.fillCells(t.id, 1, 1, 1, 1, "x"),
      change: () => bk.change(t.id, { op: "insert_rows", at: 1, count: 1 }),
      sortRows: () => bk.sortRows(t.id, { keys: [{ col: 0, dir: "asc", pattern: "text" }], header: false }),
      dataOp: () => bk.dataOp(t.id, { op: "paste", spec: { row: 1, col: 1, op: "none", skip_blanks: false, transpose: false }, text: "x" }),
      moveCells: () => bk.moveCells(t.id, 1, 1, 1, 1, 5, 5, false),
    };
    for (const [name, f] of Object.entries(tries)) {
      try {
        await f();
        res.push(`${name}: wrote`);
      } catch (e) {
        if (!(e && e.kind === "one_block")) res.push(`${name}: ${e && e.message}`);
      }
    }
    return res;
  });
  ok(refused.length === 0, `the backend refuses every write while several blocks are selected (${refused.join("; ") || "all refused"})`);

  // ---- Enter and Tab walk the blocks in the order they were picked ----
  await p.keyboard.press("Escape");
  await p.mouse.click(cx(0), ry(1)); // block 1 (picked first)
  await ctrlClick(cx(2), ry(3)); // block 2
  await ctrlDrag(cx(1), ry(6), cx(2), ry(7)); // block 3: 2 x 2 (picked last, active)
  const active = () => p.evaluate(() => [window.__nde.tab.cell.row, window.__nde.tab.cell.col]);
  const picks = await areas(); // in the order picked, the active one last
  ok(picks.length === 3, `three blocks picked (${JSON.stringify(picks)})`);
  const start = await active();
  // The slow model: the cells of each block along its rows, blocks in pick order and round again.
  const rowsOf = (b) => {
    const l = [];
    for (let r = b.row; r < b.row + b.rows; r++) for (let c = b.col; c < b.col + b.cols; c++) l.push([r, c]);
    return l;
  };
  const ring = picks.flatMap(rowsOf);
  const lastOff = picks.slice(0, -1).reduce((n, b) => n + b.rows * b.cols, 0);
  const at0 = lastOff + rowsOf(picks[picks.length - 1]).findIndex(([r, c]) => r === start[0] && c === start[1]);
  const want = Array.from({ length: 7 }, (_, i) => ring[(at0 + 1 + i) % ring.length]);
  const seq = [];
  for (let i = 0; i < 7; i++) {
    await p.keyboard.press("Tab");
    await p.waitForTimeout(120);
    seq.push(await active());
  }
  ok(JSON.stringify(seq) === JSON.stringify(want), `Tab goes block to block in the order picked, along each block's rows, and wraps (${JSON.stringify(seq)} vs ${JSON.stringify(want)})`);
  ok((await areas()).length === 3, "all three blocks stay selected while Tab moves");
  // Shift+Tab goes back the same way: two steps back from where the seven Tabs ended.
  await p.keyboard.press("Shift+Tab");
  await p.keyboard.press("Shift+Tab");
  await p.waitForTimeout(200);
  const back = await active();
  const wantBack = ring[(at0 + 1 + 6 - 2 + ring.length * 2) % ring.length];
  ok(back[0] === wantBack[0] && back[1] === wantBack[1], `Shift+Tab goes back through the cells and blocks (${JSON.stringify(back)} vs ${JSON.stringify(wantBack)})`);
  // Enter goes down each column of a block (and on to the next block past its last cell).
  const beforeEnter = await active();
  await p.keyboard.press("Enter");
  await p.waitForTimeout(150);
  const afterEnter = await active();
  ok(JSON.stringify(afterEnter) !== JSON.stringify(beforeEnter) && (await areas()).length === 3, `Enter moves the active cell and keeps the blocks (${JSON.stringify(beforeEnter)} -> ${JSON.stringify(afterEnter)})`);
  await p.keyboard.press("Escape");

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
