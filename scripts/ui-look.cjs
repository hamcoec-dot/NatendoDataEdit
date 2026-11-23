// UI test: how cells line up (browser stub backend).
// - Numbers sit on the right (View › Numbers on the right), text on the left.
// - Format › Align left / center / right / automatically, per column, undoable.
// - Long text spills into the empty cells beside it; Format › Cut off long
//   text keeps it inside its column.
// Run: (cd ui && npx vite --port 5194) then
// NODE_PATH=<folder with playwright> node scripts/ui-look.cjs <screenshot dir> [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5194";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const p = await b.newPage({ viewport: { width: 1360, height: 860 } });
  const errors = [];
  p.on("pageerror", (e) => errors.push(String(e)));
  // Record what the grid draws: each text with its x, and each clip box.
  await p.addInitScript(() => {
    const C = CanvasRenderingContext2D.prototype;
    const fill = C.fillText;
    const rect = C.rect;
    window.__drawn = [];
    window.__rects = [];
    C.fillText = function (t, x, y, ...rest) {
      window.__drawn.push({ t: String(t), x, y });
      return fill.call(this, t, x, y, ...rest);
    };
    C.rect = function (x, y, w, h) {
      window.__rects.push({ x, y, w, h });
      return rect.call(this, x, y, w, h);
    };
  });
  await p.goto(url);
  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => !/Loading|Indexing|Reading/i.test(document.querySelector("footer.status")?.textContent || ""), null, { timeout: 30000 }).catch(() => {});
  await p.waitForTimeout(1500);
  let failed = 0;
  const ok = (cond, msg) => {
    if (!cond) failed++;
    console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  };
  const start = async (row, col, row2 = row, col2 = col) => {
    await p.evaluate(([r, c, r2, c2]) => {
      window.__nde.grid.select({ row: r, col: c }, { row: r2, col: c2 });
      window.__nde.grid.focus();
    }, [row, col, row2, col2]);
    await p.waitForTimeout(150);
  };
  const put = async (row, col, text) => {
    await start(row, col);
    if (text === "") await p.keyboard.press("Delete");
    else {
      await p.keyboard.type(text);
      await p.keyboard.press("Enter");
    }
    await p.waitForTimeout(250);
  };
  /** Redraw and return where `text` was drawn last. */
  const drawn = async (text) =>
    p.evaluate(async (t) => {
      window.__drawn = [];
      window.__rects = [];
      window.__nde.tab.bump();
      window.__nde.grid.redraw();
      await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
      const hits = window.__drawn.filter((d) => d.t === t);
      return { hit: hits[hits.length - 1] ?? null, rects: window.__rects.slice() };
    }, text);
  const command = async (label) => {
    await p.keyboard.press("Alt+q");
    await p.waitForTimeout(250);
    await p.keyboard.type(label);
    await p.waitForTimeout(250);
    await p.keyboard.press("Enter");
    await p.waitForTimeout(300);
  };
  // Column 1 left and right edges in canvas pixels.
  const edges = async (c) =>
    p.evaluate((col) => {
      const g = window.__nde.grid;
      const r = document.querySelector("canvas").getBoundingClientRect();
      const a = g.headerPoint(col).x - r.left;
      const z = g.headerPoint(col + 1).x - r.left;
      return { right: a, next: z, width: window.__nde.tab.columns.width(col) };
    }, c);

  // ---- 1. Numbers on the right ----
  await put(3, 1, "12345");
  await put(4, 1, "plain words");
  let n = (await drawn("12345")).hit;
  let w = (await drawn("plain words")).hit;
  ok(n && w && n.x > w.x + 20, `a number sits further right than text in the same column (${n?.x} vs ${w?.x})`);
  await command("Numbers on the right");
  let n2 = (await drawn("12345")).hit;
  ok(n2 && w && Math.abs(n2.x - w.x) < 1, `with Numbers on the right off, the number lines up on the left (${n2?.x})`);
  await command("Numbers on the right");

  // ---- 2. Per-column alignment, undoable ----
  await start(4, 1);
  await command("Align right");
  let w2 = (await drawn("plain words")).hit;
  ok(w2 && w2.x > w.x + 10, `Align right moves text to the right (${w2?.x})`);
  await command("Align center");
  let w3 = (await drawn("plain words")).hit;
  ok(w3 && w3.x > w.x && w3.x < w2.x, `Align center puts it between (${w3?.x})`);
  await command("Align left");
  n2 = (await drawn("12345")).hit;
  ok(n2 && Math.abs(n2.x - w.x) < 1, "Align left puts numbers on the left too");
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(300);
  w3 = (await drawn("plain words")).hit;
  ok(w3 && w3.x > w.x && w3.x < w2.x, "Undo puts back the center alignment");
  await command("Align automatically");
  n = (await drawn("12345")).hit;
  w3 = (await drawn("plain words")).hit;
  ok(n && w3 && n.x > w3.x + 20 && Math.abs(w3.x - w.x) < 1, "Align automatically: numbers right, text left again");

  // ---- 3. Long text spills into empty cells ----
  const long = "This text is far too long to fit inside one column";
  await put(6, 2, "");
  await put(6, 3, "");
  await put(6, 1, long);
  const e = await edges(1);
  let d = await drawn(long);
  const spill = d.rects.find((r) => Math.abs(r.x - (d.hit?.x ?? -1)) < 1 && r.w > e.width + 10);
  ok(d.hit && spill, `long text spills past its column into the empty cells (clip ${spill ? Math.round(spill.w) : "none"} wide, column ${e.width})`);
  if (out) await p.screenshot({ path: path.join(out, "text-spill.png"), clip: { x: 0, y: 0, width: 1000, height: 330 } });
  // A filled neighbour stops it.
  await put(6, 2, "x");
  d = await drawn(long);
  ok(!d.rects.some((r) => Math.abs(r.x - (d.hit?.x ?? -1)) < 1 && r.w > e.width + 10), "a value in the next cell stops the spill");
  await put(6, 2, "");
  // Cut off long text.
  await start(6, 1);
  await command("Cut off long text");
  d = await drawn(long);
  ok(!d.rects.some((r) => Math.abs(r.x - (d.hit?.x ?? -1)) < 1 && r.w > e.width + 10), "Cut off long text keeps it inside the column");
  ok(await p.evaluate(() => window.__nde.tab.columns.clips.has(1)), "the column is marked as cutting off");
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(300);
  ok(await p.evaluate(() => !window.__nde.tab.columns.clips.has(1)), "Undo turns it back off");

  // ---- 4. Saved views keep alignment and cut-off ----
  const keep = await p.evaluate(() => {
    const t = window.__nde.tab;
    t.columns.aligns.set(2, "center");
    t.columns.clips.add(3);
    const s = t.columns.snapshot();
    t.columns.reset();
    t.columns.restore(s);
    return t.columns.aligns.get(2) === "center" && t.columns.clips.has(3);
  });
  ok(keep, "alignment and cut-off survive a snapshot and restore");

  // ---- 5. Undo and Redo select what changed and say what it was ----
  const status = () => p.textContent("footer.status");
  const sel = () => p.evaluate(() => ({ ...window.__nde.tab.cell, rect: { ...window.__nde.tab.drawRect } }));
  await put(8, 2, "zzz");
  await start(1, 0);
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(500);
  let s = await sel();
  ok(s.row === 8 && s.col === 2, `Undo selects the cell typed in (R${s.row + 1} C${s.col + 1})`);
  ok(/Undid typing in R9 C3/.test(await status()), `Undo says what it undid (${await status()})`);
  await start(1, 0);
  await p.keyboard.press("Control+y");
  await p.waitForTimeout(500);
  s = await sel();
  ok(s.row === 8 && s.col === 2 && /Redid typing in R9 C3/.test(await status()), "Redo selects it again and says so");
  await start(10, 1, 12, 2);
  await p.keyboard.press("Delete");
  await p.waitForTimeout(400);
  await start(1, 0);
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(500);
  s = await sel();
  ok(s.rect.row === 10 && s.rect.rows === 3 && s.rect.col === 1 && s.rect.cols === 2, "Undo of a cleared block selects the block");
  ok(/Undid “?Clear/i.test(await status()) || /Undid clearing cells/.test(await status()), `Undo names the command (${await status()})`);

  // ---- 6. Copy outline; a copied block repeats to fill a bigger selection ----
  await p.context().grantPermissions(["clipboard-read", "clipboard-write"], { origin: url });
  await put(20, 1, "a1");
  await put(20, 2, "b1");
  await put(21, 1, "a2");
  await put(21, 2, "b2");
  await start(20, 1, 21, 2);
  await p.keyboard.press("Control+c");
  await p.waitForTimeout(400);
  const copied = await p.evaluate(() => window.__nde.tab.copied);
  ok(copied && copied.row === 20 && copied.rows === 2 && copied.cols === 2, "Copy puts a dashed outline round the copied cells");
  if (out) await p.screenshot({ path: path.join(out, "copy-outline.png"), clip: { x: 0, y: 0, width: 1000, height: 400 } });
  await start(30, 1, 33, 4);
  await p.keyboard.press("Control+v");
  await p.waitForTimeout(1200);
  const tiled = await p.evaluate(() => {
    const t = window.__nde.tab;
    const out = [];
    for (let r = 30; r < 34; r++) {
      const line = [];
      for (let c = 1; c < 5; c++) line.push(t.cache.get(r, c));
      out.push(line.join(","));
    }
    return out;
  });
  ok(tiled.join("|") === "a1,b1,a1,b1|a2,b2,a2,b2|a1,b1,a1,b1|a2,b2,a2,b2", `a 2×2 block pasted into 4×4 fills it (${tiled.join(" | ")})`);
  ok(!(await p.evaluate(() => window.__nde.tab.copied)), "the outline goes once something changes");
  await start(20, 1, 21, 2);
  await p.keyboard.press("Control+c");
  await p.waitForTimeout(400);
  await p.keyboard.press("Escape");
  await p.waitForTimeout(200);
  ok(!(await p.evaluate(() => window.__nde.tab.copied)), "Esc takes the outline away");
  // A selection the block does not fit evenly: pasted once.
  await start(40, 1, 42, 2);
  await p.keyboard.press("Control+v");
  await p.waitForTimeout(1200);
  ok(await p.evaluate(() => window.__nde.tab.cache.get(42, 1) !== "a1"), "a selection the block does not fit evenly gets it once");

  // ---- 7. Status bar: Sum and Count together; others can be ticked ----
  await start(2, 4, 6, 4);
  await p.waitForTimeout(800);
  let bar = (await p.textContent("button.totals")) || "";
  ok(/Sum: .*Count: 5/.test(bar), `the status bar shows Sum and Count (${bar.trim()})`);
  await p.click("button.totals");
  await p.click(".pop button:has-text('Average')");
  await p.keyboard.press("Escape");
  await p.mouse.click(5, 5);
  await p.waitForTimeout(300);
  bar = (await p.textContent("button.totals")) || "";
  ok(/Sum: .*Average: .*Count: 5/.test(bar), `ticking Average adds it (${bar.trim()})`);
  if (out) await p.screenshot({ path: path.join(out, "status-totals.png"), clip: { x: 560, y: 820, width: 800, height: 40 } });
  await start(2, 2, 6, 2);
  await p.waitForTimeout(800);
  bar = (await p.textContent("button.totals")) || "";
  ok(/^\s*Count: \d+\s*$/.test(bar), `text only: just the count (${bar.trim()})`);
  await start(2, 4, 6, 4);
  await p.waitForTimeout(800);
  await p.click("button.totals");
  await p.click(".pop button:has-text('Average')");
  await p.mouse.click(5, 5);

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  await b.close();
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  process.exit(failed ? 1 : 0);
})();
