// UI test of the rule card (C10) against the vite dev server (browser stand-in backend): after Ctrl+E
// the status line gives the rule and the odd results with Show the rule; Data > Show me the rule
// (Ctrl+Shift+E) shows the rule as a form, the preview, the counts and the odd rows; a change to the
// form is read back and previewed; Apply (Ctrl+Enter; Enter only away from the boxes) fills with that
// rule as one undo step; examples that disagree open the card on the one that breaks the rule, with
// Leave it out (the cell stays as typed) and Go to row; Save as formula adds a computed column, and is
// greyed with the reason for a column read as numbers; Alt+Q finds it. Made-up data only.
// Run: (cd ui && npx vite --port 5321) then
// NODE_PATH=<folder with playwright> node scripts/ui-rulecard.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5321";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1360, height: 900 } });
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
  const card = () => p.$('[role="dialog"]:has-text("The rule for")');
  const cardText = async () => ((await (await card())?.textContent()) || "").replace(/\s+/g, " ");
  const cell = (r, c) => p.evaluate(([r, c]) => window.__nde.backend.value(window.__nde.backend.get(window.__nde.tab.id), r, c), [r, c]);
  const col = (c) => p.evaluate(async (c) => {
    const n = window.__nde;
    const rows = n.backend.rowCount(n.backend.get(n.tab.id));
    return Array.from({ length: rows }, (_, r) => n.backend.value(n.backend.get(n.tab.id), r, c));
  }, c);
  const open = (name, rows) =>
    p.evaluate(
      ([name, rows]) => {
        const n = window.__nde;
        n.addTab(n.backend.mockRowsTab(name, rows));
      },
      [name, rows],
    );
  const at = async (row, c) => {
    await p.evaluate(([row, c]) => {
      const n = window.__nde;
      n.tab.header = true;
      n.grid?.select({ row, col: c });
      n.grid?.focus();
    }, [row, c]);
    await p.waitForTimeout(150);
  };
  const settle = () => p.waitForTimeout(700);

  const ROWS = [
    ["first", "last", "short"],
    ["Ann", "Lee", "A.Lee"],
    ["Bo", "Kim", "B.Kim"],
    ["Cy", "Hart", ""],
    ["Di", "O'Neil", ""],
    ["Ed", "", ""],
    ["Fay", "12", ""],
    ["Gus", "Abcdefghijklmnopqrstuvwxyz", ""],
  ];
  await open("people", ROWS);
  await p.waitForTimeout(400);

  // ---- Ctrl+E: the status line gives the rule and the odd results ----
  await at(1, 2);
  await p.keyboard.press("Control+e");
  await settle();
  let st = await status();
  ok(/Filled 4 cells from 2 examples: first letter of first \+ "\." \+ last\./.test(st), `Ctrl+E fills and gives the rule in words (${st})`);
  ok(/1 row left empty/.test(st), "it says the row it could not fill");
  ok(/3 results look odd \(2 shaped unlike the examples, 1 far longer or shorter\)/.test(st), `and how many results look odd (${st})`);
  ok(!!(await p.$('footer.status button:has-text("Show the rule")')), "with Show the rule on the status line");
  ok((await cell(3, 2)) === "C.Hart", "the fill wrote C.Hart");
  await p.click('footer.status button:has-text("Show the rule")');
  await settle();
  ok(!!(await card()), "Show the rule opens the card");
  await p.keyboard.press("Escape");
  await settle();
  await p.evaluate(() => window.__nde.grid?.focus());
  await p.keyboard.press("Control+z");
  await settle();
  ok((await cell(3, 2)) === "", "one Ctrl+Z takes the whole fill back");

  // ---- Data > Show me the rule: the form, the preview, the counts ----
  await at(1, 2);
  await p.keyboard.press("Control+Shift+E");
  await settle();
  let ct = await cardText();
  ok(/The rule for short/.test(ct), "Ctrl+Shift+E opens the rule card on the column");
  ok(/Rule: first letter of first \+ "\." \+ last/.test(ct), `it gives the rule in words (${ct.slice(0, 120)})`);
  const pieces = await p.$$('ol[aria-label="The rule\'s pieces"] > li');
  ok(pieces.length === 3, `the rule is a form of 3 pieces (${pieces.length})`);
  const prevRows = await p.$$eval("table.preview tbody tr", (trs) => trs.map((tr) => tr.textContent.replace(/\s+/g, " ").trim()));
  ok(prevRows.length === 5 && /C\.Hart/.test(prevRows[0]) && /left empty/.test(prevRows[2]), `the preview lists the cells it would fill (${prevRows.join(" | ")})`);
  ok(/Showing the 5 found in the first 7 rows\./.test(ct), "and says how far it looked");
  ok(/It fills 4 cells and leaves 1 empty \(a piece is missing\)\. 3 results look odd:/.test(ct), `the counts (${ct.match(/It fills[^:]*:/)?.[0]})`);
  ok(/Row 7 “F\.12”: shaped unlike the examples/.test(ct) && /Row 8 “GAbcdefghijklmnopqrstuvwxyz”|Row 8 “G\.Abcdefghijklmnopqrstuvwxyz”: far longer or shorter/.test(ct), "each odd result with its row and why");
  if (out) await p.screenshot({ path: path.join(out, "rule-card.png") });

  // ---- change the form: read back, previewed ----
  const letters = await p.$$('select[aria-label="Letters"]');
  await letters[1].selectOption("upper");
  await settle();
  ct = await cardText();
  ok(/Rule: first letter of first \+ "\." \+ last, in capitals/.test(ct), "a change to the form is read back in words");
  ok(/C\.HART/.test(ct), "and the preview shows what it would write now");
  const formulaBtn = await p.$('[role="dialog"] button:has-text("Save as formula")');
  ok(!(await formulaBtn.isDisabled()), "Save as formula can be pressed");

  // ---- Enter in a box does not apply; Ctrl+Enter does ----
  await p.focus('[role="dialog"] input[type="text"]');
  await p.keyboard.press("Enter");
  await settle();
  ok(!!(await card()), "Enter in a text box does not apply");
  await p.keyboard.press("Control+Enter");
  await settle();
  ok(!(await card()), "Ctrl+Enter applies and closes the card");
  ok((await cell(3, 2)) === "C.HART" && (await cell(4, 2)) === "D.O'NEIL", `it filled with the changed rule (${await cell(3, 2)}, ${await cell(4, 2)})`);
  st = await status();
  ok(/Filled 4 cells/.test(st) && /look odd/.test(st), `the status line counts as after Ctrl+E (${st})`);
  await p.evaluate(() => window.__nde.grid?.focus());
  await p.keyboard.press("Control+z");
  await settle();
  ok((await cell(3, 2)) === "" && (await cell(4, 2)) === "", "one Ctrl+Z takes the card's fill back");

  // ---- examples that disagree: the card names the one ----
  await p.evaluate(async () => {
    const n = window.__nde;
    n.tab.update(await n.backend.setCells(n.tab.id, [[3, 2, "Hart"]]), true);
  });
  await at(1, 2);
  await p.keyboard.press("Control+e");
  await settle();
  ct = await cardText();
  ok(/Row 4 does not fit the rule the other examples share: you typed “Hart”\./.test(ct), `Ctrl+E with an example that disagrees opens the card on it (${ct.slice(0, 120)})`);
  if (out) await p.screenshot({ path: path.join(out, "rule-card-conflict.png") });
  await p.click('[role="dialog"] button:has-text("Leave it out")');
  await settle();
  ct = await cardText();
  ok(/Row 4 is left out of learning and stays as you typed it\./.test(ct) && !/does not fit/.test(ct), "Leave it out learns again without it");
  await p.click('[role="dialog"] button.primary');
  await settle();
  st = await status();
  ok(/Row 4 was left as you typed it\./.test(st), `Apply says the left-out row stayed (${st})`);
  ok((await cell(3, 2)) === "Hart" && (await cell(4, 2)) === "D.O'Neil", "and it did");
  await p.evaluate(() => window.__nde.grid?.focus());
  await p.keyboard.press("Control+z");
  await settle();

  // ---- Go to row ----
  await at(1, 2);
  await p.keyboard.press("Control+Shift+E");
  await settle();
  await p.click('[role="dialog"] button:has-text("Go to row 4")');
  await settle();
  const sel = await p.evaluate(() => window.__nde.tab.cell);
  ok(!(await card()) && sel.row === 3 && sel.col === 2, `Go to row closes the card and selects the cell (${JSON.stringify(sel)})`);
  await p.evaluate(async () => {
    const n = window.__nde;
    n.tab.update(await n.backend.setCells(n.tab.id, [[3, 2, ""]]), true);
  });

  // ---- Save as formula: greyed with the reason for a column read as numbers ----
  await p.evaluate(async () => {
    const n = window.__nde;
    n.tab.update(await n.backend.setColumnTypes(n.tab.id, { types: ["auto", "number"] }), true);
  });
  await at(1, 2);
  await p.keyboard.press("Control+Shift+E");
  await settle();
  let fb = await p.$('[role="dialog"] button:has-text("Save as formula")');
  ok((await fb.isDisabled()) && /last is read as numbers or dates/.test((await fb.getAttribute("title")) || ""), `greyed, with the reason (${await fb.getAttribute("title")})`);
  ok(/No formula form: last is read as numbers or dates/.test(await cardText()), "the reason is also written beside the buttons");
  await p.keyboard.press("Escape");
  await settle();
  await p.evaluate(async () => {
    const n = window.__nde;
    n.tab.update(await n.backend.setColumnTypes(n.tab.id, { types: [] }), true);
  });
  await at(1, 2);
  await p.keyboard.press("Control+Shift+E");
  await settle();
  const cols = await p.evaluate(() => window.__nde.tab.info.cols);
  await p.click('[role="dialog"] button:has-text("Save as formula")');
  await settle();
  const after = await p.evaluate(() => window.__nde.tab.info.cols);
  ok(after === cols + 1 && (await cell(0, 3)) === "short (formula)", `Save as formula adds "short (formula)" to the right (${cols} → ${after}, ${await cell(0, 3)})`);
  st = await status();
  ok(/Computed/.test(st), `as a computed column (${st})`);
  await p.evaluate(() => window.__nde.grid?.focus());
  await p.keyboard.press("Control+z");
  await settle();

  // ---- the Data menu and Alt+Q ----
  const menu = await p.evaluate(() => document.body.innerText.includes("Show me the rule"));
  await p.evaluate(() => window.__nde.grid?.focus());
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("show me the rule");
  await p.waitForTimeout(500);
  const found = await p.evaluate(() => document.body.innerText);
  ok(/Show me the rule/.test(found) && /Ctrl\+Shift\+E/.test(found), "Alt+Q finds Show me the rule with its keys");
  await p.keyboard.press("Escape");
  void menu;

  ok(errors.length === 0, `no page errors (${errors.join("; ")})`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
