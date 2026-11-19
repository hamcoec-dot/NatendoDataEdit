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
  await command("Copy column names as a line of text");
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

  // 3. Copy with column names (Ctrl+Shift+C): names on top of the cells.
  await p.evaluate(() => window.__nde.grid.select({ row: 1, col: 0 }, { row: 2, col: 1 }));
  await wait(200);
  await p.keyboard.press("Control+Shift+C");
  await wait(500);
  const withNames = (await clipboard()).split(/\r?\n/).filter(Boolean);
  ok(withNames[0] === "id\tfirst_name" && withNames.length === 3, `Ctrl+Shift+C: names then the 2 rows (${JSON.stringify(withNames)})`);

  // 4. The right-click menus have it.
  const canvas = await (await p.$("canvas")).boundingBox();
  await p.mouse.click(canvas.x + 8, canvas.y + 8, { button: "right" });
  await wait(300);
  const corner = await p.$$eval(".context [role=menuitem]", (e) => e.map((x) => x.textContent.trim()));
  ok(corner.some((t) => /Copy column names/.test(t)) && corner.some((t) => /Paste as column names/.test(t)), "the corner's right-click menu has Copy and Paste as column names");
  await p.keyboard.press("Escape");
  await wait(200);
  if (out) await p.screenshot({ path: path.join(out, "copynames.png") });

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

  // 6. A list, one name per line, over a file with titles; undo puts them back.
  await setClipboard("id\r\nfull name\r\n");
  await command("Paste as column names");
  ok(JSON.stringify((await names()).slice(0, 3)) === JSON.stringify(["id", "full name", "place"]), `a list pastes one name per column (${await names()})`);
  await p.keyboard.press("Control+z");
  await wait(500);
  ok(JSON.stringify((await names()).slice(0, 3)) === JSON.stringify(["key", "name", "place"]), "Ctrl+Z puts the names back in one step");

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
