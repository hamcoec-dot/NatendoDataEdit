// UI test of File > Open recent against the vite dev server (browser stub).
// Run: (cd ui && npx vite --port 5192) then
// NODE_PATH=<folder with playwright> node scripts/ui-recent.cjs <screenshot dir> [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5192";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const p = await b.newPage({ viewport: { width: 1360, height: 860 } });
  const errors = [];
  p.on("pageerror", (e) => errors.push(String(e)));
  await p.goto(url);
  await p.waitForTimeout(1000);
  let failed = 0;
  const ok = (cond, msg) => {
    if (!cond) failed++;
    console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  };
  const tabs = () => p.$$eval(".tabs .tab", (e) => e.length);
  const dialog = "[role=dialog]:has(h2:has-text('Open recent'))";

  // From the File menu.
  await p.click("button:has-text('File')");
  ok(await p.isVisible("[role=menuitem]:has-text('Open recent')"), "File menu has Open recent");
  await p.click("[role=menuitem]:has-text('Open recent')");
  await p.waitForTimeout(300);
  ok(await p.isVisible(dialog), "the recent files list opens");
  ok(await p.isDisabled(`${dialog} button.primary`), "Open waits for a tick");
  await p.click(`${dialog} input[type=checkbox]`);
  await p.screenshot({ path: path.join(out, "open-recent.png") });
  await p.click(`${dialog} button.primary`);
  await p.waitForTimeout(3000);
  ok((await tabs()) === 1 && !(await p.isVisible(dialog)), "the ticked file opens in a tab");

  // Ctrl+Shift+O with a file open: it shows as open, and is not opened twice.
  await p.keyboard.press("Control+Shift+O");
  await p.waitForTimeout(300);
  ok(await p.isVisible(`${dialog} .open`), "a file already open is marked");
  await p.dblclick(`${dialog} label`);
  await p.waitForTimeout(800);
  ok((await tabs()) === 1, "double-clicking an open file switches to it, no second tab");

  // Alt+Q finds it.
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("recent files");
  await p.waitForTimeout(200);
  const top = await p.$$eval("#command-results [role=option]", (e) => e.slice(0, 3).map((x) => x.textContent.trim()));
  ok(top.some((t) => /Open recent/.test(t)), `Alt+Q "recent files" finds it (${top[0]})`);
  await p.keyboard.press("Escape");

  ok(errors.length === 0, `no page errors ${errors.join("; ")}`);
  await b.close();
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  process.exit(failed ? 1 : 0);
})();
