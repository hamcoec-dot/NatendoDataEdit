// UI test of "which rows does this change" in the browser stand-in: with rows hidden, the Sort screen says it sorts the
// shown rows and offers a tick for all, the quick sort says hidden rows stayed, a new computed column offers to fill
// them, and the tools that change every row say so; with nothing hidden none of it shows.
// Run: (cd ui && npx vite --port 5301) then NODE_PATH=<folder with playwright> node scripts/ui-shown.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5301";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1280, height: 820 } });
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
  const run = async (id) => {
    await p.evaluate((id) => window.__nde.reg.run(id), id);
    await p.waitForTimeout(900);
  };
  const dialog = p.getByRole("dialog").last();
  const note = async () => ((await p.getByTestId("hidden-note").first().textContent({ timeout: 3000 }).catch(() => "")) || "").replace(/\s+/g, " ").trim();
  const close = async () => {
    await p.keyboard.press("Escape");
    await p.waitForTimeout(400);
  };

  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => window.__nde.tab && window.__nde.tab.info.indexed && window.__nde.tab.info.rows > 1000, null, { timeout: 30000 });
  await p.waitForTimeout(800);
  await p.getByRole("button", { name: "Yes, column names" }).click({ timeout: 1500 }).catch(() => {});
  await run("data.sort");
  ok((await dialog.count()) === 1 && (await p.getByTestId("hidden-note").count()) === 0, "nothing is said while every row shows");
  await close();
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    window.__nde.grid.select({ row: 2, col: 1 }, { row: 4, col: 1 });
    t.bump();
  });
  await p.waitForTimeout(300);
  await run("format.hideRows");
  const hidden = await p.evaluate(() => window.__nde.tab.info.total_rows - window.__nde.tab.info.rows);
  ok(hidden === 3, `three rows are hidden (${hidden})`);

  await run("data.sort");
  ok(/^Sorting the [\d,]+ shown rows; 3 hidden rows stay where they are\.$/.test(await note()), `Sort says what it sorts (${await note()})`);
  const all = dialog.getByRole("checkbox", { name: /Sort all rows, including hidden ones/ });
  ok((await all.count()) === 1 && !(await all.isChecked()), "the tick for all rows is offered, off");
  if (out) await p.screenshot({ path: path.join(out, "shown-sort.png") });
  await all.check();
  await p.waitForTimeout(200);
  ok(/^Including hidden rows/.test(await note()), `ticked, it says hidden rows are included (${await note()})`);
  await close();

  await run("data.compute");
  const fill = dialog.getByRole("checkbox", { name: "Also fill the hidden rows" });
  ok((await fill.count()) === 1 && !(await fill.isChecked()) && /3 hidden rows left empty/.test(await note()), `a new computed column leaves hidden rows empty unless ticked (${await note()})`);
  await close();

  await run("data.splitColumn");
  ok(/This changes all [\d,]+ rows, including 3 hidden ones, because the source column is changed too\./.test(await note()), `Split column says it changes every row (${await note()})`);
  ok((await dialog.getByTestId("hidden-pill").textContent().catch(() => "")) === "Including hidden rows", "with an amber pill in the dialog");
  if (out) await p.screenshot({ path: path.join(out, "shown-split.png") });
  await close();

  // The status pill and the dotted line where rows are missing.
  const pill = async () => ((await p.getByTestId("acting-pill").textContent({ timeout: 3000 }).catch(() => "")) || "").trim();
  ok(/^Acting on [\d,]+ shown rows \(3 hidden, untouched\)$/.test(await pill()), `the pill says which rows edits act on (${await pill()})`);
  await run("data.sort");
  await dialog.getByRole("checkbox", { name: /Sort all rows/ }).check();
  await p.waitForTimeout(300);
  ok((await pill()) === "Including hidden rows (3 hidden)" && (await p.locator("[data-testid=acting-pill].including").count()) === 1, `amber while a screen includes hidden rows (${await pill()})`);
  await close();
  ok(/^Acting on/.test(await pill()), "and back when it closes");
  await p.evaluate(() => window.__nde.grid.select({ row: 1, col: 1 }, { row: 1, col: 1 }));
  await p.waitForTimeout(300);
  const box = await p.locator("canvas").first().boundingBox();
  let found = null;
  for (let y = box.y + 20; y < box.y + 110 && !found; y += 3) {
    await p.mouse.move(box.x + 25, y);
    await p.waitForTimeout(60);
    const tip = await p.evaluate(() => [...document.querySelectorAll("[title]")].map((e) => e.getAttribute("title")).find((t) => /hidden here/.test(t || "")) || "");
    if (tip) found = { y, tip };
  }
  ok(!!found && /^3 rows hidden here\. Click to show them\.$/.test(found.tip), `pointing at the dotted line says how many (${found?.tip})`);
  if (out) await p.screenshot({ path: path.join(out, "shown-seam.png") });
  if (found) await p.mouse.click(box.x + 25, found.y);
  await p.waitForTimeout(1200);
  ok((await p.evaluate(() => window.__nde.tab.info.total_rows - window.__nde.tab.info.rows)) === 0 && (await p.getByTestId("acting-pill").count()) === 0, "clicking it shows the rows again, and the pill goes");
  // Each end of the table, the line's colour, and a gap that is only filtered out.
  const hiddenNow = () => p.evaluate(() => window.__nde.tab.info.total_rows - window.__nde.tab.info.rows);
  const findSeam = async (from, to, re) => {
    for (let y = from; y < to; y += 3) {
      await p.mouse.move(box.x + 25, y);
      await p.waitForTimeout(150);
      const tip = await p.evaluate(() => [...document.querySelectorAll("[title]")].map((e) => e.getAttribute("title")).find((t) => /here/.test(t || "")) || "");
      if (re.test(tip)) return { y, tip };
    }
    return null;
  };
  const hide = async (row, count) => {
    await p.evaluate(([row, count]) => window.__nde.grid.select({ row, col: 1 }, { row: row + count - 1, col: 1 }), [row, count]);
    await p.waitForTimeout(300);
    await run("format.hideRows");
  };
  const showByClick = async (seam, name, count) => {
    await p.mouse.click(box.x + 25, seam.y);
    await p.waitForTimeout(1200);
    ok((await hiddenNow()) === 0, `${name}: clicking shows the rows again (${await hiddenNow()} hidden)`);
    await run("edit.undo");
    await p.waitForTimeout(600);
    ok((await hiddenNow()) === count, `${name}: one undo hides them again (${await hiddenNow()})`);
    await run("format.showAllRows");
  };
  await run("view.accent.amber");
  await hide(1, 2);
  await p.mouse.move(600, 400);
  await p.waitForTimeout(400);
  const first = await findSeam(box.y + 12, box.y + 45, /hidden here/);
  ok(!!first && /^2 rows hidden here/.test(first.tip), `before the first data row (${first?.tip})`);
  await p.mouse.move(600, 400);
  await p.waitForTimeout(300);
  const colours = await p.evaluate((y) => {
    const c = document.querySelector("canvas");
    const r = c.getBoundingClientRect();
    const k = c.width / r.width;
    const g = c.getContext("2d");
    const rgb = (css) => {
      const x = document.createElement("canvas").getContext("2d");
      x.fillStyle = css;
      x.fillRect(0, 0, 1, 1);
      return [...x.getImageData(0, 0, 1, 1).data].slice(0, 3);
    };
    const v = (n) => rgb(getComputedStyle(document.documentElement).getPropertyValue(n).trim());
    const near = (a, b) => a.every((q, i) => Math.abs(q - b[i]) < 24);
    const [text, accent] = [v("--text"), v("--accent")];
    let asText = 0;
    let asAccent = 0;
    // The dotted line's own pixels: two rows of pixels under the header row, left of the digits.
    for (let dy = 25; dy <= 27; dy++) {
      for (let x = 1; x < 30; x++) {
        const px = [...g.getImageData(Math.round(x * k), Math.round(dy * k), 1, 1).data].slice(0, 3);
        if (near(px, text)) asText++;
        if (near(px, accent)) asAccent++;
      }
    }
    return { asText, asAccent, accent };
  }, first?.y);
  ok(colours.asText >= 5 && colours.asAccent === 0, `with the Amber accent the line is in the text colour (${JSON.stringify(colours)})`);
  if (out) await p.screenshot({ path: path.join(out, "shown-seam-amber.png") });
  if (first) await showByClick(first, "first gap", 2);
  const last = await p.evaluate(() => window.__nde.tab.info.rows - 1);
  await hide(last - 1, 2);
  await p.evaluate(() => window.__nde.grid.select({ row: window.__nde.tab.info.rows - 1, col: 1 }, { row: window.__nde.tab.info.rows - 1, col: 1 }));
  await p.waitForTimeout(900);
  const end = await findSeam(box.y + box.height - 140, box.y + box.height, /hidden here/);
  ok(!!end && /^2 rows hidden here/.test(end.tip), `after the last row (${end?.tip})`);
  if (end) await showByClick(end, "last gap", 2);
  // A filter leaves the second data row out; a row far below is hidden by hand.
  await run("edit.goHome").catch(() => {});
  await p.evaluate(() => window.__nde.grid.select({ row: 0, col: 1 }, { row: 0, col: 1 }));
  await p.waitForTimeout(600);
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    t.update(await window.__nde.backend.setFilter(t.id, { columns: [{ col: 1, rule: { type: "conditions", first: { type: "text", op: "not_equals", text: "Bo" }, second: null, or: false } }], keep_first: true }), true);
    t.bump();
  });
  await p.waitForTimeout(700);
  await hide(6, 1);
  // The filter chips bar above the grid moves it down: measure it again.
  const fbox = await p.locator("canvas").first().boundingBox();
  const only = await findSeam(fbox.y + 40, fbox.y + 75, /filtered out here/);
  ok(!!only && only.tip === "1 row filtered out here. Click for the filter.", `a filter-only gap promises nothing hidden by hand (${only?.tip})`);
  const before = await hiddenNow();
  if (only) await p.mouse.click(box.x + 25, only.y);
  await p.waitForTimeout(1200);
  const menu = await p.getByRole("dialog").or(p.locator("[data-testid=filter-menu], .filter-menu")).count();
  ok(menu > 0 && (await hiddenNow()) === before, `clicking it opens the filter menu and clears nothing (menus ${menu}, ${await hiddenNow()} hidden)`);
  if (out) await p.screenshot({ path: path.join(out, "shown-filter-gap.png") });
  await close();
  await run("data.clearFilter");
  await run("format.showAllRows");
  // Hide the rows again for the quick sort (the stand-in does not keep hidden rows in place, so it comes last).
  await p.evaluate(() => window.__nde.grid.select({ row: 2, col: 1 }, { row: 4, col: 1 }));
  await run("format.hideRows");
  await p.evaluate(() => window.__nde.grid.select({ row: 1, col: 1 }, { row: 1, col: 1 }));
  await run("data.sortAsc");
  await p.waitForTimeout(1500);
  const said = ((await p.textContent("footer.status")) || "").replace(/\s+/g, " ");
  ok(/Sorted the shown rows; 3 hidden rows stayed in place/.test(said), `the quick sort says hidden rows stayed (${said.slice(0, 100)})`);
  ok(errors.length === 0, `no page errors (${errors.join("; ").slice(0, 200)})`);
  await b.close();
  process.exit(failed ? 1 : 0);
})();
