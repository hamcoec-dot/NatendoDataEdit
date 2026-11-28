// UI test of the format painter in the browser stand-in: Paste column look is
// greyed with its reason until a look is copied; Ctrl+Alt+Shift+C and V copy one
// column's look (display format, alignment, wrapping, width) onto others in one
// look step that Ctrl+Z takes back; the toolbar brush pastes once on the next
// heading picked, or stays on after a double-click until Esc; the heading and
// cell right-click menus have the entries; nothing in the file changes.
// Run: (cd ui && npx vite --port 5390) then
// NODE_PATH=<folder with playwright> node scripts/ui-painter.cjs [url] [screenshot dir]
const { chromium } = require("playwright");
const path = require("path");
const url = process.argv[2] || "http://localhost:5390";
const shots = process.argv[3];
(async () => {
  const b = await chromium.launch(process.env.NDE_BROWSER === "chromium" ? { executablePath: process.env.NDE_CHROMIUM } : { channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
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
  const command = (id) => p.evaluate((id) => ({ enabled: window.__nde.reg.enabled(id), why: window.__nde.reg.get(id).why?.() ?? "" }), id);
  const look = (c) =>
    p.evaluate((c) => {
      const k = window.__nde.tab.columns;
      return { kind: k.formats.get(c)?.kind ?? null, align: k.aligns.get(c) ?? null, wrap: k.wraps.has(c), width: k.width(c) };
    }, c);
  const doc = () => p.evaluate(() => ({ modified: window.__nde.tab.info.modified, pos: window.__nde.tab.info.history_pos, undo: window.__nde.tab.info.can_undo }));
  const steps = () => p.evaluate(() => window.__nde.tab.lookSteps.length);
  const select = async (row, col, toCol = col) => {
    await p.evaluate(([row, col, toCol]) => {
      window.__nde.grid.select({ row, col }, { row: 3, col: toCol });
      window.__nde.grid.focus();
    }, [row, col, toCol]);
    await p.waitForTimeout(150);
  };
  const heading = (c) => p.evaluate((c) => window.__nde.grid.headerPoint(c), c);
  const brushBtn = p.getByRole("button", { name: "Format brush (copy the look, then pick columns)" });
  const brushOn = () => brushBtn.evaluate((el) => el.classList.contains("on"));

  await p.evaluate(() =>
    window.__nde.addTab(
      window.__nde.backend.mockFileTab("look.csv", [
        ["when", "a", "b", "c", "d"],
        ["14:30", "1", "2", "3", "4"],
        ["9:05", "5", "6", "7", "8"],
        ["21:00", "9", "10", "11", "12"],
      ]),
    ),
  );
  await p.waitForTimeout(900);
  const yes = await p.$('button:has-text("Yes, column names")');
  if (yes) {
    await yes.click();
    await p.waitForTimeout(400);
  }
  const before = await doc();
  const paste0 = await command("format.pasteLook");
  ok(!paste0.enabled && /Copy a column's look first/.test(paste0.why), `Paste column look is greyed and says why (${paste0.why})`);

  // Column 0's look: time, centred, wrapped, 170 px.
  await select(1, 0);
  await p.keyboard.press("Control+Shift+Digit2");
  await p.evaluate(() => window.__nde.reg.run("format.alignCenter"));
  await p.evaluate(() => window.__nde.reg.run("format.wrap"));
  await p.evaluate(() => {
    window.__nde.tab.columns.setWidth(0, 170);
    window.__nde.tab.bump();
  });
  await p.mouse.click(5, 790);
  await p.waitForTimeout(200);
  await select(1, 0);
  await p.keyboard.press("Control+Alt+Shift+KeyC");
  await p.waitForTimeout(200);
  ok(/Copied the look of 1 column\./.test(await status()), `Ctrl+Alt+Shift+C copies it (${(await status()).slice(0, 90)})`);
  ok((await command("format.pasteLook")).enabled, "Paste column look is on now");
  const s0 = await steps();
  await select(1, 2, 3);
  await p.keyboard.press("Control+Alt+Shift+KeyV");
  await p.waitForTimeout(300);
  const l2 = await look(2);
  const l3 = await look(3);
  ok(JSON.stringify(l2) === JSON.stringify({ kind: "time", align: "center", wrap: true, width: 170 }) && JSON.stringify(l3) === JSON.stringify(l2), `Ctrl+Alt+Shift+V gives both columns the look (${JSON.stringify(l2)})`);
  ok((await steps()) === s0 + 1, `as one look step (${s0} -> ${await steps()})`);
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(300);
  ok((await look(2)).kind === null && (await look(3)).width !== 170, `one Ctrl+Z takes the whole paste back (${JSON.stringify(await look(2))})`);

  // The brush: once.
  await select(1, 0);
  await brushBtn.click();
  await p.waitForTimeout(250);
  ok(await brushOn(), "the brush is on after a click");
  if (shots) await p.screenshot({ path: path.join(shots, "format-brush.png") });
  const h4 = await heading(4);
  await p.mouse.click(h4.x - 20, h4.y - 8);
  await p.waitForTimeout(350);
  ok((await look(4)).kind === "time", `the next heading picked takes the look (${JSON.stringify(await look(4))})`);
  ok(!(await brushOn()), "and the brush is off again");
  // Locked: a double-click keeps it on for several picks; Esc ends it.
  await select(1, 0);
  await brushBtn.dblclick();
  await p.waitForTimeout(250);
  for (const c of [1, 2]) {
    const h = await heading(c);
    await p.mouse.click(h.x - 20, h.y - 8);
    await p.waitForTimeout(300);
  }
  ok((await look(1)).kind === "time" && (await look(2)).kind === "time", "a double-clicked brush paints several picks");
  ok(await brushOn(), "and stays on");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(200);
  ok(!(await brushOn()) && /brush is off/.test(await status()), "Esc turns it off");

  // The menus.
  const h3 = await heading(3);
  await p.mouse.click(h3.x - 20, h3.y - 8, { button: "right" });
  await p.waitForTimeout(300);
  const menu = ((await p.locator('[role="menu"]').last().textContent()) || "").replace(/\s+/g, " ");
  ok(menu.includes("Copy column look") && menu.includes("Paste column look"), "the heading menu has Copy and Paste column look");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(150);
  const cp = await p.evaluate(() => window.__nde.grid.cellPoint({ row: 2, col: 3 }));
  await p.mouse.click(cp.x + 5, cp.y - 8, { button: "right" });
  await p.waitForTimeout(300);
  const cellMenu = ((await p.locator('[role="menu"]').last().textContent()) || "").replace(/\s+/g, " ");
  ok(cellMenu.includes("Paste column look"), "the cell menu has Paste column look once a look is copied");
  await p.keyboard.press("Escape");

  const after = await doc();
  ok(JSON.stringify(after) === JSON.stringify(before), `the file, its changed mark and its undo are untouched (${JSON.stringify(before)} -> ${JSON.stringify(after)})`);
  ok(errors.length === 0, `no page errors (${errors.join("; ")})`);
  await b.close();
  console.log(failed ? `${failed} FAILED` : "ALL PASSED");
  process.exit(failed ? 1 : 0);
})();
