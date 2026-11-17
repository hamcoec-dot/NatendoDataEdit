// UI test: Undo and Redo cover filters (Show duplicates too) and changes to
// the look (widths, hiding columns, banding), in order with edits, and a
// deleted column comes back with its width (browser stub backend).
// Run: (cd ui && npx vite --port 5193) then
// NODE_PATH=<folder with playwright> node scripts/ui-undo.cjs <screenshot dir> [url]
const { chromium } = require("playwright");
const url = process.argv[3] || "http://localhost:5193";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const p = await b.newPage({ viewport: { width: 1360, height: 860 } });
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
  const command = async (q) => {
    await p.keyboard.press("Alt+q");
    await p.keyboard.type(q);
    await p.waitForTimeout(250);
    await p.keyboard.press("Enter");
    await p.waitForTimeout(600);
  };
  const state = () =>
    p.evaluate(() => {
      const t = window.__nde.tab;
      return {
        filtered: t.info.has_filter,
        rows: t.info.rows,
        modified: t.info.modified,
        w0: t.columns.width(0),
        w1: t.columns.width(1),
        hidden0: t.columns.isHidden(0),
        banded: t.banded,
        canUndo: t.canUndo,
        canRedo: t.canRedo,
      };
    });
  const select = (row, col) => p.evaluate(([r, c]) => window.__nde.grid.select({ row: r, col: c }), [row, col]);
  const key = async (k) => {
    await p.keyboard.press(k);
    await p.waitForTimeout(500);
  };
  const focusGrid = () => p.evaluate(() => window.__nde.grid.focus());

  await focusGrid();
  const s0 = await state();
  ok(!s0.filtered && !s0.canUndo, "starts with nothing to undo");

  // A filter is an undo step that leaves the file saved.
  await select(1, 2);
  await command("Filter by selected value");
  const s1 = await state();
  ok(s1.filtered && s1.rows < s0.rows, `Filter by selected value: ${s1.rows} rows show`);
  ok(!s1.modified, "the filter leaves the file saved");
  await focusGrid();
  await key("Control+z");
  const s2 = await state();
  ok(!s2.filtered && s2.rows === s0.rows, "Ctrl+Z takes the filter off");
  await key("Control+y");
  ok((await state()).filtered, "Ctrl+Y puts it back");
  await command("Clear filter");
  ok(!(await state()).filtered, "Clear filter shows every row");
  await focusGrid();
  await key("Control+z");
  ok((await state()).filtered, "Ctrl+Z brings the cleared filter back");
  await key("Control+z");
  ok(!(await state()).filtered && !(await state()).canUndo, "and undo again takes it off, back to the start");

  // Show duplicates is a filter too.
  await command("Show duplicates");
  const dlg = "[role=dialog]:has(h2:has-text('Show duplicates'))";
  if (await p.isVisible(dlg)) {
    await p.click(`${dlg} .all-none ~ div input[type=checkbox] >> nth=1`);
    await p.click(`${dlg} button.primary`);
    await p.waitForTimeout(1500);
  }
  ok((await state()).filtered, "Show duplicates filters the rows");
  await focusGrid();
  await key("Control+z");
  ok(!(await state()).filtered, "Ctrl+Z after Show duplicates shows every row again");

  // Look changes: AutoFit, hide, banding; each its own step, in order with an edit.
  await select(1, 0);
  await focusGrid();
  const wDefault = (await state()).w0;
  await command("AutoFit column width");
  const wFit = (await state()).w0;
  ok(wFit !== wDefault, `AutoFit changes the width (${wDefault} -> ${wFit})`);
  await focusGrid();
  await key("Control+0");
  ok((await state()).hidden0, "Ctrl+0 hides the column");
  await select(1, 1);
  await focusGrid();
  await p.keyboard.type("zz");
  await key("Enter");
  ok((await state()).modified, "typing in a cell is an edit");
  await command("Alternating row colours");
  ok((await state()).banded, "banding on");
  await focusGrid();
  await key("Control+z");
  let s = await state();
  ok(!s.banded && s.modified && s.hidden0, "Ctrl+Z: banding off first");
  await key("Control+z");
  s = await state();
  ok(!s.modified && s.hidden0, "then the edit");
  await key("Control+z");
  s = await state();
  ok(!s.hidden0 && s.w0 === wFit, "then the hide (the column shows at its fitted width)");
  await key("Control+z");
  ok((await state()).w0 === wDefault, "then the AutoFit");
  await key("Control+y");
  await key("Control+y");
  s = await state();
  ok(s.hidden0 && !s.modified, "Ctrl+Y twice: fitted, then hidden again");
  await key("Control+y");
  ok((await state()).modified, "Ctrl+Y redoes the edit");
  // A look change after undoing ends the redo.
  await key("Control+z");
  await command("Alternating row colours");
  await focusGrid();
  s = await state();
  ok(s.banded && !s.canRedo, "a new change after undo: nothing left to redo");
  await key("Control+y");
  ok(!(await state()).modified, "Ctrl+Y does not bring back the undone edit");
  await key("Control+z");
  await command("Unhide all columns");
  await focusGrid();

  // A deleted column comes back with its width.
  await select(1, 1);
  await command("AutoFit column width");
  const w1 = (await state()).w1;
  await select(1, 1);
  await command("Delete columns");
  await focusGrid();
  ok((await state()).w1 !== w1, "the column is deleted (the next one has its own width)");
  await key("Control+z");
  ok((await state()).w1 === w1, `Ctrl+Z brings the column back at its width (${w1})`);

  // ---- a long check list does not slow each key press, and steps share it ----
  const cost = await p.evaluate(() => {
    const t = window.__nde.tab;
    const values = Array.from({ length: 100000 }, (_, i) => `value ${i}`);
    t.checks = [{ col: 2, kind: { type: "list", values, match_case: false } }];
    t.recordView();
    const t0 = performance.now();
    for (let i = 0; i < 50; i++) t.recordView();
    return (performance.now() - t0) / 50;
  });
  ok(cost < 5, `with a 100,000-value check list a key press costs ${cost.toFixed(2)} ms to note`);
  for (let i = 0; i < 4; i++) {
    await select(2 + i, 1);
    await focusGrid();
    await p.keyboard.type(`e${i}`);
    await key("Enter");
  }
  const shared = await p.evaluate(() => {
    const t = window.__nde.tab;
    const looks = [...t.stepViews.values()].slice(-4).map((s) => s.before.look.checks);
    return new Set(looks).size;
  });
  ok(shared === 1, `four edits share one copy of the check list (${shared})`);
  for (let i = 0; i < 4; i++) await key("Control+z");
  await key("Control+z"); // and the check list itself
  ok((await p.evaluate(() => window.__nde.tab.checks.length)) === 0, "Ctrl+Z takes the check list off again");

  // ---- opening a filter menu after Undo leaves Redo working ----
  await select(1, 1);
  await focusGrid();
  await p.keyboard.type("r1");
  await key("Enter");
  await key("Control+z");
  await command("Filter"); // arrows off
  await focusGrid();
  await select(1, 2);
  await command("Filter menu for this column");
  await key("Escape");
  await focusGrid();
  ok((await state()).canRedo, "after opening and closing a filter menu, Ctrl+Y can still redo");
  await key("Control+y");
  ok((await state()).modified, "and it redoes the edit");
  await key("Control+z");

  // ---- a saved view applies, and undoes, in one step ----
  await command("Alternating row colours");
  await select(1, 3);
  await command("Sort A");
  await select(1, 2);
  await command("Filter by selected value");
  await command("Save view to a file");
  await focusGrid();
  for (let i = 0; i < 3; i++) await key("Control+z");
  s = await state();
  ok(!s.banded && !s.filtered, "back to no banding and no filter");
  await command("Apply a view from a file");
  await p.waitForTimeout(1500);
  s = await state();
  ok(s.banded && s.filtered, "Apply a view: banding and the filter are back");
  await focusGrid();
  await key("Control+z");
  s = await state();
  ok(!s.banded && !s.filtered, "one Ctrl+Z takes the whole view back");
  await key("Control+y");
  s = await state();
  ok(s.banded && s.filtered, "one Ctrl+Y puts it all back");

  // ---- Alt+Q finds Undo by what it now covers ----
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("undo filter");
  await p.waitForTimeout(300);
  const hits = await p.$$eval("#command-results [role=option]", (e) => e.slice(0, 5).map((x) => x.textContent.trim()));
  ok(hits.some((h) => /^Undo/.test(h)), `Alt+Q "undo filter" offers Undo (${hits.slice(0, 3).join(" / ")})`);
  await p.keyboard.press("Escape");

  ok(errors.length === 0, `no page errors ${errors.join("; ")}`);
  await b.close();
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  process.exit(failed ? 1 : 0);
})();
