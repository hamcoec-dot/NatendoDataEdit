// UI test: what follows a column rename (browser stub backend).
// - A data check's row rule naming the column ([first_name]) is rewritten
//   to the new name, and undo/redo follow the rename.
// - A saved pivot layout whose column was renamed asks to use the column
//   in the same place (the stub's layout "by city" uses column 4).
// Run: (cd ui && npx vite --port 5193) then
// NODE_PATH=<folder with playwright> node scripts/ui-renref.cjs <screenshot dir> [url]
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
  const hx = (c) => canvas.x + left + c * W + 40;
  const hy = canvas.y + HEAD / 2;
  const command = async (q) => {
    await p.keyboard.press("Alt+q");
    await p.keyboard.type(q);
    await p.waitForTimeout(200);
    await p.keyboard.press("Enter");
    await p.waitForTimeout(400);
  };
  const checks = "[role=dialog]:has(h2:has-text('Data checks'))";
  const rule = async () => {
    await command("data checks");
    const v = await p.inputValue(`${checks} input[aria-label=Rule]`).catch(() => null);
    await p.keyboard.press("Escape");
    await p.waitForTimeout(200);
    return v;
  };
  const rename = async (c, name) => {
    await p.mouse.dblclick(hx(c), hy);
    await p.waitForTimeout(400);
    await p.fill("[role=dialog]:has(h2:has-text('Rename column')) input.name", name);
    await p.keyboard.press("Enter");
    await p.waitForTimeout(700);
  };

  // A row rule that names first_name.
  await command("data checks");
  ok(await p.isVisible(checks), "Data checks opens");
  if (!(await p.$(`${checks} select[aria-label=Check]`))) await p.click(`${checks} button:has-text('Add a check')`);
  await p.selectOption(`${checks} select[aria-label=Check] >> nth=0`, "expression");
  await p.fill(`${checks} input[aria-label=Rule]`, '[first_name] <> [id]');
  await p.click(`${checks} button:has-text('Keep and close')`);
  await p.waitForTimeout(300);

  await rename(1, "given name");
  ok(/1 data check rule now uses the new name/.test(await status()), `the rename says the rule follows (${await status()})`);
  ok((await rule()) === '[given name] <> [id]', "the row rule now names [given name]");
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(600);
  ok((await rule()) === '[first_name] <> [id]', "Ctrl+Z puts the rule back with the old name");
  await p.keyboard.press("Control+y");
  await p.waitForTimeout(600);
  ok((await rule()) === '[given name] <> [id]', "Ctrl+Y renames it in the rule again");

  // A rule changed by hand after the rename is not touched by undo.
  await command("data checks");
  await p.fill(`${checks} input[aria-label=Rule]`, "[id] > 5");
  await p.click(`${checks} button:has-text('Keep and close')`);
  await p.waitForTimeout(300);
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(600);
  ok((await rule()) === "[id] > 5", "a rule edited by hand since is left alone by undo");
  await p.keyboard.press("Control+y");
  await p.waitForTimeout(600);

  // Renaming to a name another column has: the rule is pinned by number.
  await command("data checks");
  await p.fill(`${checks} input[aria-label=Rule]`, '[last_name] <> [id]');
  await p.click(`${checks} button:has-text('Keep and close')`);
  await p.waitForTimeout(300);
  await rename(1, "last_name");
  ok(/by number/.test(await status()), `renaming to another column's name says rules are pinned (${await status()})`);
  ok((await rule()) === "[#3] <> [id]", `the rule now names column 3 by number (${await rule()})`);
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(600);
  ok((await rule()) === "[last_name] <> [id]", "undo puts the rule back");

  // A saved view's rules follow the columns it is applied to by place.
  await command("data checks");
  await p.fill(`${checks} input[aria-label=Rule]`, '[given name] <> [id]');
  await p.click(`${checks} button:has-text('Keep and close')`);
  await p.waitForTimeout(300);
  await command("save view");
  await p.fill("[role=dialog]:has(h2:has-text('Save view')) input", "renref view");
  await p.keyboard.press("Enter");
  await p.waitForTimeout(400);
  await rename(1, "forename");
  await command("View: renref view");
  await p.waitForTimeout(400);
  const viewAsk = "[role=alertdialog]:has-text('Use columns in the same place')";
  ok(await p.isVisible(viewAsk), "applying the view asks about the renamed column");
  ok(/Data check rules in the view that name them follow too/.test((await p.textContent(viewAsk)) || ""), "and says its check rules follow");
  await p.click(`${viewAsk} button:has-text('Use that column')`);
  await p.waitForTimeout(800);
  ok((await rule()) === "[forename] <> [id]", `the view's rule names the column it now uses (${await rule()})`);

  // Pivot layout saved with "city" in column 4; rename it, then open the layout.
  await rename(3, "town");
  await command("pivot table");
  const pivot = "[role=dialog][aria-labelledby=pivot-title]";
  ok(await p.isVisible(pivot), "the pivot builder opens");
  await p.click(`${pivot} button:has-text('Open layout')`);
  await p.waitForTimeout(500);
  const ask = `${pivot} .by-place`;
  ok(await p.isVisible(ask), "a layout naming a renamed column asks first");
  const text = (await p.textContent(ask)) || "";
  ok(/“city” → column 4, “town”/.test(text), `it says which column stands in (${text.replace(/\s+/g, " ").slice(0, 160)})`);
  await p.screenshot({ path: path.join(out, "pivot-by-place.png"), clip: { x: 0, y: 0, width: 1360, height: 640 } });
  await p.click(`${ask} button:has-text("Don't open the layout")`);
  await p.waitForTimeout(200);
  ok(!(await p.isVisible(ask)) && (await p.inputValue(`${pivot} input[aria-label='Layout name']`)) === "", "Don't open leaves the builder as it was");
  await p.click(`${pivot} button:has-text('Open layout')`);
  await p.waitForTimeout(500);
  await p.click(`${ask} button:has-text('Use that column')`);
  await p.waitForTimeout(800);
  ok((await p.inputValue(`${pivot} input[aria-label='Layout name']`)) === "by city", "Use that column opens the layout");
  ok(/town/.test((await p.textContent(pivot)) || ""), "with the renamed column in Rows");

  ok(errors.length === 0, `no page errors ${errors.join("; ")}`);
  await b.close();
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  process.exit(failed ? 1 : 0);
})();
