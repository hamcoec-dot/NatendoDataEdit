// UI test of Find and Replace v2b: blank cells, Special…, column titles,
// the column picker, unticking changes, Show changed rows, acting on the rows
// Find all found, all open tabs, notes, the status bar count and Replace
// from a list.
// Run: (cd ui && npx vite --port 5198) then
// NODE_PATH=<folder with playwright> node scripts/ui-findb.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5198";
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
  const info = () => p.evaluate(() => ({ rows: window.__nde.tab.info.rows, total: window.__nde.tab.info.total_rows, name: window.__nde.tab.info.name }));
  const count = () => p.textContent(".find .count").catch(() => "");
  const fstatus = () => p.textContent(".find .status").catch(() => "");
  const footer = () => p.textContent("footer").catch(() => "");
  const tick = (label) => p.locator(".find label", { hasText: label }).locator("input");
  const type = async (text) => {
    await p.fill("input[aria-label='Find']", text);
    await wait(900);
  };
  const grid = () => p.evaluate(() => window.__nde.grid.focus());
  const sel = (r, c, r2 = r, c2 = c) =>
    p.evaluate(([a, b2, c3, d]) => {
      window.__nde.grid.select({ row: a, col: b2 }, { row: c3, col: d });
      window.__nde.grid.focus();
    }, [r, c, r2, c2]).then(() => wait(200));
  const newTable = async (text) => {
    await p.keyboard.press("Control+n");
    await wait(700);
    await sel(0, 0);
    await clip(text);
    await p.keyboard.press("Control+v");
    await wait(700);
    await p.evaluate(() => {
      window.__nde.tab.header = true;
      window.__nde.tab.bump();
    });
    await wait(200);
  };
  const closeTab = async () => {
    await grid();
    await p.keyboard.press("Control+w");
    await wait(400);
    const d = await p.$("[role=dialog] button:has-text(\"Don't save\"), [role=alertdialog] button:has-text(\"Don't save\")");
    if (d) await d.click();
    await wait(400);
  };

  await p.goto(url);
  await newTable("name,state,code\r\nAnn,TN,1\r\nBob,GA,\r\nCy,TN,3\r\nDee,AL,4\r\n".replace(/,/g, "\t"));
  ok((await info()).rows === 5, "the table is pasted in");

  // Blank cells, and the status bar count.
  await sel(1, 0);
  await p.keyboard.press("Control+h");
  await wait(300);
  await p.selectOption("select[aria-label='Pattern kind']", "blank");
  await wait(900);
  ok((await count()) === "1 match", `Blank cells finds the empty cell (${await count()})`);
  ok(/Find: 1\b/.test(await footer()), "the status bar shows Find: 1");
  await p.selectOption("select[aria-label='Pattern kind']", "plain");

  // Special…: a non-breaking space goes in as \u{A0} with \t \n ticked.
  await type("");
  await p.click(".find button:has-text('Special…')");
  await wait(200);
  ok(await p.isDisabled(".find .menu button:has-text('Any digit')"), "regex-only items wait for Regular expression");
  if (out) await p.screenshot({ path: path.join(out, "find-special.png"), clip: { x: 0, y: 0, width: 1000, height: 380 } });
  await p.click(".find .menu button:has-text('Non-breaking space')");
  await wait(300);
  ok((await p.inputValue("input[aria-label='Find']")) === "\\u{A0}" && (await tick("\\t \\n").isChecked()), "Special… puts in \\u{A0} and ticks \\t \\n");
  await tick("\\t \\n").uncheck();

  // Column titles: not searched unless ticked; replacing there renames.
  await type("state");
  ok((await count()) === "No matches", "titles are not searched by default");
  await tick("Column titles").check();
  await wait(900);
  ok((await count()) === "1 match", `with Column titles ticked the title matches (${await count()})`);
  await p.fill("input[aria-label='Replace with']", "region");
  await p.click(".find button:has-text('Replace all…')");
  await wait(700);
  ok(/rename 1 column/.test(await p.textContent(".find .confirm")), "the preview says a column is renamed");
  await p.click(".find .confirm button.primary");
  await wait(900);
  ok((await cell(0, 1)) === "region", `the column was renamed (${await cell(0, 1)})`);
  await tick("Column titles").uncheck();

  // The column picker: nothing ticked asks first.
  await p.selectOption("select[aria-label='Search in']", "pick");
  await wait(300);
  ok(await p.isVisible(".find .picker"), "Columns… opens the column picker");
  await type("TN");
  await p.keyboard.press("Enter");
  await wait(400);
  ok(/Tick the columns/.test(await fstatus()), "with no column ticked it asks for one");
  await p.click(".find .picker label:has-text('region') input");
  await wait(200);
  if (out) await p.screenshot({ path: path.join(out, "find-columns.png"), clip: { x: 0, y: 0, width: 1360, height: 420 } });
  await p.click(".find .picker button:has-text('Done')");
  await wait(900);
  ok((await count()) === "2 matches", `only the picked column is searched (${await count()})`);

  // Untick a change, then Show changed rows.
  await p.fill("input[aria-label='Replace with']", "Tennessee");
  await p.click(".find button:has-text('Replace all…')");
  await wait(700);
  const boxes = p.locator(".find .confirm input[type=checkbox]");
  ok((await boxes.count()) === 2, "the preview has a tick box per change");
  await boxes.nth(0).uncheck();
  await wait(200);
  ok(/Replace 2 matches in 1 cell/.test(await p.textContent(".find .confirm")) || /in 1 cell/.test(await p.textContent(".find .confirm")), "the question counts the ticked changes");
  await p.click(".find .confirm button.primary");
  await wait(900);
  ok((await cell(1, 1)) === "TN" && (await cell(3, 1)) === "Tennessee", `the unticked change was left as it was (${await cell(1, 1)}, ${await cell(3, 1)})`);
  ok(/1 change left out/.test(await fstatus()), "the status says one change was left out");
  await p.click(".find button:has-text('Show changed rows')");
  await wait(900);
  ok((await info()).rows === 2, `Show changed rows shows the changed row and the titles (${(await info()).rows})`);
  await grid();
  await p.keyboard.press("Control+z");
  await wait(600);
  ok((await info()).rows === 5, "undo shows every row again");

  // Act on the rows Find all found.
  await p.selectOption("select[aria-label='Search in']", "all");
  await type("GA");
  await p.focus("input[aria-label='Find']");
  await p.keyboard.press("Alt+Enter");
  await wait(900);
  ok(await p.isVisible(".find .acts"), "Find all offers actions on the rows");
  if (out) await p.screenshot({ path: path.join(out, "find-row-actions.png"), clip: { x: 0, y: 0, width: 1360, height: 460 } });
  await p.click(".find .acts button:has-text('Show only these')");
  await wait(900);
  ok((await info()).rows === 2, "Show only these hides the other rows");
  await grid();
  await p.keyboard.press("Control+z");
  await wait(600);
  await p.focus("input[aria-label='Find']");
  await p.keyboard.press("Alt+Enter");
  await wait(900);
  await p.click(".find .acts button:has-text('Delete rows…')");
  await wait(200);
  ok(/Delete 1 row\?/.test(await p.textContent(".find .acts")), "Delete rows asks first");
  await p.click(".find .acts button.danger");
  await wait(900);
  ok((await info()).total === 4, "the row with a match was deleted");
  await grid();
  await p.keyboard.press("Control+z");
  await wait(600);
  ok((await info()).total === 5, "undo puts it back");
  await p.focus("input[aria-label='Find']");
  await p.keyboard.press("Alt+Enter");
  await wait(900);
  const before = await p.evaluate(() => window.__nde.tabs?.length ?? 0);
  await p.click(".find .acts button:has-text('Copy to new tab')");
  await wait(1000);
  const nt = await info();
  ok(nt.rows === 2 && /^Matches/.test(nt.name), `Copy to new tab makes "${nt.name}" with the row and its titles (${nt.rows})`);
  await closeTab();
  void before;

  // A change after Find all closes the list, so its row actions never act
  // on rows it doesn't show.
  await p.focus("input[aria-label='Find']");
  await p.keyboard.press("Alt+Enter");
  await wait(900);
  ok(await p.isVisible(".find .acts"), "Find all is open again");
  await p.evaluate(() => window.__nde.backend.setCells(window.__nde.tab.id, [[4, 1, "GA"]]).then((i) => window.__nde.tab.update(i, true)));
  await wait(600);
  ok(!(await p.isVisible(".find .acts")) && /Find all list was closed/.test(await fstatus()), `a change closes the list (${await fstatus()})`);
  await grid();
  await p.keyboard.press("Control+z");
  await wait(600);

  // Copy to new tab leaves a hidden column out, and says so.
  await sel(1, 2);
  await p.keyboard.press("Control+0");
  await wait(400);
  await p.focus("input[aria-label='Find']");
  await p.keyboard.press("Alt+Enter");
  await wait(900);
  await p.click(".find .acts button:has-text('Copy to new tab')");
  await wait(1000);
  const hc = await p.evaluate(() => window.__nde.tab.info.cols);
  ok(hc === 2 && /1 hidden column left out/.test(await fstatus()), `the hidden column is left out of the new tab (${hc} columns; ${await fstatus()})`);
  await closeTab();
  await p.evaluate(() => {
    window.__nde.tab.columns.showAll();
    window.__nde.tab.bump();
  });
  await wait(300);

  // Flag says how to take flags off (Ctrl+Z doesn't).
  await p.focus("input[aria-label='Find']");
  await p.keyboard.press("Alt+Enter");
  await wait(900);
  await p.click(".find .acts button:has-text('Flag')");
  await wait(700);
  ok(/Ctrl\+Z doesn't take them off/.test(await fstatus()), "Flag says Ctrl+Z doesn't take flags off");

  // All open tabs: Find all and Replace all across them.
  await newTable("place,state\r\nOak,TN\r\nElm,KY\r\n".replace(/,/g, "\t"));
  await sel(1, 0);
  await p.keyboard.press("Control+h");
  await wait(300);
  await p.selectOption("select[aria-label='Search in']", "tabs");
  await type("Tennessee");
  await p.fill("input[aria-label='Find']", "TN");
  await wait(600);
  await p.focus("input[aria-label='Find']");
  await p.keyboard.press("Alt+Enter");
  await wait(1200);
  ok(/in 2 tabs/.test(await p.textContent(".find .rhead")), `Find all lists matches in both tabs (${await p.textContent(".find .rhead")})`);
  await p.fill("input[aria-label='Replace with']", "Tenn.");
  await p.click(".find button:has-text('Replace all…')");
  await wait(1200);
  ok(/across 2 tabs/.test(await p.textContent(".find .confirm")), "Replace all shows both tabs' parts");
  // A change to the other tab closes the preview.
  await p.evaluate(async () => {
    const t = window.__nde.tabs[0];
    const i = await window.__nde.backend.setCells(t.id, [[4, 0, "Dee2"]]);
    t.update(i, true);
  });
  await wait(600);
  ok(!(await p.isVisible(".find .confirm")) && /A tab changed/.test(await fstatus()), `a change to another tab closes the preview (${await fstatus()})`);
  await p.click(".find button:has-text('Replace all…')");
  await wait(1200);
  await p.click(".find .confirm button.primary");
  await wait(1200);
  ok((await cell(1, 1)) === "Tenn." && /in 2 tabs/.test(await fstatus()), `both tabs were changed (${await fstatus()})`);
  await p.selectOption("select[aria-label='Search in']", "all");

  // Row notes: Look in Notes finds them, Find next goes to the row.
  await p.evaluate(async () => {
    const b = window.__nde.backend;
    const id = window.__nde.tab.id;
    await b.noteSet(id, 1, 0, "Call back on Tuesday");
    await b.noteSet(id, 2, 2, "moved away");
  });
  await p.selectOption("select[aria-label='Look in']", "notes");
  await type("zzz");
  await p.focus("input[aria-label='Find']");
  await p.keyboard.press("Alt+Enter");
  await wait(600);
  ok(/No notes match/.test(await fstatus()), "a note search with no match says so");
  await type("tuesday");
  await sel(0, 1);
  await p.focus("input[aria-label='Find']");
  await p.keyboard.press("Enter");
  await wait(600);
  ok((await p.evaluate(() => window.__nde.tab.cell.row)) === 1 && /Call back on Tuesday/.test(await fstatus()), `Find next goes to the row with the note (${await fstatus()})`);
  await type("a");
  await p.focus("input[aria-label='Find']");
  await p.keyboard.press("Alt+Enter");
  await wait(600);
  ok((await p.$$eval(".find .nlist .hit", (e) => e.length)) === 2, "Find all lists every matching note");
  await p.selectOption("select[aria-label='Look in']", "cells");
  await closeTab();

  // Replace from a list.
  await sel(1, 0);
  await p.keyboard.press("Control+h");
  await wait(300);
  await p.click(".find button:has-text('From a list…')");
  await wait(400);
  await p.fill("textarea[aria-label='Pairs to replace']", "Ann\tAnne\nBob\tRobert\nAn\tXX");
  await wait(200);
  await p.click("[role=dialog] button:has-text('Show what changes')");
  await wait(900);
  ok(/2 cells/.test(await p.textContent("[role=dialog]")), "whole cells: Ann and Bob change, not part of a name");
  if (out) await p.screenshot({ path: path.join(out, "replace-from-list.png") });
  await p.click("[role=dialog] button.primary");
  await wait(900);
  ok((await cell(1, 0)) === "Anne" && (await cell(2, 0)) === "Robert", `the list was applied (${await cell(1, 0)}, ${await cell(2, 0)})`);
  await grid();
  await p.keyboard.press("Control+z");
  await wait(600);
  ok((await cell(1, 0)) === "Ann", "one undo step takes it all back");
  // "find, replace" lines (no tab anywhere): the spaces after the comma go.
  await p.click(".find button:has-text('From a list…')");
  await wait(400);
  await p.fill("textarea[aria-label='Pairs to replace']", "Ann, Anne");
  await p.click("[role=dialog] button:has-text('Show what changes')");
  await wait(900);
  await p.click("[role=dialog] button.primary");
  await wait(900);
  ok((await cell(1, 0)) === "Anne", `a comma list drops the space after the comma (${JSON.stringify(await cell(1, 0))})`);

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
