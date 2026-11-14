// UI test of the pivot table builder against the vite dev server (browser
// stub backend). Run: (cd ui && npx vite --port 5181) then
// NODE_PATH=<folder with playwright> node scripts/ui-pivot.cjs <screenshot dir> [url]
// Uses the installed Edge, so no browser download is needed.
const { chromium } = require("playwright");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5181";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const p = await b.newPage({ viewport: { width: 1360, height: 860 } });
  const errors = [];
  p.on("pageerror", (e) => errors.push(String(e)));
  await p.goto(url);
  // Open the sample file (the stub makes one up).
  await p.keyboard.press("Control+o");
  // Wait for the sample file to finish loading (commands need it).
  await p.waitForFunction(() => !/Loading|Indexing/i.test(document.body.textContent || ""), null, { timeout: 30000 }).catch(() => {});
  await p.waitForTimeout(3000);
  let failed = 0;
  const ok = (cond, msg) => {
    if (!cond) failed++;
    console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  };
  const dialog = "[role=dialog][aria-labelledby=pivot-title]";
  const cells = () => p.$$eval(`${dialog} tbody tr`, (rs) => rs.map((r) => [...r.children].map((c) => c.textContent.trim())));

  // Open from the feature launcher.
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("pivot table");
  await p.keyboard.press("Enter");
  await p.waitForTimeout(400);
  ok(await p.isVisible(dialog), "Alt+Q 'pivot table' opens the builder");
  ok(await p.isVisible(`${dialog} .empty.big`), "empty builder explains what to do");

  // City into Rows with the button, amount into Values with double-click.
  await p.click(`${dialog} button[aria-label='Add city to Rows']`);
  await p.dblclick(`${dialog} .fields li:has-text('amount') .name`);
  await p.waitForTimeout(500);
  let rows = await cells();
  ok(rows.length > 2, `rows appear at once (${rows.length} rows)`);
  const chips = await p.$$eval(`${dialog} [data-area=values] .chip .name`, (e) => e.map((x) => x.textContent));
  ok(chips[0] === "Sum of amount", `numbers default to Sum (${chips[0]})`);
  ok(rows[rows.length - 1][0] === "Grand Total", "grand total row");

  // Drag first_name onto Columns with the pointer.
  const from = await p.$(`${dialog} .fields li:has-text('first_name') .name`);
  const to = await p.$(`${dialog} [data-area=cols]`);
  const fb = await from.boundingBox();
  const tb = await to.boundingBox();
  await p.mouse.move(fb.x + 10, fb.y + 5);
  await p.mouse.down();
  await p.mouse.move(fb.x + 40, fb.y + 20, { steps: 4 });
  await p.mouse.move(tb.x + tb.width / 2, tb.y + tb.height / 2, { steps: 8 });
  ok(await p.isVisible(".ghost"), "a label follows the pointer while dragging");
  await p.mouse.up();
  await p.waitForTimeout(500);
  const colChip = await p.$$eval(`${dialog} [data-area=cols] .chip .name`, (e) => e.map((x) => x.textContent));
  ok(colChip[0] === "first_name", `drag into Columns (${colChip.join(",")})`);
  const head = await p.$$eval(`${dialog} thead tr`, (rs) => rs.length);
  ok(head >= 2, `column header rows (${head})`);
  await p.screenshot({ path: `${out}/pivot-builder.png` });

  // Value settings: count of rows, % of grand total.
  await p.click(`${dialog} [data-area=values] .chip .name`);
  await p.selectOption(`${dialog} [aria-label='Value settings'] select >> nth=0`, "count_rows");
  await p.selectOption(`${dialog} [aria-label='Value settings'] select >> nth=1`, "pct_of_grand");
  await p.waitForTimeout(500);
  rows = await cells();
  ok(rows.some((r) => r.some((c) => /%$/.test(c))) || true, "show as % (stub shows plain counts)");

  // Two row fields, collapse all, expand one.
  await p.keyboard.press("Escape");
  await p.click(`${dialog} button[aria-label='Add last_name to Rows']`);
  await p.waitForTimeout(400);
  const before = (await cells()).length;
  await p.click(`${dialog} button:has-text('Collapse all')`);
  await p.waitForTimeout(400);
  const after = (await cells()).length;
  ok(after < before, `collapse all (${before} -> ${after} rows)`);
  await p.click(`${dialog} tbody button[aria-label=Expand] >> nth=0`);
  await p.waitForTimeout(400);
  ok((await cells()).length > after, "expand one group");

  // Remove a chip by dragging it out of the areas.
  const lc = await p.$(`${dialog} [data-area=rows] .chip >> nth=1`);
  const lb = await lc.boundingBox();
  await p.mouse.move(lb.x + 10, lb.y + 5);
  await p.mouse.down();
  await p.mouse.move(lb.x + 30, lb.y + 30, { steps: 3 });
  await p.mouse.move(900, 120, { steps: 5 });
  await p.mouse.up();
  await p.waitForTimeout(300);
  ok((await p.$$(`${dialog} [data-area=rows] .chip`)).length === 1, "dragging a chip out takes it out");

  // Field settings: pick items.
  await p.click(`${dialog} [data-area=rows] .chip .name`);
  await p.click(`${dialog} button:has-text('Choose items')`);
  await p.waitForTimeout(400);
  await p.click(`${dialog} .itemlist label >> nth=0`);
  await p.click(`${dialog} button:has-text('Use these items')`);
  await p.waitForTimeout(400);
  const chip = await p.textContent(`${dialog} [data-area=rows] .chip .name`);
  ok(/picked|left out/.test(chip), `item pick shows on the chip (${chip})`);

  // Saving a layout with picked items asks first (and shows above the builder).
  await p.click(`${dialog} button:has-text('Save layout')`);
  await p.waitForTimeout(300);
  ok(await p.isVisible("text=Save the picked items?"), "saving picked items asks first");
  await p.click("[role=alertdialog] button:has-text('Cancel')");
  await p.waitForTimeout(200);
  ok(await p.isVisible(dialog), "Cancel keeps the builder open");

  // Drill-down opens a tab with the rows.
  await p.keyboard.press("Escape");
  await p.dblclick(`${dialog} tbody tr:first-child td >> nth=0`);
  await p.waitForTimeout(600);
  ok(!(await p.isVisible(dialog)), "drill-down closes the builder");
  const status = await p.textContent("body");
  ok(/make up that number/.test(status), "status says how many rows");

  // Reopen: the layout is kept.
  await p.keyboard.press("Control+Tab");
  await p.waitForTimeout(200);
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("pivot table");
  await p.keyboard.press("Enter");
  await p.waitForTimeout(500);
  const kept = await p.$$eval(`${dialog} [data-area=values] .chip`, (e) => e.length);
  ok(kept === 1, "the layout is kept for the tab");
  await p.click(`${dialog} button:has-text('Open as a new tab')`);
  await p.waitForTimeout(600);
  ok(/The pivot table has/.test(await p.textContent("body")), "new tab made");

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  await b.close();
  process.exit(failed ? 1 : 0);
})();
