// UI test of Group look-alike values by sound or spelling (the Value editor):
// the groups by sound and by spelling, picking another winner, leaving a
// value out, putting groups in the list of changes, applying them as one
// undo step, and Cancel while it looks.
// Run: (cd ui && npx vite --port 5211) then
// NODE_PATH=<folder with playwright> node scripts/ui-lookalikes.cjs [screenshot folder] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5211";
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
  const command = async (q) => {
    await p.keyboard.press("Alt+q");
    await p.keyboard.type(q);
    await wait(300);
    await p.keyboard.press("Enter");
    await wait(600);
  };
  const column = () => p.evaluate(async () => (await window.__nde.backend.viewport(window.__nde.tab.id, 0, 20, 0, 1, 50)).cells);
  const dialog = p.locator("[role=dialog]:has(h2:has-text('Edit values'))");
  const groups = () => dialog.locator(".groups .group");
  const rowsOf = async () => groups().evaluateAll((els) => els.map((e) => e.textContent.replace(/\s+/g, " ").trim()));

  await p.goto(url);
  await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });
  await p.keyboard.press("Control+n");
  await wait(700);
  const names = ["name", "Smith", "Smith", "Smith", "Smyth", "Smythe", "Johnson", "Johnson", "Jonson", "Catherine", "Kathryn", "Lee", "Lee", "Lew", "Martin", "Marten", "Ward 1", "Ward 2"];
  await p.evaluate((t) => navigator.clipboard.writeText(t), names.join("\r\n") + "\r\n");
  await p.keyboard.press("Control+v");
  await wait(800);
  await p.evaluate(() => {
    window.__nde.tab.header = true;
    window.__nde.tab.bump();
    window.__nde.grid.select({ row: 1, col: 0 });
  });
  await wait(300);

  // 1. By sound: Smith/Smyth/Smythe and Catherine/Kathryn; Lee and Lew stay apart.
  await command("Group look-alike values");
  ok(await dialog.isVisible(), "the command opens the value editor");
  const method = dialog.locator("select[aria-label='Group look-alikes by']");
  ok((await method.inputValue()) === "both", "it starts on sound or spelling");
  await method.selectOption("sounds");
  await wait(800);
  let rows = await rowsOf();
  ok(rows.some((r) => /Smith \(3\)/.test(r) && /Smyth \(1\)/.test(r) && /Smythe \(1\)/.test(r)), `sound: Smith, Smyth, Smythe (${rows.length} groups)`);
  ok(rows.some((r) => /Catherine/.test(r) && /Kathryn/.test(r)), "sound: Catherine and Kathryn");
  ok(!rows.some((r) => /Lee/.test(r) && /Lew/.test(r)), "Lee and Lew are not joined");
  ok(!rows.some((r) => /Ward 1/.test(r) && /Ward 2/.test(r)), "values that differ only in digits are not joined");
  if (out) await dialog.screenshot({ path: path.join(out, "lookalikes.png") });

  // 2. By spelling: Johnson/Jonson, Martin/Marten.
  await method.selectOption("spelling");
  await wait(800);
  rows = await rowsOf();
  ok(rows.some((r) => /Johnson \(2\)/.test(r) && /Jonson \(1\)/.test(r)), "spelling: Johnson and Jonson");
  ok(rows.some((r) => /Martin/.test(r) && /Marten/.test(r)), "spelling: Martin and Marten");

  // 3. Both; leave Smythe out of its group, pick another winner, put in list.
  await method.selectOption("both");
  await wait(800);
  const smith = groups().filter({ hasText: "Smith (3)" }).first();
  await smith.getByLabel("Include Smythe").uncheck();
  await smith.getByRole("button", { name: "Put in list" }).click();
  await wait(300);
  const johnson = groups().filter({ hasText: "Johnson (2)" }).first();
  await johnson.locator("select[aria-label='Becomes']").selectOption("Jonson");
  await johnson.getByRole("button", { name: "Put in list" }).click();
  await wait(300);
  const note = (await dialog.textContent()) ?? "";
  ok(/2 values will change \(3 cells\)/.test(note), "the list holds Smyth and Johnson (Smythe was left out): 2 values, 3 cells");
  await dialog.getByRole("button", { name: "Change" }).click();
  await wait(1000);
  let col = await column();
  ok(col[4] === "Smith" && col[5] === "Smythe" && col[1] === "Smith", `Smyth became Smith; Smythe left alone (${col.slice(1, 6)})`);
  ok(col[6] === "Jonson" && col[7] === "Jonson", `Johnson became the chosen Jonson (${col.slice(6, 9)})`);

  // 4. One undo step puts everything back.
  await p.keyboard.press("Control+z");
  await wait(600);
  col = await column();
  ok(col[4] === "Smyth" && col[6] === "Johnson" && col[7] === "Johnson", "Ctrl+Z puts all of it back at once");

  // 5. Many groups at once stays quick (the real file has tens of thousands).
  await p.keyboard.press("Control+n");
  await wait(700);
  const word = (i) => {
    // Consonants with an "m" between, so no two words share a sound key and
    // no letter doubles.
    let s = "";
    for (let k = 0; k < 5; k++, i = Math.floor(i / 9)) s += (k ? "m" : "") + "bdgjlnprt"[i % 9];
    return s;
  };
  const many = ["name"];
  for (let i = 0; i < 15000; i++) many.push(word(i) + "son", word(i) + "sen");
  await p.evaluate((t) => navigator.clipboard.writeText(t), many.join("\r\n") + "\r\n");
  await p.keyboard.press("Control+v");
  await wait(1500);
  await p.evaluate(() => {
    window.__nde.tab.header = true;
    window.__nde.tab.bump();
    window.__nde.grid.select({ row: 1, col: 0 });
  });
  await command("Group look-alike values");
  await dialog.locator("select[aria-label='Group look-alikes by']").selectOption("sounds");
  await dialog.locator("legend:has-text('group')").waitFor({ timeout: 60000 });
  const lg = (await dialog.locator("legend").first().textContent()) ?? "";
  ok(/15,000 groups/.test(lg), `30,000 values make 15,000 groups (${lg.replace(/\s+/g, " ").trim()})`);
  const t0 = Date.now();
  await dialog.getByRole("button", { name: "Put all groups in the list" }).click();
  await dialog.locator("p.good").first().waitFor({ timeout: 20000 });
  const took = Date.now() - t0;
  ok(took < 5000, `Put all groups in the list for 15,000 groups takes ${took} ms`);
  ok(/15000 values will change|15,000 values will change/.test((await dialog.locator("p.good").first().textContent()) ?? ""), "the list holds 15,000 changes");

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
