// UI test of renaming columns against the vite dev server (browser stub):
// double-click a heading, right-click it, Format > Rename column, undo, a
// file without a header row, and that dragging a selected column still works.
// Run: (cd ui && npx vite --port 5193) then
// NODE_PATH=<folder with playwright> node scripts/ui-rename.cjs <screenshot dir> [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5193";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const p = await b.newPage({ viewport: { width: 1360, height: 860 } });
  const errors = [];
  p.on("pageerror", (e) => errors.push(String(e)));
  await p.goto(url);
  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => !/Loading|Indexing|Reading/i.test(document.querySelector("footer.status")?.textContent || ""), null, { timeout: 30000 }).catch(() => {});
  await p.waitForTimeout(1500);
  let failed = 0;
  const ok = (cond, msg) => {
    if (!cond) failed++;
    console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  };
  const canvas = await (await p.$("canvas")).boundingBox();
  const HEAD = 25;
  const ROW = 25;
  const W = 130;
  const at = async () => p.inputValue("input.goto");
  const status = async () => (await p.textContent("footer.status .msg").catch(() => "")) || "";
  let left = 0;
  for (let x = 30; x < 120; x += 2) {
    await p.mouse.click(canvas.x + x, canvas.y + HEAD + ROW + 12);
    if (/C1$/.test(await at())) {
      left = x;
      break;
    }
  }
  const hx = (c) => canvas.x + left + c * W + 40; // clear of the edge and the filter button
  const hy = canvas.y + HEAD / 2;
  const dialog = "[role=dialog]:has(h2:has-text('Rename column'))";
  const nameBox = `${dialog} input.name`;

  // Double-click a heading.
  await p.mouse.dblclick(hx(1), hy);
  await p.waitForTimeout(400);
  ok(await p.isVisible(dialog), "double-clicking a heading opens Rename column");
  ok((await p.textContent(`${dialog} h2`)).includes("column 2"), "for that column");
  ok((await p.inputValue(nameBox)) === "first_name", `with its name filled in (${await p.inputValue(nameBox)})`);
  ok(await p.$eval(nameBox, (e) => e === document.activeElement && e.selectionStart === 0 && e.selectionEnd === e.value.length), "selected, ready to type over");
  await p.fill(nameBox, "last_name");
  await p.waitForTimeout(100);
  ok(await p.isVisible(`${dialog} .warn`), "a name another column has is pointed out");
  await p.fill(nameBox, "given name");
  await p.screenshot({ path: path.join(out, "rename-column.png"), clip: { x: 0, y: 0, width: 1360, height: 520 } });
  await p.keyboard.press("Enter");
  await p.waitForTimeout(600);
  ok(!(await p.isVisible(dialog)), "Enter renames and closes");
  ok(/Column 2 is now "given name"/.test(await status()), `it says so (${await status()})`);
  await p.mouse.dblclick(hx(1), hy);
  await p.waitForTimeout(400);
  ok((await p.inputValue(nameBox)) === "given name", "the heading has the new name");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(200);

  // Undo puts the old name back.
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(600);
  await p.mouse.dblclick(hx(1), hy);
  await p.waitForTimeout(400);
  ok((await p.inputValue(nameBox)) === "first_name", "Ctrl+Z puts the old name back");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(200);

  // Right-click a heading outside the selection: it is picked, and the menu has Rename.
  await p.mouse.click(canvas.x + left + 10, canvas.y + HEAD + ROW + 12); // cell in column 1
  await p.mouse.click(hx(3), hy, { button: "right" });
  await p.waitForTimeout(300);
  ok(/C4$/.test(await at()), `right-click picks that column first (${await at()})`);
  const first = await p.$$eval(".context [role=menuitem]", (e) => e.map((x) => x.textContent.trim())[0] || "");
  ok(/Rename column/.test(first), `the heading menu starts with Rename column (${first})`);
  await p.click(".context [role=menuitem]:has-text('Rename column')");
  await p.waitForTimeout(400);
  ok((await p.textContent(`${dialog} h2`)).includes("column 4"), "the menu renames the clicked column");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(200);

  // Dragging a selected column by its heading still moves it.
  await p.mouse.click(hx(1), hy);
  await p.waitForTimeout(300);
  await p.mouse.move(hx(1), hy);
  await p.mouse.down();
  await p.mouse.move(canvas.x + left + 3 * W + 100, hy, { steps: 12 });
  await p.mouse.up();
  await p.waitForTimeout(800);
  ok(/Moved/i.test(await status()) && !(await p.isVisible(dialog)), `dragging a selected heading still moves the column (${await status()})`);
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(600);

  // No header row: renaming adds one.
  await p.click("button:has-text('Format')");
  await p.click("[role=menuitem]:has-text('First row is header')");
  await p.waitForTimeout(400);
  await p.mouse.click(canvas.x + left + 10, canvas.y + HEAD + ROW + 12);
  await p.click("button:has-text('Format')");
  await p.click("[role=menuitem]:has-text('Rename column')");
  await p.waitForTimeout(400);
  ok(await p.isVisible(dialog), "Format > Rename column opens it");
  ok(/no header row/.test(await p.textContent(dialog)), "without a header row it says one is added");
  await p.fill(nameBox, "record");
  await p.click(`${dialog} button.primary`);
  await p.waitForTimeout(800);
  await p.mouse.dblclick(hx(0), hy);
  await p.waitForTimeout(400);
  ok((await p.inputValue(nameBox)) === "record", "a header row was added with the name");
  ok(!/no header row/.test(await p.textContent(dialog)), "and is now on");
  await p.keyboard.press("Escape");

  // Alt+Q finds it by "rename header".
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("rename header");
  await p.waitForTimeout(200);
  const top = await p.$$eval("#command-results [role=option]", (e) => e.slice(0, 3).map((x) => x.textContent.trim()));
  ok(top.some((t) => /Rename column/.test(t)), `Alt+Q "rename header" finds it (${top[0]})`);
  await p.keyboard.press("Escape");

  ok(errors.length === 0, `no page errors ${errors.join("; ")}`);
  await b.close();
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  process.exit(failed ? 1 : 0);
})();
