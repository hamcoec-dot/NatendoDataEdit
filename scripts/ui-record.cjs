// UI test of the Record panel in the browser stand-in: it follows the selection, flips rows, searches, pins (kept
// by title), shows hidden columns only when asked, edits in place as one undo step with Esc to revert, refuses
// protected columns and locked files, opens a long value in the expander, and draws only the fields in view on a
// file with 600 columns.
// Run: (cd ui && npx vite --port 5261) then NODE_PATH=<folder with playwright> node scripts/ui-record.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5261";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1280, height: 820 } });
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
  const fill = async (text) => {
    await p.evaluate(() => window.__nde.reg.run("file.new"));
    await p.waitForTimeout(500);
    await p.evaluate(async (text) => {
      const t = window.__nde.tab;
      t.update(await window.__nde.backend.dataOp(t.id, { op: "paste", spec: { row: 0, col: 0, op: "none", skip_blanks: false, transpose: false }, text }).then((r) => r.info), true);
      t.header = true;
      t.bump();
    }, text);
    await p.waitForTimeout(400);
  };
  const TAB = String.fromCharCode(9);
  const NL = String.fromCharCode(10);
  const EOL = String.fromCharCode(13, 10);
  const lines = [["Name", "City", "Notes", "Age"]];
  for (let i = 0; i < 20; i++) lines.push([`Person${i}`, `City${i % 5}`, i === 3 ? "x".repeat(200) : `note ${i}`, String(20 + i)]);
  await fill(lines.map((r) => r.join(TAB)).join(EOL));
  const sel = (row, col) => p.evaluate(([r, c]) => window.__nde.grid.select({ row: r, col: c }), [row, col]);
  const panel = () => p.getByTestId("record-panel");
  const field = (name) => panel().getByLabel(name, { exact: true });
  const cell = (r, c) => p.evaluate(async ([r, c]) => (await window.__nde.backend.cellText(window.__nde.tab.id, r, c)).text, [r, c]);

  await sel(2, 0);
  await p.keyboard.press("Alt+r");
  await panel().waitFor({ timeout: 5000 });
  await p.waitForTimeout(500);
  ok((await field("Name").inputValue()) === "Person1", "Alt+R opens the panel on the active row (Person1)");
  ok(/Row 2 of 20/.test((await p.getByTestId("record-pos").textContent()) || ""), "it says which row of how many");
  if (out) await p.screenshot({ path: path.join(out, "record.png") });

  await sel(5, 1);
  await p.waitForTimeout(400);
  ok((await field("Name").inputValue()) === "Person4", "it follows the selection to another row");
  await p.getByRole("button", { name: "Next row" }).click();
  await p.waitForTimeout(400);
  ok((await field("Name").inputValue()) === "Person5", "the next-row button flips");
  await p.keyboard.press("Control+Alt+ArrowUp");
  await p.waitForTimeout(400);
  ok((await field("Name").inputValue()) === "Person4", "Ctrl+Alt+Up flips back from anywhere");

  await p.getByLabel("Search fields by title or value").fill("city");
  await p.waitForTimeout(200);
  ok((await panel().locator("input.value").count()) === 1, "search by title leaves one field");
  await p.getByLabel("Search fields by title or value").fill("note 4");
  await p.waitForTimeout(200);
  ok((await field("Notes").count()) === 1 && (await field("City").count()) === 0, "search by value finds the field");
  await p.getByLabel("Search fields by title or value").fill("");

  await p.getByRole("button", { name: "Pin Age to the top" }).click();
  await p.waitForTimeout(200);
  const first = await panel().locator("input.value").first().getAttribute("aria-label");
  ok(first === "Age", `a pinned field goes to the top (${first})`);
  const pins = await p.evaluate(() => window.__nde.tab.recordPins);
  ok(JSON.stringify(pins) === '["Age"]', "pins are kept by title on the tab");
  await sel(8, 0);
  await p.waitForTimeout(300);
  ok((await panel().locator("input.value").first().getAttribute("aria-label")) === "Age", "flipping rows keeps the pin");

  await field("City").fill("Paris");
  await field("City").press("Enter");
  await p.waitForTimeout(500);
  ok((await cell(8, 1)) === "Paris", "Enter saves the field");
  await sel(8, 0);
  await p.evaluate(() => window.__nde.grid.focus());
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(500);
  ok((await cell(8, 1)) === "City2", "Ctrl+Z takes the one field back in one step");
  await field("City").fill("Rome");
  await field("City").press("Escape");
  await p.waitForTimeout(300);
  ok((await field("City").inputValue()) === "City2" && (await cell(8, 1)) === "City2", "Esc puts the field back and changes nothing");

  await field("Name").fill("Zed");
  await field("Age").focus();
  await p.waitForTimeout(500);
  ok((await cell(8, 0)) === "Zed", "leaving a changed field saves it");

  // Typing in a field and flipping rows by keyboard: the typed value is saved into the row it was typed in.
  await sel(8, 0);
  await p.waitForTimeout(400);
  const nextNote = await cell(9, 2);
  await field("Notes").fill("typed-note");
  await p.keyboard.press("Control+Alt+ArrowDown");
  await p.waitForTimeout(700);
  ok((await cell(8, 2)) === "typed-note", "a field typed in before a keyboard flip is saved into its own row");
  ok((await cell(9, 2)) === nextNote, "and the next row is not touched");
  ok((await field("Name").inputValue()) === "Person8", "the panel then shows the next row");

  // A value with a line break edits in a textarea and keeps the break.
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    t.update(await window.__nde.backend.setCells(t.id, [[6, 2, "12 Main St" + String.fromCharCode(10) + "Apt 4"]]), true);
  });
  await sel(6, 0);
  await p.waitForTimeout(500);
  const ta = panel().locator("textarea.value");
  ok((await ta.count()) === 1, "a value with a line break gets a multi-line box");
  await ta.fill("12 Main St" + NL + "Apt 4B");
  await ta.press("Control+Enter");
  await p.waitForTimeout(500);
  ok((await cell(6, 2)) === "12 Main St" + NL + "Apt 4B", "the line break survives an edit");
  // Leaving a multi-line field unchanged does not rewrite it.
  await field("Name").focus();
  await field("Name").press("Tab");
  await p.waitForTimeout(300);
  ok((await cell(6, 2)) === "12 Main St" + NL + "Apt 4B", "leaving it alone keeps it");

  await sel(4, 0);
  await p.waitForTimeout(400);
  await p.getByRole("button", { name: "Open Notes in a bigger editor" }).click();
  await p.waitForTimeout(300);
  const box = p.getByRole("dialog", { name: "Edit a long value" }).locator("textarea");
  ok((await box.inputValue()).length === 200, `the expander holds the whole value (${(await box.inputValue()).length})`);
  await box.fill("short");
  await p.getByRole("dialog", { name: "Edit a long value" }).getByRole("button", { name: "Save" }).click();
  await p.waitForTimeout(400);
  ok((await cell(4, 2)) === "short", "the expander saves");

  await p.evaluate(async () => {
    const t = window.__nde.tab;
    t.update(await window.__nde.backend.setProtected(t.id, [3]), false);
  });
  await p.waitForTimeout(400);
  ok(await field("Age").isDisabled(), "a protected column's field is disabled");
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    t.update(await window.__nde.backend.setReadOnly(t.id, true), false);
  });
  await p.waitForTimeout(400);
  ok(await field("Name").isDisabled(), "a locked file's fields are disabled");
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    t.update(await window.__nde.backend.setReadOnly(t.id, false), false);
  });

  await p.evaluate(() => (window.__nde.tab.columns.setWidth(1, 0), window.__nde.tab.bump()));
  await p.waitForTimeout(300);
  const hiddenGone = (await field("City").count()) === 0;
  await p.getByLabel("Show hidden columns (dimmed)").check();
  await p.waitForTimeout(300);
  ok(hiddenGone && (await field("City").count()) === 1, "a hidden column shows only when asked");

  const wide = [Array.from({ length: 600 }, (_, i) => `C${i}`), Array.from({ length: 600 }, (_, i) => `v${i}`)];
  await fill(wide.map((r) => r.join(TAB)).join(EOL));
  await sel(1, 0);
  await p.waitForTimeout(700);
  const drawn = await panel().locator("input.value").count();
  ok(drawn > 0 && drawn < 100, `600 columns draw only the fields in view (${drawn})`);

  ok(errors.length === 0, `no page errors ${errors.join(" | ").slice(0, 200)}`);
  await b.close();
  process.exit(failed ? 1 : 0);
})();
