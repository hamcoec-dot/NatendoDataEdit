// UI test: dragging cells onto a tab in the other window, and several blocks
// (gap #6). Browser stub: two pages in one browser share a BroadcastChannel
// and stand in for the two windows; page B is "to the right" of page A, so a
// point past A's right edge is in B's viewport. The pointer leaving page A is
// played with window mouse events (the grid follows a drag on the window).
// Covers: copy, Shift move, private source refused, read-only target refused,
// Esc, several blocks that line up (copy and all-or-nothing move), blocks that
// don't, several blocks within one file refused, a frozen target, a late
// target, the source changed before the answer, and the keyboard path.
// Made-up data (the stub's sample).
// Run: (cd ui && npx vite --port 5389) then
// NODE_PATH=<folder with playwright> node scripts/ui-drag-windows.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2] && !process.argv[2].startsWith("http") ? process.argv[2] : null;
const url = process.argv.find((a) => a.startsWith("http")) || "http://localhost:5389";
(async () => {
  const br = await chromium.launch({ channel: "msedge" });
  const ctx = await br.newContext({ viewport: { width: 1300, height: 800 }, permissions: ["clipboard-read", "clipboard-write"] });
  const A = await ctx.newPage();
  const B = await ctx.newPage();
  const errors = [];
  for (const p of [A, B]) p.on("pageerror", (e) => errors.push(String(e)));
  A.on("crash", () => console.log("INFO page A crashed"));
  B.on("crash", () => console.log("INFO page B crashed"));
  let failed = 0;
  const ok = (cond, msg) => {
    if (!cond) failed++;
    console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  };
  const J = (x) => JSON.stringify(x);
  const settle = (p, ms = 300) => p.waitForTimeout(ms);
  const status = async (p) => ((await p.locator("footer.status").textContent()) || "").replace(/\s+/g, " ");
  const block = async (p, row, rows, col, cols) =>
    (await p.evaluate(([row, rows, col, cols]) => window.__nde.backend.copyCells(window.__nde.tab.id, row, rows, col, cols), [row, rows, col, cols])).replace(/\r/g, "").replace(/\n$/, "");
  const setCells = (p, cells) => p.evaluate(async (cells) => {
    const t = window.__nde.tab;
    // A test edit, made even while several blocks are selected.
    t.update(await window.__nde.reg.withMulti(() => window.__nde.backend.setCells(t.id, cells)), true);
  }, cells);

  // Each "window" gets a small made-up table (pasted into a new file, with titles).
  const lines = ["city\tname\tnote\tregion\textra\tlast"];
  for (let i = 1; i <= 20; i++) lines.push(`Town ${i}\tPerson ${i}\tn${i}\tR${i % 4}\tx${i}\ty${i}`);
  for (const p of [A, B]) {
    await p.goto(url);
    await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });
    await p.evaluate(() => window.__nde.reg.run("file.new"));
    await settle(p, 400);
    await p.evaluate(() => {
      window.__nde.grid.select({ row: 0, col: 0 });
      window.__nde.grid.focus();
    });
    await p.evaluate((t) => navigator.clipboard.writeText(t), lines.join("\r\n") + "\r\n");
    await p.keyboard.press("Control+v");
    await settle(p, 800);
    await p.getByRole("button", { name: /Yes, column names/ }).first().click({ timeout: 1500 }).catch(() => {});
    if (!(await p.evaluate(() => window.__nde.tab.header))) await p.evaluate(() => window.__nde.reg.run("format.header"));
    await settle(p, 300);
  }
  await B.evaluate(() => window.__nde.tab.info.name).then((n) => console.log(`INFO target file ${n}`));
  // Mark A's cells, clear B's landing place, and B's selected cell: row 5, column 1.
  await setCells(A, [[2, 1, "AAA"], [2, 2, "BBB"], [3, 1, "CCC"], [3, 2, "DDD"], [2, 4, "EEE"], [3, 4, "FFF"]]);
  const blank = [];
  for (let r = 5; r < 9; r++) for (let c = 1; c < 5; c++) blank.push([r, c, ""]);
  await setCells(B, blank);
  await B.evaluate(() => window.__nde.grid.select({ row: 5, col: 1 }));
  await settle(B, 400); // B's strip reaches A
  const bName = await B.evaluate(() => window.__nde.tab.info.name);

  // Geometry of A's grid (as the cell-tab suite finds it).
  const canvas = await (await A.$("canvas")).boundingBox();
  const HEAD = 25;
  const ROW = 25;
  let left = 0;
  for (let x = 30; x < 120; x += 2) {
    await A.mouse.click(canvas.x + x, canvas.y + HEAD + ROW + 12);
    if ((await A.evaluate(() => window.__nde.tab.drawRect.cols)) === 1 && /C1$/.test(await A.inputValue("input.goto"))) {
      left = x;
      break;
    }
  }
  ok(left > 0, `row numbers end at ${left}px`);
  const W = 130;
  const cx = (c) => canvas.x + left + c * W + W / 2;
  const ry = (r) => canvas.y + HEAD + r * ROW + ROW / 2;
  const edgeX = canvas.x + left + W + 1;
  const selectBlock = async () => {
    await A.mouse.click(cx(1), ry(1));
    await A.keyboard.down("Shift");
    await A.mouse.click(cx(2), ry(2));
    await A.keyboard.up("Shift");
  };
  const addBlock = async (c0, r0, c1, r1) => {
    await A.keyboard.down("Control");
    await A.mouse.move(cx(c0), ry(r0));
    await A.mouse.down();
    await A.mouse.move(cx(c1), ry(r1), { steps: 5 });
    await A.mouse.up();
    await A.keyboard.up("Control");
  };
  // Where B's first tab is, in A's viewport.
  const bTab = async () => {
    const r = await B.locator('[data-tab-index="0"]').boundingBox();
    const w = await A.evaluate(() => window.innerWidth);
    return { x: w + r.x + r.width / 2, y: r.y + r.height / 2 };
  };
  /** Drag the active block's outline out of A onto B's tab and let go. */
  const dragOut = async (o = {}) => {
    const t = await bTab();
    // The active block's left outline (the last block added).
    const ex = canvas.x + left + (o.col ?? 1) * W + 1;
    const ey = ry(o.row ?? 1);
    await A.mouse.move(ex, ey);
    await A.mouse.down();
    await A.mouse.move(ex + 8, ey, { steps: 2 });
    await A.evaluate(([x, y]) => {
      for (let i = 1; i <= 4; i++) window.dispatchEvent(new MouseEvent("mousemove", { clientX: x * (i / 4) + (1 - i / 4) * 400, clientY: y, buttons: 1 }));
    }, [t.x, t.y]);
    await settle(A, 200);
    if (o.esc) await A.keyboard.press("Escape");
    await A.evaluate(([x, y, shift]) => window.dispatchEvent(new MouseEvent("mouseup", { clientX: x, clientY: y, shiftKey: shift })), [t.x, t.y, !!o.shift]);
    await A.mouse.up();
    if (o.during) await o.during();
    await settle(A, o.wait ?? 800);
  };
  const bBlock = () => block(B, 5, 2, 1, 2);
  const undoB = async () => {
    await B.evaluate(() => window.__nde.reg.run("edit.undo"));
    await settle(B, 400);
  };
  const undoA = async () => {
    await A.evaluate(() => window.__nde.reg.run("edit.undo"));
    await settle(A, 400);
  };

  // ---- copy ----
  await selectBlock();
  await dragOut();
  ok((await bBlock()) === "AAA\tBBB\nCCC\tDDD", `copied into the other window's file at its selected cell (${J(await bBlock())})`);
  ok(/Copied 4 cells to .*\(in the other window\)/.test(await status(A)), `the source says so (${(await status(A)).slice(0, 110)})`);
  ok(/Put 4 cells from .*\(in the other window\)/.test(await status(B)), `the target says its half (${(await status(B)).slice(0, 110)})`);
  ok((await block(A, 2, 2, 1, 2)) === "AAA\tBBB\nCCC\tDDD", "the source keeps its cells");
  if (out) await A.screenshot({ path: path.join(out, "drag-windows-copy.png") });
  await undoB();
  ok((await bBlock()) === "\t\n\t", "Ctrl+Z in the other window takes its half back");

  // ---- Shift: move ----
  await selectBlock();
  await dragOut({ shift: true });
  ok((await bBlock()) === "AAA\tBBB\nCCC\tDDD" && (await block(A, 2, 2, 1, 2)) === "\t\n\t", `Shift moves: there, and cleared here (${J(await block(A, 2, 2, 1, 2))})`);
  ok(/Moved 4 cells to .*Ctrl\+Z in each file/.test(await status(A)), "the source says moved");
  await undoA();
  await undoB();
  ok((await block(A, 2, 2, 1, 2)) === "AAA\tBBB\nCCC\tDDD" && (await bBlock()) === "\t\n\t", "Ctrl+Z in each window undoes its half");

  // ---- Esc: nothing ----
  await selectBlock();
  await dragOut({ esc: true });
  ok((await bBlock()) === "\t\n\t", "Esc during the drag drops nothing");

  // ---- a private source to a file that is not private: refused, the other window hears nothing ----
  await A.evaluate(() => window.__nde.backend.setPrivate(window.__nde.tab.id, false, true));
  const heard0 = await B.evaluate(() => window.__ndeDropsHeard ?? 0);
  await selectBlock();
  await dragOut();
  ok(/is private .* not copied into/.test(await status(A)) && (await bBlock()) === "\t\n\t", `private cells refused (${(await status(A)).slice(0, 100)})`);
  ok((await B.evaluate(() => window.__ndeDropsHeard ?? 0)) === heard0, "the other window was not told anything");
  await A.evaluate(() => window.__nde.backend.setPrivate(window.__nde.tab.id, false, false));

  // ---- a read-only target: refused there, said here ----
  await B.evaluate(() => (window.__nde.tab.info = { ...window.__nde.tab.info, read_only: true }));
  await selectBlock();
  await dragOut();
  ok(/Nothing was copied to .*read-only/.test(await status(A)) && (await bBlock()) === "\t\n\t", `a read-only target says no (${(await status(A)).slice(0, 110)})`);
  await B.evaluate(() => (window.__nde.tab.info = { ...window.__nde.tab.info, read_only: false }));

  // ---- several blocks that line up (same rows): packed, side by side ----
  await selectBlock();
  await addBlock(4, 1, 4, 2);
  ok((await A.evaluate(() => window.__nde.tab.areas.length)) === 2, "two blocks selected");
  await dragOut({ col: 4 });
  ok((await block(B, 5, 2, 1, 3)) === "AAA\tBBB\tEEE\nCCC\tDDD\tFFF", `two blocks go in packed (${J(await block(B, 5, 2, 1, 3))})`);
  await undoB();
  // ...moved: both cleared, in one undo step.
  await selectBlock();
  await addBlock(4, 1, 4, 2);
  await dragOut({ shift: true, col: 4 });
  const both = async () => (await block(A, 2, 2, 1, 2)) + "|" + (await block(A, 2, 2, 4, 1));
  ok((await both()) === "\t\n\t|\n", `both blocks cleared, the column between left alone (${J(await both())}, ${J(await block(A, 2, 2, 3, 1))})`);
  await undoA();
  ok((await both()) === "AAA\tBBB\nCCC\tDDD|EEE\nFFF", `one Ctrl+Z brings both back (${J(await both())})`);
  await undoB();

  // ---- several blocks that don't line up: refused ----
  await selectBlock();
  await addBlock(4, 4, 4, 4);
  await dragOut({ col: 4, row: 4 });
  ok(/needs them to line up/.test(await status(A)) && (await bBlock()) === "\t\n\t", "blocks that don't line up are refused");

  // ---- several blocks within this file: refused ----
  await selectBlock();
  await addBlock(4, 1, 4, 2);
  const e4 = canvas.x + left + 4 * W + 1;
  await A.mouse.move(e4, ry(1));
  await A.mouse.down();
  await A.mouse.move(e4 + 8, ry(1), { steps: 2 });
  await A.mouse.move(cx(1), ry(8), { steps: 6 });
  await A.mouse.up();
  await settle(A, 400);
  ok(/Several blocks can be dragged onto another file's tab, not within this file/.test(await status(A)), "several blocks within one file: refused, with why");
  ok((await block(A, 2, 2, 1, 2)) === "AAA\tBBB\nCCC\tDDD", "...and nothing moved");

  // ---- a frozen target: too late, nothing written, nothing cleared ----
  await A.evaluate(() => (window.__nde.backend.constructor.DROP_DEADLINE_MS = 1500));
  await B.evaluate(() => (window.__ndeDropFreeze = true));
  await selectBlock();
  await dragOut({ shift: true, wait: 2500 });
  ok(/did not take the cells in time/.test(await status(A)), `frozen target: the source says too late (${(await status(A)).slice(0, 100)})`);
  ok((await bBlock()) === "\t\n\t" && (await block(A, 2, 2, 1, 2)) === "AAA\tBBB\nCCC\tDDD", "nothing written there, nothing cleared here");
  await B.evaluate(() => (window.__ndeDropFreeze = false));

  // ---- a late target: it refuses, writes nothing; the source says too late ----
  await B.evaluate(() => (window.__ndeDropDelay = 2500));
  await selectBlock();
  await dragOut({ wait: 3500 });
  ok(/did not take the cells in time/.test(await status(A)), "late target: the source says too late");
  ok((await bBlock()) === "\t\n\t", "the late target wrote nothing");
  await B.evaluate(() => (window.__ndeDropDelay = 0));

  // ---- the source changes before the answer: copied, not moved ----
  await A.evaluate(() => (window.__nde.backend.constructor.DROP_DEADLINE_MS = 10000));
  await B.evaluate(() => (window.__ndeDropDelay = 700));
  await selectBlock();
  await dragOut({ shift: true, wait: 1500, during: () => setCells(A, [[9, 5, "edited meanwhile"]]) });
  ok(/Copied, not moved: 4 cells went to .*changed meanwhile/.test(await status(A)), `source changed: copied, not moved (${(await status(A)).slice(0, 120)})`);
  ok((await bBlock()) === "AAA\tBBB\nCCC\tDDD" && (await block(A, 2, 2, 1, 2)) === "AAA\tBBB\nCCC\tDDD", "there, and still here");
  await undoB();
  // ...several blocks, one edited meanwhile: none cleared.
  await selectBlock();
  await addBlock(4, 1, 4, 2);
  await dragOut({ shift: true, col: 4, wait: 1500, during: () => setCells(A, [[2, 4, "EEE edited"]]) });
  ok((await block(A, 2, 2, 1, 2)) === "AAA\tBBB\nCCC\tDDD" && /Copied, not moved/.test(await status(A)), "several blocks, one edited meanwhile: none cleared");
  await undoB();
  await B.evaluate(() => (window.__ndeDropDelay = 0));

  // ---- the keyboard path lists the other window's file ----
  await selectBlock();
  await A.evaluate(() => window.__nde.reg.run("edit.copyToTab"));
  await settle(A, 400);
  const listed = await A.getByRole("button", { name: new RegExp(`${bName.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")} \\(other window\\)`) }).count();
  ok(listed === 1, "Copy selection to another file lists the other window's file");
  await A.getByRole("button", { name: /\(other window\)/ }).click();
  await settle(A, 1000);
  ok((await bBlock()) === "AAA\tBBB\nCCC\tDDD", `...and copies there (${J(await bBlock())})`);

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  await br.close();
  console.log(failed ? `${failed} FAILED` : "ALL PASSED");
  process.exit(failed ? 1 : 0);
})();
