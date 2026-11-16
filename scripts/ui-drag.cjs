// UI test of the mouse gestures (drag columns, rows and cells to move them,
// Ctrl+drag to copy, Esc to cancel, resize several columns at once, show
// hidden columns from their mark, drag the corner to freeze, drag file tabs)
// against the vite dev server (browser stub backend).
// Run: (cd ui && npx vite --port 5191) then
// NODE_PATH=<folder with playwright> node scripts/ui-drag.cjs <screenshot dir> [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5191";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1360, height: 860 }, permissions: ["clipboard-read", "clipboard-write"] });
  const p = await ctx.newPage();
  const errors = [];
  p.on("pageerror", (e) => errors.push(String(e)));
  await p.goto(url);
  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => !/Loading|Indexing|Reading/i.test(document.querySelector("footer.status")?.textContent || ""), null, { timeout: 30000 }).catch(() => {});
  await p.waitForTimeout(1500);
  let failed = 0;
  const ok = (cond, msg) => {
    if (!cond) failed++;
    console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  };
  const canvas = await (await p.$("canvas")).boundingBox();
  const HEAD = 25;
  const ROW = 25;
  const at = async () => p.inputValue("input.goto");
  const value = async () => p.inputValue("textarea.cellvalue");
  const status = async () => (await p.textContent("footer.status .msg").catch(() => "")) || "";
  // Find where the row numbers end and how wide a column is.
  let left = 0;
  for (let x = 30; x < 120; x += 2) {
    await p.mouse.click(canvas.x + x, canvas.y + HEAD + ROW + 12);
    if (/C1$/.test(await at())) {
      left = x;
      break;
    }
  }
  ok(left > 0, `row numbers end at ${left}px`);
  const W = 130;
  const cx = (c) => canvas.x + left + c * W + W / 2;
  const ry = (r) => canvas.y + HEAD + r * ROW + ROW / 2; // r: rows on screen below the titles
  const cellAt = async (r, c) => {
    await p.mouse.click(cx(c), ry(r));
    return value();
  };
  const row = async (r) => {
    const v = [];
    for (let c = 0; c < 5; c++) v.push(await cellAt(r, c));
    return v;
  };
  const drag = async (x0, y0, x1, y1, opts = {}) => {
    await p.mouse.move(x0, y0);
    await p.mouse.down();
    await p.mouse.move(x0 + 8, y0 + (opts.vertical ? 8 : 0), { steps: 2 });
    if (opts.ctrl) await p.keyboard.down("Control");
    await p.mouse.move(x1, y1, { steps: 8 });
    if (opts.shot) await p.screenshot({ path: path.join(out, opts.shot) });
    if (opts.esc) await p.keyboard.press("Escape");
    await p.mouse.up();
    if (opts.ctrl) await p.keyboard.up("Control");
    await p.waitForTimeout(300);
  };
  const shiftClick = async (x, y) => {
    await p.keyboard.down("Shift");
    await p.mouse.click(x, y);
    await p.keyboard.up("Shift");
  };
  const undo = async () => {
    await p.keyboard.press("Control+z");
    await p.waitForTimeout(300);
  };

  const first = await row(1);
  const second = await row(2);
  ok(first[1] !== first[2], `sample row read (${first.join(" | ")})`);

  // ---- columns: select B, drag its letter past D ------------------------------
  await p.mouse.click(cx(1), canvas.y + HEAD / 2);
  await drag(cx(1), canvas.y + HEAD / 2, canvas.x + left + 4 * W - 10, canvas.y + HEAD / 2, { shot: "drag-column.png" });
  let now = await row(1);
  ok(now[1] === first[2] && now[2] === first[3] && now[3] === first[1], `column B moved after D (${now.join(" | ")})`);
  ok(/Moved 1 column/.test(await status()), `status says so (${await status()})`);
  await undo();
  now = await row(1);
  ok(now.join() === first.join(), "Ctrl+Z puts the column back");

  // A click on a selected column without moving only selects it.
  await p.mouse.click(cx(1), canvas.y + HEAD / 2);
  await shiftClick(cx(3), canvas.y + HEAD / 2);
  await p.mouse.click(cx(2), canvas.y + HEAD / 2);
  now = await row(1);
  ok(now.join() === first.join(), "a click without a drag moves nothing");

  // Esc while dragging drops nothing.
  await p.mouse.click(cx(1), canvas.y + HEAD / 2);
  await drag(cx(1), canvas.y + HEAD / 2, cx(4), canvas.y + HEAD / 2, { esc: true });
  now = await row(1);
  ok(now.join() === first.join(), "Esc cancels a column drag");

  // ---- rows: select row 1, drag its number down past row 3 ----------------------
  const rx = canvas.x + left / 2;
  await p.mouse.click(rx, ry(1));
  await drag(rx, ry(1), rx, ry(3) + ROW / 2 - 2, { vertical: true, shot: "drag-row.png" });
  const r1 = await row(1);
  const r3 = await row(3);
  ok(r1.join() === second.join() && r3.join() === first.join(), `row moved down two (${r3.join(" | ")})`);
  await undo();
  ok((await row(1)).join() === first.join(), "Ctrl+Z puts the row back");

  // ---- a block of cells: select B1:C2, drag its outline two right, one down -----
  await p.mouse.click(cx(1), ry(1));
  await shiftClick(cx(2), ry(2));
  const edgeX = canvas.x + left + W + 1;
  await p.mouse.move(edgeX, ry(1));
  await p.waitForTimeout(100);
  const cur = await p.$eval(".grid, [aria-label='Data grid']", (e) => getComputedStyle(e).cursor).catch(() => "");
  ok(cur === "grab", `the outline shows a grab cursor (${cur})`);
  await drag(edgeX, ry(1), edgeX + 2 * W, ry(2), { shot: "drag-cells.png" });
  now = await row(2);
  ok(now[3] === first[1] && now[4] === first[2], `block landed at D2 (${now.join(" | ")})`);
  ok((await cellAt(1, 1)) === "" && (await cellAt(1, 2)) === "", "the cells it left are empty");
  await undo();
  ok((await row(1)).join() === first.join() && (await row(2)).join() === second.join(), "Ctrl+Z puts the block back");

  // Ctrl+drag copies (Ctrl pressed mid-drag, and Ctrl held before the press).
  await p.mouse.click(cx(1), ry(1));
  await drag(canvas.x + left + W + 1, ry(1), canvas.x + left + W + 1, ry(4), { ctrl: true });
  ok((await cellAt(1, 1)) === first[1] && (await cellAt(4, 1)) === first[1], "Ctrl+drag copies the cell");
  await undo();
  await p.mouse.click(cx(1), ry(1));
  await p.keyboard.down("Control");
  await drag(canvas.x + left + W + 1, ry(1), canvas.x + left + W + 1, ry(5));
  await p.keyboard.up("Control");
  ok((await cellAt(1, 1)) === first[1] && (await cellAt(5, 1)) === first[1], "Ctrl held before pressing the outline copies too");
  await undo();

  // ---- cut and paste that moves --------------------------------------------------
  // Columns: cut B, pick D, paste: B goes in before D (A C B D ...).
  await p.mouse.click(cx(1), canvas.y + HEAD / 2);
  await p.keyboard.press("Control+x");
  await p.waitForTimeout(300);
  ok(/Cut 1 column/.test(await status()), `cut says what to do next (${await status()})`);
  ok((await row(1)).join() === first.join(), "cutting changes nothing yet");
  await p.mouse.click(cx(3), canvas.y + HEAD / 2);
  await p.keyboard.press("Control+v");
  await p.waitForTimeout(400);
  now = await row(1);
  ok(now[1] === first[2] && now[2] === first[1] && now[3] === first[3] && now.length === first.length, `cut column went in before D, no gap (${now.join(" | ")})`);
  await undo();
  ok((await row(1)).join() === first.join(), "Ctrl+Z undoes the cut and paste in one step");
  // Rows: cut screen row 1, pick screen row 4, paste: it goes above row 4.
  await p.mouse.click(rx, ry(1));
  await p.keyboard.press("Control+x");
  await p.waitForTimeout(300);
  await p.mouse.click(rx, ry(4));
  await p.keyboard.press("Control+v");
  await p.waitForTimeout(400);
  ok((await row(1)).join() === second.join() && (await row(3)).join() === first.join(), "cut row went in above the row picked");
  await undo();
  // Cells: cut B1, pick E3, paste: moved, the old place empty.
  await p.mouse.click(cx(1), ry(1));
  await p.keyboard.press("Control+x");
  await p.waitForTimeout(300);
  await p.mouse.click(cx(4), ry(3));
  await p.keyboard.press("Control+v");
  await p.waitForTimeout(400);
  ok((await cellAt(3, 4)) === first[1] && (await cellAt(1, 1)) === "", "cut cell moved on paste");
  await undo();
  // Esc drops the cut: a paste then pastes the copied text instead of moving.
  await p.mouse.click(cx(1), ry(1));
  await p.keyboard.press("Control+x");
  await p.keyboard.press("Escape");
  await p.mouse.click(cx(4), ry(6));
  await p.keyboard.press("Control+v");
  await p.waitForTimeout(400);
  ok((await cellAt(1, 1)) === first[1], "after Esc the cut cell stays where it was");
  await undo();
  ok((await row(1)).join() === first.join() && (await row(2)).join() === second.join(), "all back as it was");

  // ---- a block never moves over or out of hidden columns -------------------------
  await p.mouse.click(cx(2), canvas.y + HEAD / 2);
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("hide columns");
  await p.keyboard.press("Enter");
  await p.waitForTimeout(300);
  // B and D now sit side by side; select B1:D1 (C hidden between) and drag.
  await p.mouse.click(cx(1), ry(1));
  await shiftClick(cx(2), ry(1));
  await drag(canvas.x + left + W + 1, ry(1), canvas.x + left + W + 1, ry(4));
  ok(/hidden columns/.test(await status()) && (await cellAt(1, 1)) === first[1], `refused with hidden columns inside (${await status()})`);
  // A1:B1 dragged one column right would cover the hidden C: refused too.
  await p.mouse.click(cx(0), ry(1));
  await shiftClick(cx(1), ry(1));
  await drag(canvas.x + left + 1, ry(1), canvas.x + left + W + 1, ry(1));
  ok(/hidden columns/.test(await status()) && (await cellAt(1, 0)) === first[0], `the block did not go over the hidden column (${await status()})`);
  await p.keyboard.press("Control+Shift+0");
  await p.waitForTimeout(200);
  ok((await row(1)).join() === first.join(), "nothing changed");

  // Hidden columns at the far right get an arrow on the last shown heading.
  await p.mouse.click(cx(6), canvas.y + HEAD / 2);
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("hide columns");
  await p.keyboard.press("Enter");
  await p.waitForTimeout(300);
  await p.screenshot({ path: path.join(out, "trailing-hidden.png") });
  await p.mouse.click(canvas.x + left + 6 * W - 16 - 6 - 5, canvas.y + HEAD / 2);
  await p.mouse.click(cx(6), ry(1));
  ok(/C7$/.test(await at()), `the arrow on F shows it again (${await at()})`);

  // ---- several columns at once: select B:C, widen C's edge by 60 ------------------
  await p.mouse.click(cx(1), canvas.y + HEAD / 2);
  await shiftClick(cx(2), canvas.y + HEAD / 2);
  const edge = canvas.x + left + 3 * W - 1;
  await drag(edge, canvas.y + HEAD / 2, edge + 60, canvas.y + HEAD / 2);
  // B now ends at left + W + 190.
  await p.mouse.click(canvas.x + left + W + 185, ry(1));
  ok(/C2$/.test(await at()), `B widened with C (${await at()})`);
  await p.mouse.click(canvas.x + left + W + 195, ry(1));
  ok(/C3$/.test(await at()), `C starts after the wider B (${await at()})`);
  await undo(); // widths are view only: undo does nothing to them
  await p.keyboard.press("Control+Shift+0");

  // ---- hidden columns: hide C, click its mark on D's header -------------------
  const widen = canvas.x + left + W + 190 + 190; // D's left edge with B, C at 190
  await p.mouse.click(canvas.x + left + W + 250, canvas.y + HEAD / 2);
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("hide columns");
  await p.keyboard.press("Enter");
  await p.waitForTimeout(300);
  await p.mouse.click(canvas.x + left + W + 250, ry(1));
  ok(/C4$/.test(await at()), `C hidden, D moved left (${await at()})`);
  await p.screenshot({ path: path.join(out, "hidden-mark.png") });
  await p.mouse.click(canvas.x + left + W + 190 + 9, canvas.y + HEAD / 2);
  await p.mouse.click(canvas.x + left + W + 190 + 185, ry(1));
  ok(/C3$/.test(await at()), `the mark shows C again, as wide as before (${await at()})`);
  void widen;

  // ---- freeze: drag the corner's bottom edge down two rows ------------------------
  await drag(canvas.x + left / 2, canvas.y + HEAD - 2, canvas.x + left / 2, canvas.y + HEAD + 2 * ROW, { vertical: true, shot: "freeze-drag.png" });
  await p.mouse.move(cx(1), ry(8));
  await p.mouse.wheel(0, 600);
  await p.waitForTimeout(200);
  await p.mouse.click(cx(1), ry(1));
  const frozenAt = await at();
  await p.mouse.click(cx(1), ry(4));
  const scrolledAt = await at();
  ok(/^R3 /.test(frozenAt) && !/^R6 /.test(scrolledAt), `two rows stay in view (${frozenAt}, ${scrolledAt})`);
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("unfreeze");
  await p.keyboard.press("Enter");

  // ---- widths travel with moved columns, through undo and redo -----------------
  // Now A 130, B 190, C 190 (shown again with its old width), D 130.
  // Make B 250, then drag it after D.
  await p.mouse.click(cx(1), canvas.y + HEAD / 2);
  const bEdge = canvas.x + left + W + 190 - 1;
  await drag(bEdge, canvas.y + HEAD / 2, bEdge + 60, canvas.y + HEAD / 2);
  await p.mouse.click(canvas.x + left + W + 100, canvas.y + HEAD / 2);
  await drag(canvas.x + left + W + 100, canvas.y + HEAD / 2, canvas.x + left + W + 250 + 190 + 120, canvas.y + HEAD / 2);
  const colAtX = async (x) => {
    await p.mouse.click(x, ry(1));
    return (await at()).replace(/^R\d+ /, "");
  };
  // A 130, C 190, D 130, B 250: x = left + 130 + 190 + 130 + 240 is still B (C4).
  await p.screenshot({ path: path.join(out, "width-move.png") });
  let probe = await colAtX(canvas.x + left + 130 + 190 + 130 + 240);
  ok(probe === "C4", `the moved column keeps its width (${probe})`);
  await undo();
  // A 130, B 250, C 190, D 130.
  ok((await colAtX(canvas.x + left + 130 + 245)) === "C2" && (await colAtX(canvas.x + left + 130 + 250 + 185)) === "C3", "undo brings the width back with it");
  await p.keyboard.press("Control+y");
  await p.waitForTimeout(300);
  ok((await colAtX(canvas.x + left + 130 + 190 + 130 + 240)) === "C4", "redo moves the width again");

  // ---- file tabs: open another, drag it in front of the first -------------------
  await p.keyboard.press("Control+o");
  await p.waitForTimeout(1500);
  const names = () => p.$$eval(".tabs .tab span:first-child", (e) => e.map((x) => x.textContent.trim()));
  const before = await names();
  if (before.length >= 2) {
    const tabs = await p.$$(".tabs .tab");
    const a = await tabs[0].boundingBox();
    const z = await tabs[before.length - 1].boundingBox();
    await drag(z.x + 20, z.y + z.height / 2, a.x + 10, a.y + a.height / 2);
    const after = await names();
    ok(after[0] === before[before.length - 1], `tab dragged to the front (${before.join(", ")} -> ${after.join(", ")})`);
  } else ok(false, "a second tab opened");

  // ---- Alt+Q finds the mouse moves --------------------------------------------
  for (const words of ["drag columns", "reorder rows", "mouse"]) {
    await p.keyboard.press("Alt+q");
    await p.keyboard.type(words);
    await p.waitForTimeout(200);
    const top = await p.$$eval("#command-results [role=option]", (e) => e.slice(0, 3).map((x) => x.textContent.trim()));
    ok(top.some((t) => /Mouse and drag moves|Move (columns|rows)/.test(t)), `Alt+Q "${words}" finds it (${top[0]})`);
    await p.keyboard.press("Escape");
  }
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("mouse and drag");
  await p.keyboard.press("Enter");
  await p.waitForTimeout(300);
  ok(await p.isVisible("text=Drag the selection's outline"), "Help > Mouse and drag moves lists the moves");
  await p.screenshot({ path: path.join(out, "mouse-help.png") });
  await p.keyboard.press("Escape");

  ok(errors.length === 0, `no page errors ${errors.join("; ")}`);
  await b.close();
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  process.exit(failed ? 1 : 0);
})();
