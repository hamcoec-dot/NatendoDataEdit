// UI test of Copy column names: copy the names (as a row, or a line of the
// file's delimiter), copy a selection with its names, and paste names as
// another file's title row.
// Run: (cd ui && npx vite --port 5209) then
// NODE_PATH=<folder with playwright> node scripts/ui-copynames.cjs [screenshot folder] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5209";
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
  const command = async (q) => {
    await p.keyboard.press("Alt+q");
    await p.keyboard.type(q);
    await wait(300);
    await p.keyboard.press("Enter");
    await wait(500);
  };
  const clipboard = () => p.evaluate(() => navigator.clipboard.readText());
  const setClipboard = (t) => p.evaluate((x) => navigator.clipboard.writeText(x), t);
  const names = () => p.evaluate(() => window.__nde.backend.firstRowNames(window.__nde.tab.id));
  const status = async () => (await p.textContent("footer.status .msg").catch(() => "")) || "";
  const ALL = ["id", "first_name", "last_name", "city", "amount", "joined", "notes"];

  await p.goto(url);
  await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });
  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => window.__nde?.tab?.info?.indexed, null, { timeout: 30000 });
  await wait(600);
  await p.locator(".header-ask button.close").click({ timeout: 2000 }).catch(() => {});
  await wait(300);

  // 1. All the names, as a row split by tabs, then as a line of the file.
  await command("Copy column names");
  ok((await clipboard()) === ALL.join("\t"), `Copy column names: one row split by tabs (${JSON.stringify(await clipboard())})`);
  ok(/Copied 7 column names/.test(await status()), "the status says how many");
  await command("Copy column names as a text line");
  ok((await clipboard()) === ALL.join(","), "as a line of text: split by the file's delimiter");

  // 2. Whole columns selected: just theirs.
  await p.evaluate(() => {
    const g = window.__nde.grid;
    const rows = window.__nde.tab.info.rows;
    g.select({ row: 0, col: 1 }, { row: rows - 1, col: 2 });
  });
  await wait(200);
  await command("Copy column names");
  ok((await clipboard()) === "first_name\tlast_name", `whole columns selected: their names (${JSON.stringify(await clipboard())})`);

  // 3. Copy with column names (Ctrl+Shift+C): names on top of the cells;
  // like any copy, it cancels a pending cut and shows the copy outline.
  await p.evaluate(() => window.__nde.grid.select({ row: 4, col: 1 }, { row: 5, col: 2 }));
  await p.keyboard.press("Control+x");
  await wait(500);
  ok(!!(await p.evaluate(() => window.__nde.tab.cut)), "(a cut is pending)");
  await p.evaluate(() => window.__nde.grid.select({ row: 1, col: 0 }, { row: 2, col: 1 }));
  await wait(200);
  await p.keyboard.press("Control+Shift+C");
  await wait(500);
  const withNames = (await clipboard()).split(/\r?\n/).filter(Boolean);
  ok(withNames[0] === "id\tfirst_name" && withNames.length === 3, `Ctrl+Shift+C: names then the 2 rows (${JSON.stringify(withNames)})`);
  ok(!(await p.evaluate(() => window.__nde.tab.cut)) && !!(await p.evaluate(() => window.__nde.tab.copied)), "it cancels the cut and shows the copy outline");

  // 4. The right-click menus have it.
  const canvas = await (await p.$("canvas")).boundingBox();
  await p.mouse.click(canvas.x + 8, canvas.y + 8, { button: "right" });
  await wait(300);
  const corner = await p.$$eval(".context [role=menuitem]", (e) => e.map((x) => x.textContent.trim()));
  ok(corner.some((t) => /Copy column names/.test(t)) && corner.some((t) => /Paste as column names/.test(t)), "the corner's right-click menu has Copy and Paste as column names");
  await p.keyboard.press("Escape");
  await wait(200);
  if (out) await p.screenshot({ path: path.join(out, "copynames.png") });

  // Nate's case: copy the names, a new file, right-click a cell, Paste.
  const cellMenu = async () => {
    const cv = await (await p.$("canvas")).boundingBox();
    await p.mouse.click(cv.x + 120, cv.y + 40, { button: "right" });
    await wait(300);
    return p.$$eval(".context [role=menuitem]", (e) => e.map((x) => x.textContent.trim()));
  };
  const pasteFromCellMenu = async (label) => {
    await cellMenu();
    await p.click(`.context [role=menuitem]:has-text('${label}')`);
    await wait(800);
  };
  const cols = () => p.evaluate(() => window.__nde.tab.info.cols);
  const row0 = () => p.evaluate(async () => (await window.__nde.backend.viewport(window.__nde.tab.id, 0, 1, 0, window.__nde.tab.info.cols, 200)).cells);
  for (const [what, cmd] of [["Copy column names", "Copy column names"], ["as a text line", "Copy column names as a text line"]]) {
    await p.evaluate(() => window.__nde.activate(0));
    await wait(300);
    await command(cmd);
    await p.keyboard.press("Control+n");
    await wait(700);
    await pasteFromCellMenu("Paste");
    ok((await cols()) === 7 && JSON.stringify(await row0()) === JSON.stringify(ALL), `${what}, new file, right-click Paste: one name per column (${await cols()} columns)`);
    await p.keyboard.press("Control+w");
    await wait(400);
    const d = p.locator(".dialog[role=alertdialog]");
    if (await d.count()) await d.getByRole("button", { name: /Don.t save/ }).click();
    await wait(300);
  }
  // The cell menu has Paste as column names; on a new, empty file it asks
  // nothing and the names are the title row (no blank row under it).
  await p.evaluate(() => window.__nde.activate(0));
  await command("Copy column names");
  await p.keyboard.press("Control+n");
  await wait(700);
  ok((await cellMenu()).some((t) => /Paste as column names/.test(t)), "the cell right-click menu has Paste as column names");
  await p.keyboard.press("Escape");
  await wait(200);
  await pasteFromCellMenu("Paste as column names");
  ok((await p.locator(".dialog[role=alertdialog]").count()) === 0, "a new, empty file: no question");
  ok((await cols()) === 7 && (await p.evaluate(() => window.__nde.tab.header)) === true && (await names()).join() === ALL.join(), "7 columns, the names are its title row");
  ok((await p.evaluate(() => window.__nde.tab.info.rows)) === 1, "and no blank row was added");
  await p.keyboard.press("Control+w");
  await wait(400);
  if (await p.locator(".dialog[role=alertdialog]").count()) await p.locator(".dialog[role=alertdialog]").getByRole("button", { name: /Don.t save/ }).click();
  await wait(300);

  // Someone else's comma line pasted into one cell: offered a split.
  await p.keyboard.press("Control+n");
  await wait(700);
  await setClipboard("north,south,east");
  await p.keyboard.press("Control+v");
  await wait(700);
  ok((await cols()) === 1, "another program's comma line goes into one cell");
  const split = p.locator("footer.status button:has-text('Split into columns')");
  ok((await split.count()) === 1, "and the status offers Split into columns");
  await split.click();
  await wait(600);
  ok((await cols()) === 3 && JSON.stringify(await row0()) === JSON.stringify(["north", "south", "east"]), "Split into columns: one part per column");
  await p.keyboard.press("Control+z");
  await wait(500);
  ok(JSON.stringify(await row0()) === JSON.stringify(["north,south,east"]), "Ctrl+Z puts the line back in one cell");
  await p.keyboard.press("Control+w");
  await wait(400);
  if (await p.locator(".dialog[role=alertdialog]").count()) await p.locator(".dialog[role=alertdialog]").getByRole("button", { name: /Don.t save/ }).click();
  await wait(300);
  await p.evaluate(() => window.__nde.activate(0));
  await wait(300);

  // 5. Paste as column names into a new file without a title row.
  await p.keyboard.press("Control+n");
  await wait(700);
  await setClipboard("a\tb\tc\r\n1\t2\t3\r\n");
  await p.keyboard.press("Control+v");
  await wait(700);
  const rowsBefore = await p.evaluate(() => window.__nde.tab.info.rows);
  ok((await p.evaluate(() => window.__nde.tab.header)) === false, "a new file has no title row");
  await setClipboard("key\tname\tplace");
  await command("Paste as column names");
  ok((await p.evaluate(() => window.__nde.tab.header)) === true, "Paste as column names turns the title row on");
  ok(JSON.stringify((await names()).slice(0, 3)) === JSON.stringify(["key", "name", "place"]), `the names are the new title row (${await names()})`);
  ok((await p.evaluate(() => window.__nde.tab.info.rows)) === rowsBefore + 1, "a title row was added; no data row was written over");

  // 6. A list, one name per line, over a file with titles, from the active
  // cell's column; undo puts them back.
  await p.evaluate(() => window.__nde.grid.select({ row: 1, col: 1 }));
  await setClipboard("full name\r\nplace\r\n");
  await command("Paste as column names");
  ok(JSON.stringify((await names()).slice(0, 3)) === JSON.stringify(["key", "full name", "place"]), `pasted from the active cell's column (${await names()})`);
  await p.keyboard.press("Control+z");
  await wait(500);
  await p.evaluate(() => window.__nde.grid.select({ row: 1, col: 0 }));
  await setClipboard("id\r\nfull name\r\n");
  await command("Paste as column names");
  ok(JSON.stringify((await names()).slice(0, 3)) === JSON.stringify(["id", "full name", "place"]), `a list pastes one name per column (${await names()})`);
  await p.keyboard.press("Control+z");
  await wait(500);
  ok(JSON.stringify((await names()).slice(0, 3)) === JSON.stringify(["key", "name", "place"]), "Ctrl+Z puts the names back in one step");

  // More names than columns: it asks first, and can paste only what fits.
  await p.evaluate(() => window.__nde.grid.select({ row: 1, col: 2 }));
  await setClipboard("c1\tc2\tc3");
  await command("Paste as column names");
  const dlg = p.locator(".dialog[role=alertdialog]");
  ok((await dlg.count()) === 1 && /2 names run past the last column/.test((await dlg.textContent()) ?? ""), "more names than columns: it asks first");
  await dlg.getByRole("button", { name: /Paste only/ }).click();
  await wait(500);
  ok((await p.evaluate(() => window.__nde.tab.info.cols)) === 3 && (await names())[2] === "c1", "Paste only what fits: no columns added");
  await p.keyboard.press("Control+z");
  await wait(400);
  await command("Paste as column names");
  await dlg.getByRole("button", { name: /Add 2 columns/ }).click();
  await wait(500);
  ok((await p.evaluate(() => window.__nde.tab.info.cols)) === 5 && /2 columns were added/.test(await status()), "Add columns: the file widens and says so");
  await p.keyboard.press("Control+z");
  await wait(400);

  // 7. No title row: nothing to copy, and it says why.
  await p.evaluate(() => {
    window.__nde.tab.header = false;
    window.__nde.tab.bump();
  });
  await setClipboard("unchanged");
  await command("Copy column names");
  ok((await clipboard()) === "unchanged" && /no column names/.test(await status()), "without a title row it copies nothing and says why");

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
