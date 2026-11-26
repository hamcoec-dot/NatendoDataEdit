// UI test of golden records in the browser stand-in: match one list with itself, accept pairs, choose rules, the
// preview with its highlighted doubt, leaving a cluster out, a stale preview, the new tabs, a changed table
// refused, command search. (The stand-in's pairs are fixed and its rules simple; the real ones are the engine's.)
// Run: (cd ui && npx vite --port 5366) then NODE_PATH=<folder with playwright> node scripts/ui-golden.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5366";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const p = await (await b.newContext({ viewport: { width: 1360, height: 960 } })).newPage();
  const errors = [];
  p.on("pageerror", (e) => errors.push(String(e)));
  await p.goto(url);
  await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });
  let failed = 0;
  const ok = (cond, msg) => {
    if (!cond) failed++;
    console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  };
  const body = async () => ((await p.textContent("body")) || "").replace(/\s+/g, " ");
  const run = async (id) => {
    await p.evaluate((id) => void window.__nde.reg.run(id), id);
    await p.waitForTimeout(600);
  };
  const rows = [
    ["Member Id", "Name", "Town", "Paid"],
    ["101", "Zebedee Quill", "Yarrowby", "$5"],
    ["102", "Ottilie Vantree", "Mossford", "1"],
    ["103", "Perrin Ashgrove", "Hollin", "2.50"],
    ["104", "Zebedee Quil", "", "3"],
    ["105", "Isolde Fennick", "Brackwen", "4"],
    ["106", "Corwin Dellamar", "Wrayholt", "5"],
    ["107", "Maelis Thorne", "Hollin", "6"],
    ["108", "Tobiah Renwick", "Yarrowby", "7"],
    ["109", "Perin Ashgrove", "Hollin", "0.50"],
  ];
  await run("file.new");
  await p.evaluate(async (text) => {
    const t = window.__nde.tab;
    t.update(await window.__nde.backend.dataOp(t.id, { op: "paste", spec: { row: 0, col: 0, op: "none", skip_blanks: false, transpose: false }, text }).then((r) => r.info), true);
    t.header = true;
    t.headerTouched = true;
    t.bump();
  }, rows.map((r) => r.join("\t")).join("\r\n"));
  await p.waitForTimeout(400);
  const source = await p.evaluate(() => window.__nde.tab.id);
  const cells = () =>
    p.evaluate(async () => {
      const t = window.__nde.tab;
      const all = [];
      for (let r = 0; r < t.info.rows; r++) {
        const line = [];
        for (let c = 0; c < t.info.cols; c++) line.push((await window.__nde.backend.cellText(t.id, r, c)).text);
        all.push(line.join("|"));
      }
      return all.filter((x) => x.replace(/\|/g, ""));
    });
  const before = await cells();

  await run("data.match");
  const dlg = p.getByRole("dialog");
  await dlg.getByLabel("Match with").selectOption("-1");
  await p.waitForTimeout(300);
  if (await dlg.getByRole("button", { name: "Find matches" }).isDisabled()) await dlg.getByRole("button", { name: "Add a column to compare" }).click();
  await dlg.getByRole("button", { name: "Find matches" }).click();
  await p.waitForTimeout(700);
  const merge = dlg.getByRole("button", { name: "Merge accepted into golden records…" });
  ok(await merge.isDisabled(), "merging waits for an accepted pair");
  await dlg.getByLabel(/Accept every pair scoring/).fill("50");
  await dlg.getByRole("button", { name: "Accept", exact: true }).click();
  await merge.click();
  await p.waitForTimeout(500);
  ok(/Merge into golden records/.test(await body()) && /Your table is not changed/.test(await body()), "the golden screen");
  ok(!(await dlg.getByLabel("Only merge clusters where every pair was accepted").isChecked()), "the strict tick starts off");
  ok((await dlg.getByLabel("Name merged rows by").inputValue()) === "0", "the id column is guessed");
  const create = dlg.getByRole("button", { name: "Create the new table" });
  ok(await create.isDisabled(), "Create waits for a preview");
  await dlg.getByLabel("Rule for Name").selectOption("newest");
  const preview = dlg.getByRole("button", { name: "Preview", exact: true });
  ok(await preview.isDisabled(), "newest waits for the date column");
  await dlg.getByLabel("Rule for Name").selectOption("keep");
  await dlg.getByLabel("Rule for Paid").selectOption("sum");
  await preview.click();
  await p.waitForTimeout(700);
  let said = await body();
  ok(/2 clusters would merge: 2 rows folded away, 7 rows in the result\./.test(said), `the counts (${(said.match(/\d+ clusters? would[^.]*\./) || [""])[0]})`);
  ok(/Cluster 1: 101; 104/.test(said), "members named by the id column");
  ok((await dlg.locator("tr.doubt").count()) === 1 && /not an exact number: first value kept/.test(said), "the doubtful cell is highlighted with why");
  const second = dlg.locator("fieldset").nth(1);
  const paid = await second.locator("tr", { has: p.locator("th", { hasText: "Paid" }) }).locator("td").first().textContent();
  ok((paid || "").trim() === "3", `an exact sum in the second cluster (${paid})`);
  if (out) await p.screenshot({ path: path.join(out, "golden-preview.png") });
  await dlg.getByLabel("Largest cluster to merge").fill("5");
  ok(/preview again/.test(await body()) && (await create.isDisabled()), "a changed choice makes the preview stale");
  await preview.click();
  await p.waitForTimeout(700);
  await dlg.locator("fieldset").nth(1).getByLabel("Merge this cluster").uncheck();
  await create.click();
  await p.waitForTimeout(1200);
  said = await body();
  ok((await p.getByRole("dialog").count()) === 0 && /Golden records made as a new tab: 1 cluster merged, 8 rows/.test(said), `created (${(said.match(/Golden records made[^.]*\./) || [""])[0]})`);
  const result = await cells();
  ok(result.length === 9 && result[0].endsWith("|Merged_IDs|Merged_Count") && result[1] === "101|Zebedee Quill|Yarrowby|$5|101; 104|2" && result[3] === "103|Perrin Ashgrove|Hollin|2.50||", `the new table (${result.slice(0, 4).join(" / ")})`);
  const names = await p.evaluate(() => window.__nde.tabs.map((t) => t.info.name));
  ok(names.includes("Merged away") && names.at(-1) !== undefined, `the folded rows have a tab (${names.join(", ")})`);
  const i = await p.evaluate((x) => window.__nde.tabs.findIndex((t) => t.id === x), source);
  await p.locator("[role=tab], .tab").nth(i).click({ position: { x: 20, y: 8 } });
  await p.waitForTimeout(500);
  ok((await cells()).join() === before.join(), "the source is unchanged");

  // A table that changes under the screen.
  await run("data.match");
  await dlg.getByLabel("Match with").selectOption("-1");
  await p.waitForTimeout(300);
  if (await dlg.getByRole("button", { name: "Find matches" }).isDisabled()) await dlg.getByRole("button", { name: "Add a column to compare" }).click();
  await dlg.getByRole("button", { name: "Find matches" }).click();
  await p.waitForTimeout(600);
  await dlg.getByLabel(/Accept every pair scoring/).fill("50");
  await dlg.getByRole("button", { name: "Accept", exact: true }).click();
  await merge.click();
  await preview.click();
  await p.waitForTimeout(600);
  await p.evaluate(async (id) => void (await window.__nde.backend.setCells(id, [[1, 2, "Moved"]])), source);
  await create.click();
  await p.waitForTimeout(700);
  ok((await p.getByRole("dialog").count()) === 1 && /run Match people again/.test(await body()), "a changed table is refused");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);
  ok(/Merge accepted into golden records/.test(await body()), "Escape goes back to the pairs");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);
  await p.keyboard.press("Alt+q");
  await p.waitForTimeout(300);
  await p.keyboard.type("golden records");
  await p.waitForTimeout(500);
  ok(/Match people/.test(await body()), "command search finds it");
  ok(errors.length === 0, `no page errors (${errors.slice(0, 3).join("; ")})`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
