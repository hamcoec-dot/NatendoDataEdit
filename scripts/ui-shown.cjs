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
