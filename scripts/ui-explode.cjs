// UI test of Explode cells (C7) in the browser stand-in: the command greyed with its reason when no file is open, the
// right-click entry on a column heading, the detect line, the keys with counts and unticking, the preview for each
// kind (missing, null and "" told apart), the caps line, "did not parse" with Show those rows, Run as one undo step,
// unnest with the original kept, flags with the matching words. Made-up data in a new small table.
// Run: (cd ui && npx vite --port 5321) then NODE_PATH=<folder with playwright> node scripts/ui-explode.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5321";
const TAB = "\t";
const EOL = "\n";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1280, height: 860 } });
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
  const text = async () => ((await p.locator("body").textContent()) || "").replace(/\s+/g, " ");
  const dlg = "[role=dialog]:has(h2:has-text('Explode cells'))";
  const cmd = (id) => p.evaluate((id) => window.__nde.reg.run(id), id);
  const state = (id) =>
    p.evaluate((id) => {
      const c = window.__nde.reg.get(id);
      return { enabled: c.enabled(), why: c.why ? c.why() : "" };
    }, id);
  const grid = () =>
    p.evaluate(() => {
      const t = window.__nde.tab;
      return Array.from({ length: t.info.rows }, (_, r) => Array.from({ length: t.info.cols }, (_, c) => t.cache.get(r, c) ?? "?"));
    });
  const settle = async () => {
    await p.evaluate(() => {
      const t = window.__nde.tab;
      t.cache.request(0, Math.max(1, t.info.rows), 0, Math.max(1, t.info.cols));
    });
    await p.waitForTimeout(500);
  };
  const testid = (id) => `${dlg} [data-testid=${id}]`;
  const open = async () => {
    await cmd("data.explode");
    await p.waitForSelector(dlg);
    await p.waitForSelector(testid("explode-caps"), { timeout: 15000 });
  };
  const run = async () => {
    await p.click(testid("explode-run"));
    await p.waitForTimeout(800);
    await settle();
  };

  const st0 = await state("data.explode");
  ok(!st0.enabled && st0.why === "open a file first", `greyed with its reason when no file is open (${st0.why})`);

  await cmd("file.new");
  const rows = [
    ["id", "details", "tags"],
    ["1", '{"plan":"gold","seats":3,"note":""}', "vip|beta"],
    ["2", '{"plan":"free","seats":null}', "beta"],
    ["3", "{not json", "VIP|annual|beta"],
    ["4", '{"plan":"gold","region":"N"}', ""],
  ];
  await p.evaluate(
    async (text) => {
      const t = window.__nde.tab;
      t.update(await window.__nde.backend.dataOp(t.id, { op: "paste", spec: { row: 0, col: 0, op: "none", skip_blanks: false, transpose: false }, text }).then((r) => r.info), true);
      t.header = true;
      t.bump();
    },
    rows.map((r) => r.join(TAB)).join(EOL),
  );
  await p.waitForTimeout(400);
  ok((await state("data.explode")).enabled, "the command is on with a file open");

  // The column heading's right-click menu has it.
  await p.evaluate(() => window.__nde.grid?.select?.({ row: 0, col: 1 }, { row: 0, col: 1 }));
  const cv = await (await p.$("canvas")).boundingBox();
  await p.mouse.click(cv.x + 160, cv.y + 8, { button: "right" });
  await p.waitForTimeout(300);
  const items = await p.$$eval(".context [role=menuitem]", (e) => e.map((x) => x.textContent.trim()));
  ok(items.some((t) => /Explode cells/.test(t)), `the column heading's right-click menu offers Explode cells (${items.length} items)`);
  await p.keyboard.press("Escape");
  await p.waitForTimeout(200);

  // JSON objects: detect, keys with counts, preview, caps, did not parse.
  await p.evaluate(() => window.__nde.grid?.select?.({ row: 1, col: 1 }, { row: 1, col: 1 }));
  await p.evaluate(() => (window.__nde.tab.cell = { row: 1, col: 1 }));
  await open();
  const det = await p.textContent(testid("explode-detect"));
  ok(/Looks like: mixed: 3 of 4 look like JSON objects/.test(det), `the detect line (${det})`);
  const keys = await p.textContent(testid("explode-keys"));
  ok(/plan \(3\)/.test(keys) && /seats \(2\)/.test(keys) && /region \(1\)/.test(keys), `keys with counts (${keys.replace(/\s+/g, " ").trim()})`);
  let caps = await p.textContent(testid("explode-caps"));
  ok(/Adds 4 columns \(at most 1,000\)/.test(caps), `the caps line (${caps.trim()})`);
  const bad = await p.textContent(testid("explode-bad"));
  ok(/1 cell was not valid JSON: it stays exactly as it is/.test(bad), `the did-not-parse line (${bad.trim()})`);
  const prev = testid("explode-preview");
  const heads = await p.$$eval(`${prev} th`, (t) => t.map((x) => x.textContent.trim()));
  ok(JSON.stringify(heads) === JSON.stringify(["Before", "details.plan", "details.seats", "details.note", "details.region"]), `preview titles (${heads.join("|")})`);
  const r1 = await p.$$eval(`${prev} tbody tr:nth-child(1) td`, (t) => t.map((x) => `${x.className.split(" ")[0]}:${x.textContent.trim()}`));
  const r2 = await p.$$eval(`${prev} tbody tr:nth-child(2) td`, (t) => t.map((x) => `${x.className.split(" ")[0]}:${x.textContent.trim()}`));
  ok(r1[3] === 'empty:""' && r1[4] === "missing:(missing)", `"" and missing told apart (${r1.slice(1).join(" ")})`);
  ok(r2[2] === "null:(null)", `null told apart (${r2[2]})`);
  ok(/did not parse/.test(await p.textContent(`${prev} tbody tr:nth-child(3)`)), "the bad row says it stays as it is");
  // Unticking a key.
  await p.locator(`${testid("explode-keys")} label:has-text('region') input`).click();
  await p.waitForTimeout(400);
  caps = await p.textContent(testid("explode-caps"));
  ok(/Adds 3 columns/.test(caps), `unticking a key takes it off the caps line (${caps.trim()})`);
  // Dropping the original with a bad cell turns Run off with the reason.
  await p.click(testid("explode-keep"));
  await p.waitForTimeout(200);
  ok(/keep the original column so they stay as they are/.test(await p.textContent(testid("explode-why"))) && (await p.isDisabled(testid("explode-run"))), "dropping the original with bad cells turns Run off, with the reason");
  await p.click(testid("explode-keep"));
  await p.waitForTimeout(200);
  if (out) await p.screenshot({ path: path.join(out, "explode-dialog.png") });
  // Show those rows.
  await p.click(testid("explode-showbad"));
  await p.waitForTimeout(800);
  ok(/Showing the 1 row whose cell did not parse/.test(await text()), "Show those rows filters to them");
  await cmd("data.clearFilter");
  await p.waitForTimeout(500);

  // Run: one undo step.
  await open();
  await p.locator(`${testid("explode-keys")} label:has-text('region') input`).click();
  await p.waitForTimeout(300);
  const before = await grid();
  await run();
  let g = await grid();
  ok(JSON.stringify(g[0]) === JSON.stringify(["id", "details", "details.plan", "details.seats", "details.note", "tags"]), `columns added after the original (${g[0].join("|")})`);
  ok(g[1][2] === "gold" && g[1][3] === "3" && g[2][3] === "" && g[3][1] === "{not json" && g[3][2] === "", `values filled; the bad cell left (${g.slice(1, 4).map((r) => r.slice(1, 4).join("/")).join(" ; ")})`);
  ok(/Exploded “details”: added 3 columns\. 1 cell did not parse and stayed as it was\. 1 key not ticked was left out\./.test(await text()), "the status says what happened");
  await cmd("edit.undo");
  await p.waitForTimeout(600);
  await settle();
  ok(JSON.stringify(await grid()) === JSON.stringify(before), "one Ctrl+Z takes it all back");

  // A list: unnest to rows with the original kept, then flags.
  await open();
  await p.selectOption(testid("explode-col"), { label: "tags" });
  await p.waitForSelector(`${testid("explode-detect")}:has-text('lists')`, { timeout: 5000 }).catch(() => {});
  ok(/lists separated by \|/.test(await p.textContent(testid("explode-detect"))), "a pipe list is detected");
  await p.click(`${dlg} input[type=radio][value=rows]`);
  await p.waitForTimeout(400);
  ok(/Shown rows only: hidden rows are left alone/.test(await p.textContent(dlg)), "unnest says it acts on shown rows only");
  await p.click(testid("explode-origcol"));
  await p.waitForSelector(`${testid("explode-caps")}:has-text('and 1 column')`, { timeout: 5000 });
  caps = await p.textContent(testid("explode-caps"));
  ok(/Adds 3 rows \(at most 10,000,000\) and 1 column/.test(caps), `caps for unnest (${caps.trim()})`);
  const lines = await p.$$eval(`${prev} tbody tr`, (t) => t.length);
  ok(lines >= 6, `the preview shows one line per new row (${lines})`);
  const before2 = await grid();
  await run();
  g = await grid();
  ok(JSON.stringify(g[0]) === JSON.stringify(["id", "details", "tags", "tags (original)"]) && g.length === 8, `rows and the original column added (${g.length} rows; ${g[0].join("|")})`);
  ok(g[1][2] === "vip" && g[2][2] === "beta" && g[2][3] === "vip|beta" && g[2][0] === "1", `items one per row, others repeated (${g.slice(1, 3).map((r) => r.join("/")).join(" ; ")})`);
  await cmd("edit.undo");
  await p.waitForTimeout(600);
  await settle();
  ok(JSON.stringify(await grid()) === JSON.stringify(before2), "one Ctrl+Z takes the rows and the column back");

  await open();
  await p.selectOption(testid("explode-col"), { label: "tags" });
  await p.waitForSelector(`${testid("explode-detect")}:has-text('lists')`, { timeout: 5000 }).catch(() => {});
  await p.click(`${dlg} input[type=radio][value=flags]`);
  await p.waitForSelector(testid("explode-matching"), { timeout: 5000 });
  const words = await p.textContent(testid("explode-matching"));
  ok(/matched ignoring capitals \(VIP = vip\), spaces around them left out/.test(words), `flags say how items are matched (${words.trim()})`);
  await p.waitForSelector(`${testid("explode-keys")}:has-text('vip')`, { timeout: 5000 }).catch(() => {});
  const tags = await p.textContent(testid("explode-keys"));
  ok(/vip \(2\)/.test(tags) && /beta \(3\)/.test(tags), `tags with counts, VIP and vip together (${tags.replace(/\s+/g, " ").trim()})`);
  await run();
  g = await grid();
  ok(JSON.stringify(g[0]) === JSON.stringify(["id", "details", "tags", "tags: vip", "tags: beta", "tags: annual"]) && g[3][3] === "1" && g[2][3] === "", `flag columns marked where present (${g.slice(0, 4).map((r) => r.slice(3).join("/")).join(" ; ")})`);
  await cmd("edit.undo");
  await p.waitForTimeout(500);

  // Esc closes without a change.
  await open();
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);
  ok(!(await p.isVisible(dlg)), "Esc closes the dialog");
  ok(errors.length === 0, `no page errors (${errors.join("; ")})`);
  console.log(failed ? `${failed} FAILED` : "ALL PASSED");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
