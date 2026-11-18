// UI test of v0.4.5 (power-user batch 5): Excel export made for Excel
// users (parts over the row limit, all tabs as one workbook), audit
// samples with their record, split view and pivot Refresh.
// Run: (cd ui && npx vite --port 5205) then
// NODE_PATH=<folder with playwright> node scripts/ui-batch5.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5205";
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
  const sel = (r, c) =>
    p.evaluate(([a, b2]) => {
      window.__nde.grid.select({ row: a, col: b2 });
      window.__nde.grid.focus();
    }, [r, c]).then(() => wait(200));
  const command = async (q) => {
    await p.keyboard.press("Alt+q");
    await p.keyboard.type(q);
    await wait(300);
    await p.keyboard.press("Enter");
    await wait(600);
  };
  const dialog = (title) => p.locator(`[role=dialog]:has(h2:has-text('${title}'))`);
  const status = () => p.locator("footer").textContent();

  await p.goto(url);
  await p.keyboard.press("Control+n");
  await wait(700);
  const rows = ["voter_id\tprecinct\tamount\tzip"];
  const precincts = ["Red Bank 1", "Soddy 4", "Lookout 2"];
  for (let i = 0; i < 40; i++) rows.push(`${3001 + i}\t${precincts[i % 3]}\t$${(i * 12.5).toFixed(2)}\t0${7400 + i}`);
  await sel(0, 0);
  await clip(rows.join("\r\n") + "\r\n");
  await p.keyboard.press("Control+v");
  await wait(800);
  await p.evaluate(() => {
    window.__nde.tab.header = true;
    window.__nde.tab.bump();
  });

  // ---- Excel export ----
  await command("Export");
  const ex = dialog("Export");
  ok(await ex.isVisible(), "Export opens");
  ok(await ex.locator("label:has-text('Made for Excel users') input").isChecked(), "made for Excel users is on by default");
  ok(/IDs, ZIP codes and phone numbers kept as text/.test(await ex.textContent()), "it says what that does");
  await ex.locator("input[placeholder^='as many as fit']").fill("10");
  await wait(300);
  const parts = ex.locator("fieldset[aria-label='Parts']");
  ok(/40 rows: 4 parts, each with the title row/.test(await parts.textContent()), `parts are counted (${(await parts.textContent()).replace(/\s+/g, " ").slice(0, 80)})`);
  await parts.locator("label:has-text('Separate files') input").check();
  if (out) await ex.screenshot({ path: path.join(out, "excel-export.png") });
  await ex.locator("button:has-text('Export to file')").click();
  await wait(700);
  ok(/to 4 files: .*\(1 of 4\)\.xlsx to .*\(4 of 4\)\.xlsx, each with the title row/.test(await status()), `parts as separate files (${(await status()).slice(0, 120)})`);
  // The choice is remembered.
  await command("Export");
  await ex.locator("input[placeholder^='as many as fit']").fill("10");
  await wait(300);
  ok(await parts.locator("label:has-text('Separate files') input").isChecked(), "the separate-files choice is remembered");
  await ex.locator("button:has-text('Cancel')").click();
  await wait(300);

  // ---- Audit sample ----
  await command("Random sample");
  const sd = dialog("Random sample");
  ok(await sd.isVisible(), "Tools › Random sample opens");
  await sd.locator("select").first().selectOption("rows");
  await sd.locator("input[type=number]").first().fill("2");
  await sd.locator("select[aria-label='Draw from']").selectOption({ label: "each group of precinct" });
  await sd.locator("label:has-text('Seed') input").fill("2024");
  ok(/: 2 from each/.test(await sd.textContent()), "per group: 2 from each");
  if (out) await sd.screenshot({ path: path.join(out, "audit-sample.png") });
  const before = await p.evaluate(() => window.__nde.tabs.length);
  await sd.locator("button:has-text('Run')").click();
  await wait(900);
  const names = await p.evaluate(() => window.__nde.tabs.map((t) => t.info.name));
  ok(names.length === before + 2, `the sample and its record open (${names.slice(-2).join(" | ")})`);
  ok(/seed 2024/.test(names[names.length - 2] ?? ""), "the sample tab names its seed");
  ok(/^Sample record/.test(names[names.length - 1] ?? ""), "the record tab");
  ok((await p.evaluate(() => window.__nde.tab.info.rows)) === 7, "2 rows from each of 3 precincts, with titles");
  ok(/with seed 2024/.test(await status()), "the status names the seed");
  if (out) {
    await p.evaluate(() => window.__nde.activate(window.__nde.tabs.length - 1));
    await wait(500);
    await p.screenshot({ path: path.join(out, "sample-record.png") });
  }

  // ---- Several tabs as one workbook: listed, the record unticked ----
  await command("Export");
  await ex.locator("label:has-text('Several open tabs') input").check();
  await wait(300);
  const tl = ex.locator("fieldset[aria-label='Tabs']");
  const recLabel = tl.locator("label:has-text('Sample record')");
  ok(/an audit record/.test(await recLabel.textContent()) && !(await recLabel.locator("input").isChecked()), "the record tab is listed, marked and unticked");
  ok(/2 of 3 tabs/.test(await tl.textContent()), `the other tabs are ticked (${(await tl.locator("legend").textContent())})`);
  await ex.locator("button:has-text('Cancel')").click();
  await wait(300);

  // ---- Split view ----
  await p.evaluate(() => {
    const i = window.__nde.tabs.findIndex((t) => t.info.rows === 41);
    if (i >= 0) window.__nde.activate?.(i);
  });
  await wait(300);
  await sel(1, 0);
  await p.keyboard.press("Alt+Shift+W");
  await wait(600);
  const sv = p.locator(".splitview");
  ok(await sv.isVisible(), "Alt+Shift+W splits the view");
  ok((await sv.locator("canvas").count()) >= 2, "two panes");
  const pane2 = sv.locator(".pane").nth(1);
  const box = await pane2.boundingBox();
  await p.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await p.mouse.wheel(0, 600);
  await wait(400);
  const sc = await p.evaluate(() => [window.__nde.tab.scrollY, window.__nde.tab.splitScroll.scrollY]);
  ok(sc[0] === 0 && sc[1] > 0, `each pane scrolls on its own (${sc.join(", ")})`);
  if (out) await p.screenshot({ path: path.join(out, "split-view.png") });
  // Drag the bar to the edge: the split closes.
  const bar = sv.locator(".divider");
  const bb = await bar.boundingBox();
  await p.mouse.move(bb.x + bb.width / 2, bb.y + bb.height / 2);
  await p.mouse.down();
  const svb = await sv.boundingBox();
  await p.mouse.move(bb.x + bb.width / 2, svb.y + svb.height - 2, { steps: 5 });
  await p.mouse.up();
  await wait(400);
  ok(!(await sv.count()), "dragging the bar to the edge closes the split");
  await command("Split side by side");
  ok(await p.locator(".splitview.cols").isVisible(), "split side by side");
  await command("Split side by side");
  ok(!(await p.locator(".splitview").count()), "again closes it");

  // ---- Pivot refresh ----
  await p.locator(".tab.active").click({ button: "right" });
  await wait(300);
  ok(!(await p.locator("[role=menu] >> text=Refresh pivot table").count()), "no Refresh on a tab that is not a pivot");
  await p.keyboard.press("Escape");
  await wait(200);
  await command("Pivot table");
  const pd = "[role=dialog][aria-labelledby=pivot-title]";
  await p.click(`${pd} button[aria-label='Add precinct to Rows']`);
  await wait(300);
  await p.click(`${pd} button:has-text('Open as a new tab')`);
  await wait(900);
  const pivotId = await p.evaluate(() => window.__nde.tab.id);
  // Edit the source, then refresh the pivot.
  await p.evaluate(() => {
    const i = window.__nde.tabs.findIndex((t) => t.info.rows === 41);
    window.__nde.activate?.(i);
  });
  await wait(300);
  await sel(1, 1);
  await p.keyboard.type("New precinct");
  await p.keyboard.press("Enter");
  await wait(400);
  await p.evaluate((id) => {
    const i = window.__nde.tabs.findIndex((t) => t.id === id);
    window.__nde.activate?.(i);
  }, pivotId);
  await wait(300);
  const rowsBefore = await p.evaluate(() => window.__nde.tab.info.rows);
  const tabEl = p.locator(".tab.active");
  await tabEl.click({ button: "right" });
  await wait(300);
  const menu = p.locator("[role=menu]");
  ok(await menu.locator("text=Refresh pivot table").isVisible(), "right-click on a pivot's tab offers Refresh");
  if (out) await p.screenshot({ path: path.join(out, "pivot-refresh.png") });
  await p.keyboard.press("Escape");
  await wait(200);
  await p.keyboard.press("Alt+F5");
  await wait(900);
  const after = await p.evaluate(() => [window.__nde.tab.info.rows, window.__nde.tab.info.name]);
  ok(after[0] === rowsBefore + 1, `Alt+F5 builds it again with the new value (${rowsBefore} → ${after[0]})`);
  ok(/^Refreshed from/.test(await status()), "it says so");
  // A column inserted at the left of the source: Refresh still groups by
  // precinct, found by its title.
  const groups = after[0];
  await p.evaluate(() => {
    const i = window.__nde.tabs.findIndex((t) => t.info.rows === 41);
    window.__nde.activate?.(i);
  });
  await wait(300);
  await sel(1, 0);
  await command("Columns left");
  await wait(400);
  ok((await p.evaluate(() => window.__nde.tab.info.cols)) === 5, "a column was inserted in the source");
  await p.evaluate((id) => {
    const i = window.__nde.tabs.findIndex((t) => t.info.name.startsWith("Pivot"));
    window.__nde.activate?.(i);
  }, pivotId);
  await wait(300);
  await p.keyboard.press("Alt+F5");
  await wait(900);
  const moved = await p.evaluate(() => [window.__nde.tab.info.rows, window.__nde.tab.info.name]);
  ok(moved[0] === groups && /precinct/.test(moved[1]), `Refresh follows precinct to its new place (${moved.join(", ")})`);

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
