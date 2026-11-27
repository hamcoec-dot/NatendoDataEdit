// UI test of three Excel/Sheets reflexes (made-up data):
// - Ctrl+Shift+V pastes into the grid as Ctrl+V does (one undo step), and in the Find box it is the box's own paste;
// - Alt+= (AutoSum) turns on the totals row with Sum under the number columns, keeps text columns counted, matches
//   the "=" place on other layouts, does not open the menu bar when Alt is let go, hides the row on a second press
//   unless a chosen column shows something else;
// - the tab menu's Make a copy of this tab (an unsaved copy beside the original, its edits and look included, named
//   "(copy)", "(copy 2)") and Tab colour (a line Windows high contrast keeps, said to screen readers).
// Run: (cd ui && npx vite --port 5321) then
// NODE_PATH=<folder with playwright> node scripts/ui-reflexes.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5321";
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
  await p.goto(url);
  await wait(600);
  const nde = (f, arg) => p.evaluate(f, arg);
  const status = async () => ((await p.textContent("footer.status .msg").catch(() => "")) || "").replace(/\s+/g, " ").trim();
  const sel = async (r, c, r2 = r, c2 = c) => {
    await nde(([a, b2, c3, d]) => {
      window.__nde.grid.select({ row: a, col: b2 }, { row: c3, col: d });
      window.__nde.grid.focus();
    }, [r, c, r2, c2]);
    await wait(150);
  };
  const cell = (r, c) => nde(([a, b2]) => window.__nde.tab.cache.get(a, b2), [r, c]);
  const hist = () => nde(() => window.__nde.tab.info.history_pos);
  const rows = [
    ["name", "city", "amount", "votes"],
    ["Ann", "Ames", "10.50", "3"],
    ["Bo", "Bly", "4.25", "5"],
    ["Cy", "Cody", "7", "2"],
    ["Di", "Dell", "12", "8"],
  ];
  await nde((r) => {
    const d = window.__nde.backend.mockRowsTab("precincts.csv", r);
    window.__nde.addTab(d);
  }, rows);
  await wait(800);
  // Title row: yes.
  const yes = p.getByRole("button", { name: "Yes, column names" });
  if (await yes.isVisible().catch(() => false)) await yes.click();
  await wait(300);
  ok((await nde(() => window.__nde.tab.header)) === true, "the made-up table has a title row");

  // ---- R1: Ctrl+Shift+V ----
  await sel(1, 0);
  await p.evaluate(() => navigator.clipboard.writeText("Eve\tElk\r\n"));
  let h = await hist();
  await p.keyboard.press("Control+Shift+v");
  await wait(600);
  ok((await cell(1, 0)) === "Eve" && (await cell(1, 1)) === "Elk", "Ctrl+Shift+V pastes into the grid");
  ok((await hist()) === h + 1, "as one undo step (not two)");
  await p.keyboard.press("Control+z");
  await wait(500);
  ok((await cell(1, 0)) === "Ann", "Ctrl+Z takes it out");
  // In the Find box it is the box's own paste.
  await p.keyboard.press("Control+f");
  await wait(400);
  await p.evaluate(() => navigator.clipboard.writeText("Cody"));
  h = await hist();
  await p.focus("input[aria-label=Find]");
  await p.keyboard.press("Control+Shift+v");
  await wait(400);
  ok((await p.inputValue("input[aria-label=Find]")) === "Cody", "Ctrl+Shift+V in the Find box pastes there");
  ok((await hist()) === h && (await cell(1, 0)) === "Ann", "and no cell changes");
  await p.keyboard.press("Escape");
  await wait(300);

  // ---- R2: Alt+= ----
  // amount and votes are numbers; city is text.
  await sel(1, 1, 3, 3);
  await p.keyboard.down("Alt");
  await p.keyboard.press("Equal");
  await p.keyboard.up("Alt");
  await wait(500);
  ok((await nde(() => window.__nde.tab.totalsOn)) === true, "Alt+= turns on the totals row");
  ok((await p.$$eval("[role=menu]", (m) => m.length)) === 0, "letting go of Alt does not open the menu bar");
  // The totals arrive, then the columns waited on settle.
  await wait(900);
  const choices = () => nde(() => [...window.__nde.tab.columns.totals.entries()].sort((a, b) => a[0] - b[0]));
  let ch = await choices();
  ok(JSON.stringify(ch) === JSON.stringify([[2, "sum"], [3, "sum"]]), `Sum under amount and votes only (${JSON.stringify(ch)})`);
  let words = await status();
  ok(/Sum of amount and votes/.test(words) && /city holds no numbers, so it shows a count/.test(words), `the status bar says so (${words})`);
  if (out) await p.screenshot({ path: path.join(out, "autosum.png") });
  // The second press hides it.
  await p.keyboard.press("Alt+Equal");
  await wait(400);
  ok((await nde(() => window.__nde.tab.totalsOn)) === false, "Alt+= again hides the totals row");
  // A chosen column set to Average: the second press sets Sum there instead of hiding.
  await p.keyboard.press("Alt+Equal");
  await wait(900);
  await nde(() => {
    window.__nde.tab.columns.totals.set(3, "average");
    window.__nde.tab.bump();
  });
  await p.keyboard.press("Alt+Equal");
  await wait(400);
  ok((await nde(() => window.__nde.tab.totalsOn)) === true && (await nde(() => window.__nde.tab.columns.totals.get(3))) === "sum", "with Average on one, Alt+= sets Sum instead of hiding");
  // Another layout: the "=" place types "´"; and a German "=" (Shift+0).
  await p.keyboard.press("Alt+Equal");
  await wait(300);
  ok((await nde(() => window.__nde.tab.totalsOn)) === false, "(hidden again)");
  await nde(() => {
    const el = document.activeElement;
    el.dispatchEvent(new KeyboardEvent("keydown", { key: "´", code: "Equal", altKey: true, bubbles: true, cancelable: true }));
  });
  await wait(400);
  ok((await nde(() => window.__nde.tab.totalsOn)) === true, "the key in the = place works on another layout");
  await nde(() => {
    const el = document.activeElement;
    el.dispatchEvent(new KeyboardEvent("keydown", { key: "=", code: "Digit0", altKey: true, shiftKey: true, bubbles: true, cancelable: true }));
  });
  await wait(400);
  ok((await nde(() => window.__nde.tab.totalsOn)) === false, "and = typed as Shift+0 (German) works too");
  // From a column heading's menu: Sum this column in the totals row.
  const hp = await nde(() => window.__nde.grid.headerPoint(2));
  await p.mouse.click(hp.x - 30, hp.y - 8, { button: "right" });
  await wait(300);
  const colItems = await p.$$eval(".context [role=menuitem]", (e) => e.map((x) => x.textContent.trim()));
  ok(colItems.some((x) => /Sum this column in the totals row/.test(x)), "the heading menu has Sum this column in the totals row");
  await p.click(".context [role=menuitem]:has-text('Sum this column in the totals row')");
  await wait(400);
  ok((await nde(() => window.__nde.tab.totalsOn)) === true && (await nde(() => window.__nde.tab.columns.totals.get(2))) === "sum", "it turns the row on with Sum there");
  await sel(1, 2);
  await p.keyboard.press("Alt+Equal");
  await wait(300);
  ok((await nde(() => window.__nde.tab.totalsOn)) === false, "(Alt+= on that column hides it)");
  // In a text box Alt+= types nothing and changes nothing.
  await p.keyboard.press("Control+f");
  await wait(300);
  await p.focus("input[aria-label=Find]");
  await p.keyboard.press("Alt+Equal");
  await wait(300);
  ok((await nde(() => window.__nde.tab.totalsOn)) === false, "Alt+= in the Find box does not turn the row on");
  await p.keyboard.press("Escape");
  await wait(300);

  // ---- R3: the tab menu ----
  const tabEl = (i) => p.locator(`[data-tab-index="${i}"]`);
  const names = () => nde(() => window.__nde.tabs.map((t) => t.info.name));
  const n0 = (await names()).length;
  const at = (await names()).indexOf("precincts.csv");
  await sel(2, 1);
  await p.keyboard.type("Boise");
  await p.keyboard.press("Enter");
  await wait(400);
  await tabEl(at).click({ button: "right" });
  await wait(300);
  const items = await p.$$eval(".context [role=menuitem]", (e) => e.map((x) => x.textContent.trim()));
  ok(items.some((x) => /Make a copy of this tab/.test(x)) && items.some((x) => /Tab colour…/.test(x)), "the tab menu has Make a copy of this tab and Tab colour…");
  if (out) await p.screenshot({ path: path.join(out, "tab-menu.png") });
  await p.click(".context [role=menuitem]:has-text('Make a copy of this tab')");
  await wait(900);
  let list = await names();
  ok(list.length === n0 + 1 && list[at + 1] === "precincts.csv (copy)", `the copy is beside the original, named (copy) (${list.join(" | ")})`);
  ok((await nde(() => window.__nde.tab.info.name)) === "precincts.csv (copy)" && (await nde(() => window.__nde.tab.info.modified)) === true, "it is in front and unsaved");
  ok((await cell(2, 1)) === "Boise" && (await cell(4, 3)) === "8", "with the edit and every row");
  ok((await nde(() => window.__nde.tab.header)) === true, "and the title row setting");
  ok(/not saved yet/.test(await status()), `the status bar says it is not saved (${await status()})`);
  // Editing the copy leaves the original alone.
  await sel(1, 0);
  await p.keyboard.type("Zed");
  await p.keyboard.press("Enter");
  await wait(400);
  await nde((i) => window.__nde.activate(i), at);
  await wait(300);
  ok((await cell(1, 0)) === "Ann", "editing the copy leaves the original alone");
  // A second copy: (copy 2).
  await tabEl(at).click({ button: "right" });
  await wait(300);
  await p.click(".context [role=menuitem]:has-text('Make a copy of this tab')");
  await wait(900);
  list = await names();
  ok(list.includes("precincts.csv (copy 2)"), `a second copy is (copy 2) (${list.join(" | ")})`);

  // Tab colour.
  const i0 = (await names()).indexOf("precincts.csv");
  await nde((i) => window.__nde.activate(i), i0);
  await wait(200);
  await tabEl(i0).click({ button: "right" });
  await wait(300);
  await p.click(".context [role=menuitem]:has-text('Tab colour…')");
  await wait(400);
  const colours = await p.$$eval(".context [role=menuitem]", (e) => e.map((x) => x.textContent.trim()));
  ok(colours.length === 9 && colours.some((x) => /green/.test(x)), `the colours menu opens under the tab (${colours.length} items)`);
  await p.click(".context [role=menuitem]:has-text('Tab colour: green')");
  await wait(300);
  ok((await tabEl(i0).getAttribute("aria-label")) === "precincts.csv, tab colour green", "a screen reader hears the colour");
  ok(await tabEl(i0).evaluate((e) => e.classList.contains("coloured")), "the tab is coloured");
  // The copy keeps a colour it was made with.
  const blue = (await names()).indexOf("precincts.csv (copy)");
  await nde((i) => window.__nde.activate(i), blue);
  await nde(() => window.__nde.reg.get("file.tabColour.blue").run());
  await wait(300);
  if (out) await p.screenshot({ path: path.join(out, "tab-colours.png") });
  // Windows high contrast keeps the line (a border).
  await p.emulateMedia({ forcedColors: "active" });
  await wait(200);
  const border = await tabEl(i0).evaluate((e) => {
    const s = getComputedStyle(e);
    return { w: s.borderBottomWidth, st: s.borderBottomStyle };
  });
  ok(border.w === "3px" && border.st === "solid", `in high contrast the line stays (${JSON.stringify(border)})`);
  await p.emulateMedia({ forcedColors: "none" });
  // None takes it away.
  await nde((i) => window.__nde.activate(i), i0);
  await nde(() => window.__nde.reg.get("file.tabColour.none").run());
  await wait(200);
  ok((await tabEl(i0).getAttribute("aria-label")) === null, "Tab colour: none takes it away");

  ok(errors.length === 0, `no page errors (${errors.join("; ")})`);
  console.log(failed ? `${failed} FAILED` : "ALL PASSED");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
