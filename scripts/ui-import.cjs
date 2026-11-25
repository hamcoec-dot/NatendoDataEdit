// UI test of the "numbers stay exactly as written" import screen in the browser stand-in: the line under the
// options (counts and column names only), the one zero-padding checkbox (its cell count and column), the line
// changing when it is ticked, the answer remembered for the file (a salted fingerprint and yes/no only), and the
// "Show what the last import kept exactly" command in the command search.
// Run: (cd ui && npx vite --port 5231) then
// NODE_PATH=<folder with playwright> node scripts/ui-import.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5231";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
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
  const open = async () => {
    await p.evaluate(() => window.__nde.reg.run("file.import"));
    await p.locator(".shell[role=dialog]").waitFor({ timeout: 5000 });
    await p.locator("[data-testid=import-notes]").waitFor({ timeout: 5000 });
    await p.waitForTimeout(400);
  };
  await open();
  const note = async () => ((await p.locator("[data-testid=import-notes]").textContent()) || "").replace(/\s+/g, " ").trim();
  const box = p.getByLabel(/Write numbers with leading zeros/);
  ok((await box.count()) === 1, "one zero-padding question on the import screen");
  ok(!(await box.isChecked()), "it starts unticked");
  const label = ((await box.locator("xpath=..").textContent()) || "").replace(/\s+/g, " ");
  ok(/3 cells in “zip”/.test(label), `the question gives the cell count and the column name (${label.trim()})`);
  ok(/kept exactly/.test(await note()), "the line says long numbers were kept exactly");
  ok(!/written with leading zeros/.test(await note()), "nothing is said about padding before it is asked for");
  if (out) await p.screenshot({ path: path.join(out, "import-exact.png") });
  await box.click();
  await p.waitForTimeout(500);
  ok(/3 numbers written with leading zeros/.test(await note()), "ticking it changes the line to say how many were padded");
  await p.getByRole("button", { name: "Import", exact: true }).click();
  await p.waitForTimeout(600);
  const st = ((await p.textContent("footer.status")) || "").replace(/\s+/g, " ");
  ok(/Imported/.test(st) && /kept exactly/.test(st), "the status bar carries the line after the import");
  // remembered for the file
  const kept = await p.evaluate(() => JSON.stringify(Object.entries(localStorage).filter(([k]) => k.startsWith("nde.padzeros"))));
  ok(/nde\.padzeros/.test(kept) && !/people|xlsx|zip/i.test(kept), `only a fingerprint and yes/no are kept (${kept.slice(0, 90)})`);
  await open();
  ok(await box.isChecked(), "the answer is remembered: the box is ticked the next time");
  await p.getByRole("button", { name: "Cancel", exact: true }).click();
  // command search
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("last import kept");
  await p.waitForTimeout(300);
  const hit = await p.getByText(/Show what the last import kept exactly/).count();
  ok(hit >= 1, "the command is found in command search");
  await p.keyboard.press("Escape");
  ok(errors.length === 0, `no page errors ${errors.join(" | ").slice(0, 200)}`);
  await b.close();
  process.exit(failed ? 1 : 0);
})();
