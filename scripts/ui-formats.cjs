// UI test of the time, date and time and custom display formats in the browser
// stand-in: Ctrl+Shift+2 (by the key's place) shows a column as time, Ctrl+1's
// Custom code shows phone digits as 000-000-0000 with a live preview, [Red]
// negatives are drawn red (a pixel probe) and not at all under forced colours,
// the dialog refuses 0.0E+0 with its message, none of it changes the file, the
// changed mark or undo, Write into cells writes it and Ctrl+Z takes it back,
// and the key does nothing while a cell is being edited.
// Run: (cd ui && npx vite --port 5390) then
// NODE_PATH=<folder with playwright> node scripts/ui-formats.cjs [url] [screenshot dir]
const { chromium } = require("playwright");
const path = require("path");
const url = process.argv[2] || "http://localhost:5390";
const shots = process.argv[3];
(async () => {
  // Edge by default; NDE_BROWSER=chromium (NDE_CHROMIUM=path to its chrome.exe when the bundled one is missing).
  const b = await chromium.launch(process.env.NDE_BROWSER === "chromium" ? { executablePath: process.env.NDE_CHROMIUM } : { channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1200, height: 800 }, deviceScaleFactor: 1 });
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
  const run = (id) => p.evaluate((id) => window.__nde.reg.run(id), id);
  const fmt = (c) => p.evaluate((c) => window.__nde.tab.columns.formats.get(c) ?? null, c);
  const state = () => p.evaluate(async () => {
    const t = window.__nde.tab;
    const cells = [];
    for (let r = 0; r < 4; r++) for (let c = 0; c < 4; c++) cells.push((await window.__nde.backend.cellText(t.id, r, c)).text);
    return { modified: t.info.modified, pos: t.info.history_pos, undo: t.info.can_undo, cells };
  });
  const select = async (row, col) => {
    await p.evaluate(([row, col]) => {
      window.__nde.grid.select({ row, col });
      window.__nde.grid.focus();
    }, [row, col]);
    await p.waitForTimeout(150);
  };
  /** Red text pixels in a cell (the canvas read back). */
  const redIn = (row, col) =>
    p.evaluate(([row, col]) => {
      const g = window.__nde.grid;
      const cv = document.querySelector(".grid canvas");
      const b = cv.getBoundingClientRect();
      const pt = g.cellPoint({ row, col });
      const k = cv.width / b.width;
      const w = window.__nde.tab.columns.width(col);
      const x = Math.round((pt.x - 4 - b.left) * k);
      const y = Math.round((pt.y - 22 - b.top) * k);
      const d = cv.getContext("2d").getImageData(x, y, Math.round(w * k), Math.round(22 * k)).data;
      let n = 0;
      for (let i = 0; i < d.length; i += 4) if (d[i] > 150 && d[i + 1] < 110 && d[i + 2] < 110) n++;
      return n;
    }, [row, col]);

  await p.evaluate(() =>
    window.__nde.addTab(
      window.__nde.backend.mockFileTab("people.csv", [
        ["when", "phone", "amount", "id"],
        ["2026-03-14T14:30:00Z", "6155551234", "-1234.5", "00123"],
        ["2026-03-15 09:05", "6155550000", "2500", "7"],
        ["9:30 PM", "123", "0", "12"],
      ]),
    ),
  );
  await p.waitForTimeout(900);
  const yes = await p.$('button:has-text("Yes, column names")');
  if (yes) {
    await yes.click();
    await p.waitForTimeout(400);
  }
  const before = await state();

  // Ctrl+Shift+2 by the key's place.
  await select(1, 0);
  await p.keyboard.press("Control+Shift+Digit2");
  await p.waitForTimeout(250);
  const f0 = await fmt(0);
  ok(f0?.kind === "time" && f0.time === "hm12", `Ctrl+Shift+2 shows the column as time (${JSON.stringify(f0)})`);
  ok(/now show as time like 2:30 PM\. The file itself does not change\./.test(await status()), `the status says so (${(await status()).slice(0, 120)})`);

  // Ctrl+1, Custom code, on the phone column: the preview shows the code at work.
  await select(1, 1);
  await p.keyboard.press("Control+1");
  await p.locator(".shell[role=dialog]").waitFor({ timeout: 4000 });
  await p.locator(".shell select").first().selectOption("custom");
  const code = p.locator("input.code");
  await code.fill("000-000-0000");
  await p.waitForTimeout(200);
  const prev = ((await p.locator("table.preview").textContent()) || "").replace(/\s+/g, " ");
  ok(prev.includes("6155551234615-555-1234"), `the preview shows 615-555-1234 (${prev.slice(0, 120)})`);
  if (shots) await p.screenshot({ path: path.join(shots, "custom-code-dialog.png") });
  // A code it cannot show: refused, with the character it stops at, and OK greyed.
  await code.fill("0.0E+0");
  await p.waitForTimeout(200);
  const why = ((await p.locator(".shell .problem").textContent()) || "").trim();
  ok(why === "Not supported at character 4: scientific notation (E+).", `0.0E+0 is refused with its message (${why})`);
  ok(await p.getByRole("button", { name: "OK", exact: true }).isDisabled(), "OK is greyed while the code cannot be shown");
  await p.keyboard.press("Enter");
  await p.waitForTimeout(200);
  ok((await p.locator(".shell[role=dialog]").count()) === 1, "Enter does not take a code that cannot be shown");
  await code.fill("000-000-0000");
  await p.getByRole("button", { name: "OK", exact: true }).click();
  await p.waitForTimeout(250);
  ok((await fmt(1))?.code === "000-000-0000", "the phone column shows as 000-000-0000");

  // Red negatives, with a pixel probe; none under forced colours.
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.columns.formats.set(2, { kind: "custom", decimals: 2, thousands: true, symbol: "$", date: "iso", time: "hm12", code: "#,##0.00;[Red]-#,##0.00" });
    t.bump();
  });
  await p.waitForTimeout(300);
  const red = await redIn(1, 2);
  const notRed = await redIn(2, 2);
  ok(red > 10 && notRed === 0, `the negative is drawn red, the positive is not (${red} and ${notRed} red pixels)`);
  await p.emulateMedia({ forcedColors: "active" });
  await p.evaluate(() => window.__nde.grid.redraw());
  await p.waitForTimeout(300);
  const forced = await redIn(1, 2);
  ok(forced === 0, `under forced colours no colour is drawn (${forced} red pixels); the minus sign says it`);
  await p.emulateMedia({ forcedColors: "none" });
  await p.evaluate(() => window.__nde.grid.redraw());

  // None of that changed the file, its changed mark or its undo.
  const mid = await state();
  ok(JSON.stringify(mid) === JSON.stringify(before), `display formats change nothing in the file (${JSON.stringify([before.modified, before.pos, before.undo])} -> ${JSON.stringify([mid.modified, mid.pos, mid.undo])})`);

  // The key does nothing while a cell is being edited.
  await select(1, 3);
  await p.keyboard.press("F2");
  await p.waitForTimeout(200);
  await p.keyboard.press("Control+Shift+Digit2");
  await p.waitForTimeout(200);
  ok((await fmt(3)) === null, "Ctrl+Shift+2 in the cell editor leaves the column alone");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(200);

  // Write into cells, then Ctrl+Z gives back exactly what was there.
  await select(1, 1);
  await run("format.bakeDisplay");
  await p.waitForTimeout(600);
  const baked = await state();
  ok(baked.cells[5] === "615-555-1234" && baked.cells[9] === "615-555-0000", `Write into cells writes 615-555-1234 (${baked.cells[5]}, ${baked.cells[9]})`);
  ok((await fmt(1)) === null, "and the column's display format is taken off");
  await select(1, 1);
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(500);
  const undone = await state();
  ok(JSON.stringify(undone.cells) === JSON.stringify(before.cells), `Ctrl+Z gives back the cells exactly (${undone.cells[5]})`);

  ok(errors.length === 0, `no page errors (${errors.join("; ")})`);
  await b.close();
  console.log(failed ? `${failed} FAILED` : "ALL PASSED");
  process.exit(failed ? 1 : 0);
})();
