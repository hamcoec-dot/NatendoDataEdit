// UI test of Data > Find relationships in the browser stand-in: columns unticked, a planted dependency found, the
// "Make this a check" (adds the data check), the exceptions view, copy, a stale refusal and command search.
// (The stand-in finds dependencies only; the real rules are the engine's tests.)
// Run: (cd ui && npx vite --port 5364) then NODE_PATH=<folder with playwright> node scripts/ui-relations.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5364";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1360, height: 860 }, permissions: ["clipboard-read", "clipboard-write"] });
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
  const run = async (id) => {
    await p.evaluate((id) => void window.__nde.reg.run(id), id);
    await p.waitForTimeout(600);
  };
  const text = async () => ((await p.locator("body").textContent()) || "").replace(/\s+/g, " ");
  await p.keyboard.press("Control+n");
  await p.waitForTimeout(600);
  // Made-up ZIP and state rows, two of them wrong.
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    const rows = [["ZIP", "State", "Voter"]];
    for (let i = 0; i < 200; i++) rows.push([`Z${i % 8}`, i === 5 || i === 77 ? "XX" : `S${(i % 8) % 3}`, `V${i}`]);
    t.update(await window.__nde.backend.change(t.id, { op: "insert_rows", at: 1, count: rows.length - 1 }), true);
    t.update(await window.__nde.backend.setCells(t.id, rows.flatMap((line, r) => line.map((v, c) => [r, c, v]))), true);
    t.header = true;
    t.headerTouched = true;
    t.bump();
  });
  await p.waitForTimeout(400);
  await run("data.relations");
  const dlg = p.getByRole("dialog", { name: "Find relationships" });
  await dlg.waitFor({ timeout: 5000 });
  ok((await dlg.locator(".cols input:checked").count()) === 0, "columns start unticked");
  const find = dlg.getByRole("button", { name: "Find", exact: true });
  ok(await find.isDisabled(), "Find waits for two columns");
  await dlg.getByRole("button", { name: "Select all", exact: true }).click();
  await find.click();
  await p.waitForTimeout(800);
  let t = await text();
  ok(/ZIP -> State: holds in 99\.0% \(198 of 200 rows whose ZIP repeats; 1 value breaks it\)/.test(t), "the dependency with its coverage");
  const check = dlg.getByRole("button", { name: "Make line 1 a check" });
  ok(!(await check.isDisabled()) && /marks the rows that break it/.test((await check.getAttribute("title")) || ""), "Make this a check is offered, saying what it adds");
  await check.click();
  await p.waitForTimeout(800);
  const added = await p.evaluate(() => (window.__nde.tab.checks || []).map((c) => c.kind?.type ?? c.kind));
  ok((await check.textContent()).includes("Added") && (await check.isDisabled()) && added.includes("settled_by"), `Make this a check adds a "same for each value" data check (${JSON.stringify(added)})`);
  if (out) await p.screenshot({ path: path.join(out, "relations.png") });
  await dlg.getByRole("button", { name: "Copy", exact: true }).click();
  await p.waitForTimeout(300);
  const clip = await p.evaluate(() => navigator.clipboard.readText());
  ok(/^ZIP -> State: holds in 99\.0%/.test(clip) && !/Z1|XX|V5/.test(clip), `copy holds titles and figures, no value (${JSON.stringify(clip.slice(0, 90))})`);
  await dlg.locator("input[type=number]").fill("80");
  ok(/press Find again/.test(await text()), "a new threshold makes the result stale");
  await dlg.locator("input[type=number]").fill("95");
  await dlg.getByRole("button", { name: "Show the exceptions to line 1" }).click();
  await p.waitForTimeout(800);
  // (A new table in the stand-in cannot hide rows; which rows show is checked in the shell test and the real app.)
  ok(/Showing the 2 rows that break it/.test(await text()) && (await p.evaluate(() => window.__nde.tab.info.filtered)) && (await p.getByRole("dialog").count()) === 0, "the exceptions filter is applied and the screen closes");
  await run("data.clearFilter");
  await run("data.relations");
  await dlg.getByRole("button", { name: "Select all", exact: true }).click();
  await find.click();
  await p.waitForTimeout(600);
  await p.evaluate(async () => void (await window.__nde.backend.setCells(window.__nde.tab.id, [[1, 2, "changed"]])));
  await dlg.getByRole("button", { name: "Show the exceptions to line 1" }).click();
  await p.waitForTimeout(600);
  ok((await p.getByRole("dialog").count()) === 1 && /scan again/.test(await text()) && (await dlg.locator("ol.found").count()) === 0, "a changed table is refused and the old lines go");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);
  ok((await p.getByRole("dialog").count()) === 0, "Escape closes it");
  await p.keyboard.press("Alt+q");
  await p.waitForTimeout(300);
  await p.keyboard.type("find relationships");
  await p.waitForTimeout(500);
  ok(/Find relationships/.test(await text()), "command search finds it");
  ok(errors.length === 0, `no page errors (${errors.slice(0, 3).join("; ")})`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
