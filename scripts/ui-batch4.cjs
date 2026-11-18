// UI test of v0.4.4 (power-user batch 4): the recipe steps panel, recipe
// questions and running a recipe on many files.
// Run: (cd ui && npx vite --port 5203) then
// NODE_PATH=<folder with playwright> node scripts/ui-batch4.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5203";
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

  await p.goto(url);
  await p.keyboard.press("Control+n");
  await wait(700);
  const rows = ["voter_id\tcounty\tstatus"];
  for (let i = 0; i < 30; i++) rows.push(`${2001 + i}\t${["Hamilton", "Knox", "TBD"][i % 3]}\t${i % 4 ? "Active" : "Inactive"}`);
  await sel(0, 0);
  await clip(rows.join("\r\n") + "\r\n");
  await p.keyboard.press("Control+v");
  await wait(800);

  // ---- Steps panel ----
  await p.keyboard.press("Alt+Shift+R");
  await wait(800);
  const list = p.locator("ol[aria-label='Recipe steps']");
  ok(await list.isVisible(), "Alt+Shift+R opens the recipe steps panel");
  ok((await list.locator("li").count()) === 5, "it lists each step (and the start)");
  await list.locator("li").nth(1).locator("button.step").click();
  await wait(600);
  ok(/after step 1/.test(await p.locator(".peek").textContent()), "clicking a step shows the table as it was then");
  ok(await p.evaluate(() => window.__nde.tab.peeking), "the table can't be edited while looking back");
  if (out) await p.locator("aside.panel").screenshot({ path: path.join(out, "steps-panel.png") });
  await p.locator(".peek button").click();
  await wait(600);
  ok(!(await p.evaluate(() => window.__nde.tab.peeking)), "Back to the latest step");
  // Turning a step off would undo a paste: it asks first.
  await p.locator("button[aria-label='Turn off step 1']").click();
  await wait(600);
  const confirm = p.locator("[role=alertdialog]");
  ok(await confirm.isVisible(), "a change that would undo a paste asks first");
  await confirm.locator("button:has-text('Go ahead')").click();
  await wait(700);
  ok(/\(off\)/.test(await list.locator("li").nth(1).textContent()), "the step is off");
  // A value becomes a question.
  await p.locator("button[aria-label='Ask for a value in step 2']").click();
  await wait(300);
  await p.locator(".edit input[placeholder='county']").fill("county");
  await p.locator(".edit input[placeholder='Which county?']").fill("Which county?");
  await p.locator(".edit button:has-text('Ask each time')").click();
  await wait(500);
  ok(/Which county\? \(last: county\)/.test(await p.locator(".steps-panel").textContent()), "the value is asked each time the recipe runs");

  // ---- Run recipe on files ----
  await command("Run recipe on files");
  const d = dialog("Run a recipe on files");
  ok(await d.isVisible(), "Tools › Run recipe on files opens");
  await d.locator("button:has-text('Folder…')").click();
  await wait(300);
  const chooses = d.locator("button:has-text('Choose…')");
  await chooses.nth(0).click();
  await wait(300);
  await chooses.nth(1).click();
  await wait(300);
  ok((await d.locator("input[type=text]").inputValue()) === "Hamilton", "the recipe's question is asked with its last answer");
  await d.locator("input[type=text]").fill("Knox");
  await d.locator("button:has-text('Run')").click();
  await wait(700);
  ok(/2 files done; 1 file not done/.test(await d.textContent()), "a summary by file: done and not done");
  ok((await d.locator("[role=row]").count()) === 3, "each file has a line");
  if (out) await d.screenshot({ path: path.join(out, "recipe-files.png") });
  await d.locator("button:has-text('Close')").click();

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
