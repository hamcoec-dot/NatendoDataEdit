// UI test of v0.4.6: a file opened again keeps its "first row is header"
// choice, and the app keeps only a fingerprint of the path.
// Run: (cd ui && npx vite --port 5206) then
// NODE_PATH=<folder with playwright> node scripts/ui-header-memory.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const url = process.argv[3] || "http://localhost:5206";
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
  const command = async (q) => {
    await p.keyboard.press("Alt+q");
    await p.keyboard.type(q);
    await wait(300);
    await p.keyboard.press("Enter");
    await wait(500);
  };
  const header = () => p.evaluate(() => window.__nde.tab.header);
  const open = async () => {
    await p.keyboard.press("Control+o");
    await wait(1200);
  };

  await p.goto(url);
  await wait(500);
  await open();
  const guessed = await header();
  ok(typeof guessed === "boolean", `opened with a guess (${guessed})`);
  await command("First row is header");
  ok((await header()) === !guessed, "the choice is flipped");
  await p.keyboard.press("Control+w");
  await wait(600);
  await open();
  ok((await header()) === !guessed, "opened again, the file keeps the choice");
  // Flip back, save, close, open: the saved choice is kept.
  await command("First row is header");
  await p.keyboard.press("Control+s");
  await wait(600);
  await p.keyboard.press("Control+w");
  await wait(600);
  await open();
  ok((await header()) === guessed, "the choice at the last save is kept");
  const kept = await p.evaluate(() => localStorage.getItem("nde.headers.v1") ?? "");
  ok(kept.length > 0 && !/sample|samples|csv/i.test(kept), `only a fingerprint is kept (${kept.slice(0, 60)})`);

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
