// UI test of v0.4.2 (power-user batch 2): cell formulas typed in a cell
// (with Ctrl+Enter fills and the remembered formula), the formula helper,
// filter by formula and change a column with a formula.
// Run: (cd ui && npx vite --port 5201) then
// NODE_PATH=<folder with playwright> node scripts/ui-batch2.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5201";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1360, height: 860 }, permissions: ["clipboard-read", "clipboard-write"] });
  const p = await ctx.newPage();
  const errors = [];
  p.on("pageerror", (e) => errors.push(String(e)));
  let failed = 0;
  const ok = (cond, msg) => {
    if (!cond) failed++;
    console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  };
  const wait = (ms) => p.waitForTimeout(ms);
  const clip = (t) => p.evaluate((x) => navigator.clipboard.writeText(x), t);
  const cell = (r, c) => p.evaluate(([a, b2]) => window.__nde.tab.cache.get(a, b2), [r, c]);
  const status = () => p.textContent("footer.status").catch(() => "");
  const sel = (r, c, r2 = r, c2 = c) =>
    p.evaluate(([a, b2, c3, d]) => {
      window.__nde.grid.select({ row: a, col: b2 }, { row: c3, col: d });
      window.__nde.grid.focus();
    }, [r, c, r2, c2]).then(() => wait(200));
  const command = async (q) => {
    await p.keyboard.press("Alt+q");
    await p.keyboard.type(q);
    await wait(300);
    await p.keyboard.press("Enter");
    await wait(500);
  };
  const dialog = (title) => p.locator(`[role=dialog]:has(h2:has-text('${title}'))`);
  const typeIn = async (r, c, text, key = "Enter") => {
    await sel(r, c);
    await p.keyboard.type(text);
    await wait(200);
    await p.keyboard.press(key);
    await wait(500);
  };

  await p.goto(url);
  await p.keyboard.press("Control+n");
  await wait(700);
  await sel(0, 0);
  await clip(["Item\tPrice\tQty\tTotal", "Pen\t2\t3\t", "Ink\t1.5\t4\t", "Pad\t4\t1\t", "Cup\t3\t5\t"].join("\r\n") + "\r\n");
  await p.keyboard.press("Control+v");
  await wait(700);
  await p.evaluate(() => {
    window.__nde.tab.header = true;
    window.__nde.tab.bump();
  });

  // ---- A formula in one cell ----
  await typeIn(1, 3, "=B2*C2");
  ok((await cell(1, 3)) === "6", `=B2*C2 leaves its result (${await cell(1, 3)})`);
  await sel(1, 3);
  await p.keyboard.press("F2");
  await wait(400);
  ok((await p.inputValue("textarea.editor").catch(() => "")) === "=B2*C2", "F2 opens the cell's formula");
  await p.keyboard.press("Escape");
  await wait(200);
  const bar = await p.inputValue("textarea[aria-label='Cell contents'], .cellbar textarea").catch(() => "");
  ok(bar === "=B2*C2", `the cell bar shows the formula (${bar})`);

  // ---- Ctrl+Enter fills it, references moving ----
  await sel(2, 3, 4, 3);
  await p.keyboard.type("=B3*C3 + $B$2");
  await wait(200);
  await p.keyboard.press("Control+Enter");
  await wait(600);
  ok((await cell(2, 3)) === "8" && (await cell(3, 3)) === "6" && (await cell(4, 3)) === "17", `filled with moved references (${await cell(2, 3)}, ${await cell(3, 3)}, ${await cell(4, 3)})`);
  ok(/Filled 3 cells/.test(await status()), "the status says how many were filled");
  await p.keyboard.press("Control+z");
  await wait(500);
  ok((await cell(3, 3)) === "", "one undo takes the whole fill back");

  // ---- Ranges, text kept with an apostrophe, a bad formula ----
  await typeIn(4, 3, "=SUM(B2:B5)");
  ok((await cell(4, 3)) === "10.5", `=SUM(B2:B5) (${await cell(4, 3)})`);
  await typeIn(3, 3, "'=not a formula");
  ok((await cell(3, 3)) === "=not a formula", "a leading apostrophe keeps the text");
  await sel(1, 3, 2, 3);
  await clip("=pasted");
  await p.keyboard.press("Control+v");
  await wait(500);
  ok((await cell(1, 3)) === "=pasted" && (await cell(2, 3)) === "=pasted", "pasted text that starts with = stays text");
  await p.keyboard.press("Control+z");
  await wait(400);
  await typeIn(2, 3, "=B3 +");
  ok(/doesn't work/.test(await status()), "a broken formula is refused with a message");
  await p.keyboard.press("Escape");
  await wait(300);

  // ---- The formula helper in a cell ----
  await sel(2, 3);
  await p.keyboard.type("=ROUND(AVER");
  await wait(300);
  const help = await p.textContent(".fxhelp").catch(() => "");
  ok(/AVERAGE/.test(help) && /ROUND\(number/.test(help), `the helper suggests AVERAGE inside ROUND (${help})`);
  if (out) await p.screenshot({ path: path.join(out, "cell-formula.png"), clip: { x: 0, y: 0, width: 760, height: 260 } });
  await p.keyboard.press("Tab");
  await wait(200);
  ok((await p.inputValue("textarea.editor").catch(() => "")) === "=ROUND(AVERAGE(", "Tab puts the suggestion in");
  await p.keyboard.type("B2:B5), 1)");
  await p.keyboard.press("Enter");
  await wait(500);
  ok((await cell(2, 3)) === "2.6", `=ROUND(AVERAGE(B2:B5), 1) (${await cell(2, 3)})`);

  // ---- Filter by formula ----
  await command("Filter by formula");
  const fd = dialog("Filter by formula");
  ok(await fd.isVisible(), "Filter by formula opens");
  await fd.locator("textarea").fill("[Price] > 2");
  await wait(500);
  if (out) await fd.screenshot({ path: path.join(out, "filter-formula.png") });
  await fd.locator("button.primary").click();
  await wait(700);
  const info = await p.evaluate(() => window.__nde.tab.info);
  // (The browser stub always shows rows pasted into a new tab, so only the
  // filter's being on is checked here; the engine test checks the rows.)
  ok(info.filtered, "the filter is on");
  ok(/Showing 2 rows/.test(await status()), "the status says how many show");
  await command("Clear filter");
  await wait(400);

  // ---- Change a column with a formula ----
  await sel(1, 0);
  await command("Change column with a formula");
  const cd = dialog("Change a column with a formula");
  ok(await cd.isVisible(), "Change column with a formula opens");
  ok((await cd.locator("textarea").inputValue()) === "[Item]", "it starts with the column itself");
  await cd.locator("textarea").fill("UPP");
  await cd.locator("textarea").press("End");
  await wait(300);
  ok(await cd.locator(".suggest button:has-text('UPPER')").isVisible(), "the helper suggests UPPER");
  await cd.locator("textarea").press("Tab");
  await p.keyboard.type("[Item])");
  await wait(500);
  await cd.locator("button.primary").click();
  await wait(700);
  ok((await cell(1, 0)) === "PEN" && (await cell(4, 0)) === "CUP", `the column changed in place (${await cell(1, 0)})`);

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
