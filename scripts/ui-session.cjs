// UI test of the start-up offer (0.4.7): files open already, such as one
// double-clicked in Windows, are not offered back, and after Reopen that
// file stays in front, even when its open finishes mid-reopen.
// Run: (cd ui && npx vite --port 5208) then
// NODE_PATH=<folder with playwright> node scripts/ui-session.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const url = process.argv[3] || "http://localhost:5208";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const p = await (await b.newContext({ viewport: { width: 1360, height: 860 } })).newPage();
  const errors = [];
  p.on("pageerror", (e) => errors.push(String(e)));
  let failed = 0;
  const ok = (cond, msg) => {
    if (!cond) failed++;
    console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  };
  const wait = (ms) => p.waitForTimeout(ms);
  const V = "C:/Data/voters.csv";
  const P = "C:/Data/precincts.csv";
  const T = "C:/Data/towns.csv";
  const front = () => p.evaluate(() => window.__nde.tab?.info.path ?? null);
  const paths = () => p.evaluate(() => window.__nde.tabs.map((t) => t.info.path));
  const fresh = async () => {
    await p.goto(url);
    await wait(600);
    await p.evaluate(() => window.__nde.resetStart());
  };
  const offer = async () => {
    await p.evaluate(() => void window.__nde.startSession());
    const box = p.locator(".dialog[role=alertdialog]");
    await box.waitFor({ timeout: 3000 }).catch(() => {});
    return box;
  };

  // 1. Double-clicked file open first: only the others are offered.
  await fresh();
  await p.evaluate(([v, pp, t]) => {
    const be = window.__nde.backend;
    be.seedSession([v, pp, t], 1);
    be.osOpen(v);
  }, [V, P, T]);
  await wait(400);
  ok((await front()) === V, "the double-clicked file is open");
  let box = await offer();
  const text = (await box.textContent()) ?? "";
  ok(/2 files/.test(text) && text.includes("precincts.csv") && text.includes("towns.csv"), "the offer lists the two others");
  ok(!text.includes("voters.csv"), "not the file already open");
  await box.getByRole("button", { name: "Reopen" }).click();
  await wait(1500);
  const all = await paths();
  ok(all.length === 3 && new Set(all).size === 3, `three tabs, no copy (${all.length})`);
  ok((await front()) === V, "after Reopen the double-clicked file is in front");

  // 2. The double-clicked file finishes opening in the middle of the reopen.
  await fresh();
  await p.evaluate(([v, pp, t]) => window.__nde.backend.seedSession([pp, t], 0, { n: 1, path: v }), [V, P, T]);
  box = await offer();
  await box.getByRole("button", { name: "Reopen" }).click();
  await wait(1500);
  ok((await paths()).length === 3, "all three open");
  ok((await front()) === V, "a file arriving mid-reopen stays in front");

  // 3. No file from Windows: last time's front tab is in front.
  await fresh();
  await p.evaluate(([pp, t]) => window.__nde.backend.seedSession([pp, t], 1), [P, T]);
  box = await offer();
  await box.getByRole("button", { name: "Reopen" }).click();
  await wait(1500);
  ok((await front()) === T, "without one, last time's front tab is in front");

  // 4. The only remembered file is the one open: nothing is asked.
  await fresh();
  await p.evaluate((v) => {
    const be = window.__nde.backend;
    be.seedSession([v], 0);
    be.osOpen(v);
  }, V);
  await wait(400);
  await p.evaluate(() => void window.__nde.startSession());
  await wait(800);
  ok((await p.locator(".dialog[role=alertdialog]").count()) === 0, "nothing to offer: no question");

  if (process.argv[2]) await p.screenshot({ path: process.argv[2] });
  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
