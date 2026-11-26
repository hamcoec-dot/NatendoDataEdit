// UI test of Data > Compare lists in the browser stand-in: key columns start unticked, the slice counts, opening a
// slice and the keys, a change making the result stale or refused, adding and removing lists, copy, and command search.
// (The stand-in compares trimmed, small-letter texts; the real rules are the engine's tests.)
// Run: (cd ui && npx vite --port 5363) then NODE_PATH=<folder with playwright> node scripts/ui-sets.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5363";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1360, height: 900 }, permissions: ["clipboard-read", "clipboard-write"] });
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
  // Three tabs of made-up voter numbers.
  const make = async (rows) => {
    await p.keyboard.press("Control+n");
    await p.waitForTimeout(600);
    await p.evaluate(async (rows) => {
      const t = window.__nde.tab;
      t.update(await window.__nde.backend.change(t.id, { op: "insert_rows", at: 1, count: rows.length - 1 }), true);
      t.update(await window.__nde.backend.setCells(t.id, rows.flatMap((line, r) => line.map((v, c) => [r, c, v]))), true);
      t.header = true;
      t.headerTouched = true;
      t.bump();
    }, rows);
    await p.waitForTimeout(300);
    return p.evaluate(() => window.__nde.tab.id);
  };
  const a = await make([["Voter", "Name"], ["V1", "Ada"], ["V2", "Grace"], ["V3", "Alan"], ["V4", "Mary"], ["", "Nobody"]]);
  const bb = await make([["Voter"], ["v2 "], ["V3"], ["V9"]]);
  const c = await make([["Voter"], ["V3"], ["V4"]]);

  await run("data.compareLists");
  const dlg = p.getByRole("dialog", { name: "Compare lists" });
  await dlg.waitFor({ timeout: 5000 });
  ok((await dlg.locator(".cols input:checked").count()) === 0, "key columns start unticked");
  ok(await dlg.getByRole("button", { name: "Compare", exact: true }).isDisabled(), "Compare waits for the keys");
  await dlg.getByRole("button", { name: "Add a list" }).click();
  for (const [i, id] of [a, bb, c].entries()) await dlg.getByLabel(`Tab of list ${"ABC"[i]}`).selectOption(String(id));
  for (const i of [0, 1, 2]) await dlg.locator("fieldset").nth(i).getByRole("checkbox", { name: "Voter" }).check();
  await dlg.getByRole("button", { name: "Compare", exact: true }).click();
  await p.waitForTimeout(800);
  const rowsOf = () => dlg.locator("table[aria-label=Slices] tbody tr").evaluateAll((all) => all.map((tr) => [...tr.querySelectorAll("th, td:not(.open)")].map((x) => x.textContent.trim())));
  let slices = await rowsOf();
  ok(slices.length === 5, `empty slices are hidden (${slices.length} shown)`);
  const t = await text();
  ok(/5 rows, 4 different keys, 1 with a blank key/.test(t), "totals with the blank key apart");
  ok(slices.some((r) => / only$/.test(r[0]) && r[1] === "1") && slices.some((r) => / and .*, not /.test(r[0])), `slices named (${slices.map((r) => r[0]).join(" / ")})`);
  if (out) await p.screenshot({ path: path.join(out, "compare-lists.png") });
  await dlg.getByLabel("Show empty slices").check();
  ok((await rowsOf()).length === 7, "all seven slices when asked");
  await dlg.getByRole("button", { name: "Copy counts" }).click();
  await p.waitForTimeout(300);
  const clip = await p.evaluate(() => navigator.clipboard.readText());
  ok(/^Slice\tDifferent keys\tRows in /.test(clip) && clip.split("\n").length >= 11, "the counts copy as a table");
  await dlg.getByLabel("Trim spaces").uncheck();
  ok(/press Compare again/.test(await text()) && (await dlg.getByRole("button", { name: "Copy counts" }).isDisabled()), "a changed rule makes the result stale");
  await dlg.getByLabel("Trim spaces").check();
  // The slice in all three: V3.
  const all = dlg.locator("table[aria-label=Slices] tbody tr").nth(6);
  await all.getByRole("button", { name: /^Open / }).click();
  await p.waitForTimeout(900);
  const opened = await p.evaluate(async () => {
    const t = window.__nde.tab;
    const cell = async (r, c) => (await window.__nde.backend.cellText(t.id, r, c)).text;
    return { rows: t.info.rows, header: t.header, first: await cell(1, 0), name: await cell(1, 1) };
  });
  ok(opened.rows === 2 && opened.header && opened.first === "V3" && opened.name === "Alan", `a slice opens as a tab with all columns (${JSON.stringify(opened)})`);
  ok((await p.getByRole("dialog").count()) === 0, "and the screen closes");

  await run("data.compareLists");
  await dlg.waitFor({ timeout: 5000 });
  await dlg.getByLabel("Tab of list A").selectOption(String(a));
  await dlg.getByLabel("Tab of list B").selectOption(String(bb));
  for (const i of [0, 1]) await dlg.locator("fieldset").nth(i).getByRole("checkbox", { name: "Voter" }).check();
  await dlg.getByRole("button", { name: "Compare", exact: true }).click();
  await p.waitForTimeout(600);
  // A list changes under the screen (another window, the agent).
  await p.evaluate(async (id) => void (await window.__nde.backend.setCells(id, [[1, 0, "V77"]])), bb);
  await dlg.locator("table[aria-label=Slices] tbody tr").first().getByRole("button", { name: /^Open / }).click();
  await p.waitForTimeout(700);
  ok((await p.getByRole("dialog").count()) === 1 && /run the comparison again/.test(await text()), "a changed list is refused and the screen stays");
  ok((await dlg.locator("table[aria-label=Slices]").count()) === 0, "the old counts are gone");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);
  ok((await p.getByRole("dialog").count()) === 0, "Escape closes it");
  await p.keyboard.press("Alt+q");
  await p.waitForTimeout(300);
  await p.keyboard.type("compare lists");
  await p.waitForTimeout(500);
  ok(/Compare lists/.test(await text()), "command search finds it");
  ok(errors.length === 0, `no page errors (${errors.slice(0, 3).join("; ")})`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
