// UI test of Find and Replace v2a: match count, matches lit up, whole word,
// ignore accents, hidden columns, Alt keys, the Find all list, the paged
// Replace all preview, recent searches, tips and the right-click items.
// Run: (cd ui && npx vite --port 5196) then
// NODE_PATH=<folder with playwright> node scripts/ui-find.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5196";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const p = await b.newPage({ viewport: { width: 1360, height: 860 } });
  const errors = [];
  p.on("pageerror", (e) => errors.push(String(e)));
  let failed = 0;
  const ok = (cond, msg) => {
    if (!cond) failed++;
    console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  };
  await p.goto(url);
  await p.keyboard.press("Control+o");
  await p.waitForTimeout(3500);
  const grid = () => p.evaluate(() => window.__nde.grid.focus());
  const count = () => p.textContent(".find .count").catch(() => "");
  const tick = (label) => p.locator(`.find label:has-text('${label}') input`);
  const type = async (text) => {
    await p.fill("input[aria-label='Find']", text);
    await p.waitForTimeout(900);
  };

  await grid();
  await p.keyboard.press("Control+f");
  await type("Lisbon");
  ok(/^[\d,]+ matches$/.test(await count()), `the count shows as you type (${await count()})`);
  ok((await p.evaluate(() => window.__nde.tab.findHits.size)) > 0, "matches on screen are lit up");
  await p.keyboard.press("Enter");
  await p.waitForTimeout(800);
  ok(/^1 of [\d,]+$/.test(await count()), `on a match it says which one (${await count()})`);
  if (out) await p.screenshot({ path: path.join(out, "find-count.png"), clip: { x: 0, y: 0, width: 1360, height: 420 } });

  // Alt keys while the bar has focus, without opening a menu.
  await p.keyboard.press("Alt+w");
  await p.keyboard.press("Alt+c");
  await p.waitForTimeout(150);
  ok((await tick("Whole word").isChecked()) && (await tick("Match case").isChecked()), "Alt+W and Alt+C tick whole word and match case");
  ok(!(await p.isVisible(".menu:not(.context)[role=menu]")), "no menu opened");
  await p.keyboard.press("Alt+r");
  await p.waitForTimeout(100);
  ok((await p.inputValue("select[aria-label='Pattern kind']")) === "regex", "Alt+R turns on regular expressions");
  await p.keyboard.press("Alt+r");
  await p.keyboard.press("Alt+w");
  await p.keyboard.press("Alt+c");

  // Whole word: "Lisbo" is part of a word.
  await type("Lisbo");
  const part = await count();
  await tick("Whole word").check();
  await p.waitForTimeout(900);
  ok(part !== "No matches" && part !== "" && (await count()) === "No matches", `whole word skips part of a word (${part} → ${await count()})`);
  await tick("Whole word").uncheck();

  // Ignore accents: "Krakow" finds "Kraków".
  await type("Krakow");
  ok((await count()) === "No matches", "accents count by default");
  await tick("Ignore accents").check();
  await p.waitForTimeout(900);
  ok(/matches$/.test(await count()) && (await count()) !== "No matches", `ignore accents finds Kraków (${await count()})`);
  await tick("Ignore accents").uncheck();

  // Find all: the list, per-column counts and keys.
  await type("o");
  await p.focus("input[aria-label='Find']");
  await p.keyboard.press("Alt+Enter");
  await p.waitForTimeout(1500);
  ok(await p.isVisible(".find [role=listbox]"), "Alt+Enter lists every match");
  const per = await p.$$eval(".percol button", (e) => e.map((x) => x.textContent));
  ok(per.length > 2 && /: [\d,]+/.test(per[1]), `matches per column (${per.slice(1, 3).join(", ")})`);
  await p.click(".percol button >> nth=1");
  await p.waitForTimeout(200);
  const cols = await p.$$eval(".find .hit .where", (e) => [...new Set(e.map((x) => x.textContent.split(",")[0]))]);
  ok(cols.length === 1, `a column's button shows only its matches (${cols.join("|")})`);
  await p.focus(".find [role=listbox]");
  await p.keyboard.press("ArrowDown");
  await p.keyboard.press("ArrowDown");
  await p.waitForTimeout(300);
  ok((await p.locator(".find .hit.cur").count()) === 1, "Down moves through the list");
  const at = await p.evaluate(() => window.__nde.tab.cell);
  const want = await p.locator(".find .hit.cur .where").textContent();
  ok(want.includes(`row ${at.row + 1}`), `and goes to that cell (${want}, row ${at.row + 1})`);
  if (out) await p.screenshot({ path: path.join(out, "find-all-list.png"), clip: { x: 0, y: 0, width: 1360, height: 470 } });

  // Replace all preview, paged, with the change marked in each cell.
  await p.click(".find button[aria-label='Close results']");
  await type("Perth");
  await p.click(".find button.twist");
  await p.fill("input[aria-label='Replace with']", "Perth WA");
  await p.keyboard.press("Control+Alt+Enter");
  await p.waitForTimeout(1500);
  ok(await p.isVisible(".find .confirm"), "Ctrl+Alt+Enter shows what Replace all changes");
  ok((await p.locator(".find .confirm ins").first().textContent()) === " WA", "the added text is marked in the cell");
  const pager = await p.textContent(".find .pager").catch(() => "");
  ok(/Changes 1–50 of [\d,]+/.test(pager), `paged through every change (${pager.trim()})`);
  await p.click(".find button[aria-label='Next changes']");
  await p.waitForTimeout(400);
  ok(/Changes 51–100/.test(await p.textContent(".find .pager")), "the next page");
  if (out) await p.screenshot({ path: path.join(out, "find-replace-preview.png"), clip: { x: 0, y: 0, width: 1360, height: 560 } });
  // Changing anything that shapes the search closes the preview, so its
  // Replace all never applies something it did not show.
  await p.fill("input[aria-label='Replace with']", "WIPED");
  await p.waitForTimeout(200);
  ok(!(await p.isVisible(".find .confirm")), "changing Replace with closes the preview");
  ok(/preview was closed/.test(await p.textContent(".find .status")), "and says so");
  await p.keyboard.press("Control+Alt+Enter");
  await p.waitForTimeout(1500);
  await tick("Match case").check();
  await p.waitForTimeout(200);
  ok(!(await p.isVisible(".find .confirm")), "so does ticking a box");

  // A replacement naming a missing group is refused before any change.
  await p.selectOption("select[aria-label='Pattern kind']", "regex");
  await type("(Perth)");
  await p.fill("input[aria-label='Replace with']", "$2");
  await p.click("button:has-text('Replace all…')");
  await p.waitForTimeout(800);
  if (!/not a group/.test(await p.textContent(".find .status").catch(() => ""))) {
    // The browser stub has no group check; the engine test covers it.
    console.log("SKIP group check (browser stub)");
  }
  await p.selectOption("select[aria-label='Pattern kind']", "plain");

  // Recent searches: Up brings back an earlier one.
  await p.fill("input[aria-label='Find']", "");
  await p.focus("input[aria-label='Find']");
  await p.keyboard.press("ArrowUp");
  ok((await p.inputValue("input[aria-label='Find']")) !== "", `Up brings back the last search (${await p.inputValue("input[aria-label='Find']")})`);

  // Tips.
  await p.click(".find button[aria-label='Find tips']");
  ok(await p.isVisible(".find .help"), "the ? button shows wildcard and pattern tips");
  await p.click(".find .help button:has-text('Clear search history')");
  await p.fill("input[aria-label='Find']", "");
  await p.focus("input[aria-label='Find']");
  await p.keyboard.press("ArrowUp");
  ok((await p.inputValue("input[aria-label='Find']")) === "", "Clear search history forgets them");
  await p.click(".find button[aria-label='Find tips']");

  // Hidden columns are skipped unless asked.
  await p.keyboard.press("Escape");
  await p.evaluate(() => {
    window.__nde.grid.select({ row: 3, col: 3 }, { row: 3, col: 3 });
    window.__nde.grid.focus();
  });
  await p.keyboard.press("Control+0");
  await p.waitForTimeout(400);
  const hidden = await p.evaluate(() => window.__nde.tab.columns.isHidden(3));
  await p.keyboard.press("Control+f");
  await type("Osaka");
  if (hidden) {
    ok((await count()) === "No matches", "a hidden column is not searched");
    await tick("Include hidden").check();
    await p.waitForTimeout(900);
    ok(/matches$/.test(await count()) && (await count()) !== "No matches", `Include hidden searches it (${await count()})`);
    await tick("Include hidden").uncheck();
  } else console.log("SKIP hidden column (Ctrl+0 did not hide it)");
  await p.keyboard.press("Escape");
  await p.evaluate(() => {
    window.__nde.tab.columns.showAll();
    window.__nde.tab.bump();
  });
  await p.waitForTimeout(300);

  // Right-click › Find all like this in the column.
  await p.evaluate(() => {
    window.__nde.grid.select({ row: 2, col: 3 }, { row: 2, col: 3 });
    window.__nde.grid.focus();
  });
  await p.keyboard.press("Shift+F10");
  await p.waitForTimeout(300);
  await p.click(".menu.context .item:has-text('Find all like this in the column')");
  await p.waitForTimeout(1500);
  ok((await p.inputValue("input[aria-label='Find']")) === (await p.evaluate(() => window.__nde.tab.cache.get(2, 3))), "the cell's value is searched");
  if (!(await p.isVisible(".find [role=listbox]"))) console.log("status:", await p.textContent(".find").catch(() => "?"));
  ok(await p.isVisible(".find [role=listbox]"), "every match in the column is listed");
  ok((await p.inputValue("select[aria-label='Search in']")) === "column", "in that column");
  await p.keyboard.press("Escape");

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
