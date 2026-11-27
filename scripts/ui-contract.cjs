// UI test of file contracts (C9) against the vite dev server (browser stand-in backend): make a contract from
// a made-up monthly file (a column that looks like personal data asks first), open next month's drifted file
// and the banner says what drifted; Review lists each kind; a count shows its rows as a filter chip; Fix what
// I can renames and orders the columns and one Ctrl+Z puts them back; Accept these new values and Update the
// baseline save the contract; Cancel stops a check and Check again runs it; Not this contract; the entry is
// greyed with its reason on a private tab; the Contracts list exports and deletes; Alt+Q finds the feature.
// Run: (cd ui && npx vite --port 5321) then
// NODE_PATH=<folder with playwright> node scripts/ui-contract.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5321";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1360, height: 860 } });
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
  const status = async () => ((await p.textContent("footer.status")) || "").replace(/\s+/g, " ");
  const calls = (pre) => p.evaluate((pre) => window.__nde.backend.calls.filter((c) => c.startsWith(pre)), pre);
  const dialog = () => p.$('[role="dialog"], [role="alertdialog"]');
  const dialogText = async () => ((await (await dialog())?.textContent()) || "").replace(/\s+/g, " ");
  const banner = async () => ((await (await p.$('[data-testid="contract-banner"]'))?.textContent()) || "").replace(/\s+/g, " ").trim();
  const panel = async () => ((await (await p.$('[data-testid="contract-panel"]'))?.textContent()) || "").replace(/\s+/g, " ");
  const run = (id) => p.evaluate((id) => window.__nde.reg.run(id), id);
  const command = (id) => p.evaluate((id) => ({ enabled: window.__nde.reg.get(id).enabled(), why: window.__nde.reg.get(id).why?.() ?? "" }), id);
  const open = async (name, rows) => {
    await p.evaluate(([name, rows]) => window.__nde.addTab(window.__nde.backend.mockFileTab(name, rows)), [name, rows]);
    await p.waitForTimeout(900);
    // The title-row question, if asked: yes, column names.
    const yes = await p.$('button:has-text("Yes, column names")');
    if (yes) {
      await yes.click();
      await p.waitForTimeout(300);
    }
  };
  const titles = () => p.evaluate(async () => {
    const t = window.__nde.tab;
    const v = await window.__nde.backend.viewport(t.id, 0, 1, 0, t.info.cols, 100);
    return v.cells.slice(0, v.cols);
  });

  // ---- January: make the contract ----
  const jan = [["id", "name", "party", "phone"], ["1", "Ann Moss", "DEM", "555-0100"], ["2", "Bo Lane", "REP", "555-0101"], ["3", "Cy Hart", "DEM", ""], ["4", "Di Fox", "REP", "555-0103"], ["5", "Ed Ray", "DEM", "555-0104"], ["6", "Flo Oak", "DEM", "555-0105"]];
  await open("voters_jan.csv", jan);
  ok((await calls("contractMatch")).length >= 1 && !(await banner()), "a file opens with no contracts saved: no bar");
  await run("data.contractMake");
  await p.waitForTimeout(500);
  let dt = await dialogText();
  ok(/Make a contract from this file/.test(dt) && /never other cells/.test(dt), "Make a contract opens and says what it keeps");
  const ticked = await p.evaluate(() => [...document.querySelectorAll('[role="dialog"] input[type="checkbox"][aria-label^="Keep the values"]')].filter((x) => x.checked).length);
  ok(ticked === 0, "no column's values are ticked to begin with");
  await p.fill('input[aria-label="Contract name"]', "Monthly voters");
  await p.fill('input[aria-label="File-name pattern"]', "voters_*.csv");
  // id: every value differs, so it asks first; Cancel leaves it unticked.
  await p.click('input[aria-label="Keep the values of id"]');
  await p.waitForTimeout(300);
  dt = await dialogText();
  ok(/looks like personal data/.test(await p.evaluate(() => document.body.innerText)), "a column that looks like personal data asks first");
  await p.click('[role="alertdialog"] button:has-text("Cancel"), [role="dialog"] button:has-text("Cancel") >> nth=-1');
  await p.waitForTimeout(300);
  ok(!(await p.isChecked('input[aria-label="Keep the values of id"]')), "and Cancel leaves it unticked");
  await p.click('input[aria-label="Keep the values of party"]');
  await p.waitForTimeout(200);
  ok(await p.isChecked('input[aria-label="Keep the values of party"]'), "party (few values) ticks with no question");
  if (out) await p.screenshot({ path: path.join(out, "contract-make.png") });
  await p.click('[role="dialog"] button.primary');
  await p.waitForTimeout(500);
  let st = await status();
  ok(/Saved the contract Monthly voters/.test(st), `Save writes it (${st})`);
  let saved = await p.evaluate(() => window.__nde.backend.contracts[0]);
  ok(saved && saved.recognise.pattern === "voters_*.csv" && saved.columns[2].allowed?.values.join() === "DEM,REP" && !saved.columns[0].allowed, "it keeps the pattern and party's values only");

  // ---- February: drifted ----
  const feb = [["id", "full_name", "phone", "party", "email"], ["7", "Gus Elm", "", "DEM", "g@example.com"], ["8", "Hal Ash", "", "GRN", "h@example.com"], ["9", "Ivy Fir", "555-0108", "REP", "i@example.com"], ["10", "Jo Yew", "", "GRN", "j@example.com"]];
  await open("voters_feb.csv", feb);
  await p.waitForTimeout(400);
  let bt = await banner();
  ok(/Looks like Monthly voters/.test(bt) && /1 renamed column/.test(bt) && /and 2 more/.test(bt), `the bar says what drifted, the first few and how many more (${bt})`);
  if (out) await p.screenshot({ path: path.join(out, "contract-banner.png") });
  await p.click('[data-testid="contract-banner"] button:has-text("Review")');
  await p.waitForTimeout(400);
  let pt = await panel();
  ok(/full_name is probably name/.test(pt), "Review: renamed, by place");
  ok(/New: email/.test(pt) && /Moved:/.test(pt), "Review: a new column and moved ones");
  ok(/2 cells in party: GRN/.test(pt), `Review: new values with the first ones (${pt.match(/\d+ cells? in party[^.]*/)?.[0]})`);
  ok(/3 blank cells in phone: 75%, up \d+ points/.test(pt), "Review: the blank share that moved");
  if (out) await p.screenshot({ path: path.join(out, "contract-review.png") });

  // A count shows its rows as a filter chip.
  await p.click('[data-testid="contract-panel"] button.link:has-text("2 cells")');
  await p.waitForTimeout(500);
  let info = await p.evaluate(() => window.__nde.tab.info);
  const chipText = await p.evaluate(() => document.body.innerText);
  ok(info.rows === 3 && /New values in party/.test(chipText), `the count shows its 2 rows as a chip (${info.rows - 1} rows)`);
  await p.evaluate(() => window.__nde.grid?.focus());
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(400);
  ok((await p.evaluate(() => window.__nde.tab.info.rows)) === 5, "Ctrl+Z takes the chip off");

  // Fix what I can: one step, then one Ctrl+Z.
  await p.click('[data-testid="contract-panel"] button:has-text("Fix what I can")');
  await p.waitForTimeout(300);
  dt = await p.evaluate(() => document.body.innerText);
  ok(/Rename full_name to name/.test(dt), "Fix lists the renames first");
  await p.click('button:has-text("Fix") >> nth=-1');
  await p.waitForTimeout(600);
  let tt = await titles();
  ok(tt.join() === "id,name,party,phone,email", `titles and order follow the contract (${tt.join()})`);
  st = await status();
  ok(/Fixed: renamed full_name to name, put the columns in the contract's order/.test(st), `the status line says what changed (${st})`);
  await p.evaluate(() => window.__nde.grid?.focus());
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(500);
  tt = await titles();
  ok(tt.join() === "id,full_name,phone,party,email", `one Ctrl+Z puts both back (${tt.join()})`);

  // Accept these new values, Update the baseline: they save the contract.
  await p.waitForTimeout(300);
  if (!(await p.$('[data-testid="contract-panel"]'))) await p.click('[data-testid="contract-banner"] button:has-text("Review")');
  await p.waitForTimeout(300);
  await p.click('[data-testid="contract-panel"] button:has-text("Accept these new values")');
  await p.waitForTimeout(600);
  saved = await p.evaluate(() => window.__nde.backend.contracts[0]);
  ok(saved.columns[2].allowed.values.includes("GRN"), "Accept these new values saves them into the contract");
  bt = await banner();
  ok(!/new value/.test(bt), `the check runs again: no new values now (${bt})`);
  await p.click('[data-testid="contract-panel"] button:has-text("Update the baseline")');
  await p.waitForTimeout(600);
  saved = await p.evaluate(() => window.__nde.backend.contracts[0]);
  ok(saved.columns[3].blank_rate === 0.75, `Update the baseline takes today's blank shares (${saved.columns[3].blank_rate})`);
  await p.click('[data-testid="contract-panel"] button[aria-label="Close"]');

  // Not this contract.
  await p.click('[data-testid="contract-banner"] button:has-text("Not this contract")');
  await p.waitForTimeout(200);
  ok(!(await banner()), "Not this contract takes the bar away");

  // ---- Cancel stops a check; Check again runs it ----
  await p.evaluate(() => (window.__nde.backend.contractDelay = 1500));
  await open("voters_mar.csv", feb);
  bt = await banner();
  ok(/Checking against Monthly voters/.test(bt), `the check runs after the file is open, with Cancel (${bt})`);
  await p.click('[data-testid="contract-banner"] button:has-text("Cancel")');
  await p.waitForTimeout(1700);
  bt = await banner();
  ok(/The check stopped/.test(bt) && (await calls("contractDriftStop")).length > 0, `Cancel stops it (${bt})`);
  await p.evaluate(() => (window.__nde.backend.contractDelay = 0));
  await p.click('[data-testid="contract-banner"] button:has-text("Check again")');
  await p.waitForTimeout(400);
  ok(/Looks like Monthly voters:/.test(await banner()), "Check again runs it");

  // ---- a private tab: greyed with the reason ----
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    t.private = await window.__nde.backend.setPrivate(t.id, false, true);
  });
  const c = await command("data.contractMake");
  ok(!c.enabled && /private.*would keep its titles and values/.test(c.why), `Make a contract is greyed on a private tab (${c.why})`);

  // ---- the Contracts list ----
  await run("data.contracts");
  await p.waitForTimeout(400);
  dt = await dialogText();
  ok(/File contracts/.test(dt) && /Monthly voters/.test(dt) && /files like voters_\*\.csv/.test(dt), "the list shows the contract");
  await p.click('[role="option"]:has-text("Monthly voters")');
  await p.click('[role="dialog"] button:has-text("Export…")');
  await p.waitForTimeout(300);
  ok((await calls("contractExport:Monthly voters")).length === 1, "Export writes it to a file");
  await p.click('[role="dialog"] button:has-text("Delete")');
  await p.waitForTimeout(300);
  await p.click('button.danger:has-text("Delete"), [role="alertdialog"] button:has-text("Delete") >> nth=-1');
  await p.waitForTimeout(400);
  ok((await p.evaluate(() => window.__nde.backend.contracts.length)) === 0 && /No contracts yet/.test(await dialogText()), "Delete asks, then removes it");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(200);

  // ---- Alt+Q ----
  await p.evaluate(() => window.__nde.grid?.focus());
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("file contracts");
  await p.waitForTimeout(500);
  const found = await p.evaluate(() => document.body.innerText);
  if (!/Make a contract from this file/.test(found)) console.log(found.slice(0, 1500));
  ok(/Make a contract from this file/.test(found) && /File contracts/.test(found), "Alt+Q finds Make a contract and File contracts");
  await p.keyboard.press("Escape");

  ok(errors.length === 0, `no page errors (${errors.join("; ")})`);
  await b.close();
  console.log(failed ? `${failed} FAILED` : "ALL PASSED");
  process.exit(failed ? 1 : 0);
})();
