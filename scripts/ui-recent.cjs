// UI test of File > Open recent against the vite dev server (browser stub).
// The stub's recent list: sample-1M.csv, 2025\Data\voters.csv,
// 2024\Data\voters.csv (same name, same folder name) and C:\precincts.csv
// (at the top of a drive).
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
  const item = (n) => `${dialog} li:nth-child(${n}) label`;
  const marked = (n) => p.isVisible(`${item(n)} .open`);
  const closeAll = async () => {
    for (let i = await tabs(); i > 0; i--) {
      await p.keyboard.press("Control+w");
      await p.waitForTimeout(300);
    }
  };

  // Start page: a middle-click opens the file and brings up the list.
  await p.click(".empty .recent li:nth-child(4) button", { button: "middle" });
  await p.waitForTimeout(1500);
  ok((await tabs()) === 1 && (await p.isVisible(dialog)), "start page middle-click opens a tab and shows the recent list");
  ok(await marked(4), "a file at the top of a drive is marked open");

  // Middle-clicks in the list: one new tab each, the list stays open.
  await p.click(item(3), { button: "middle" });
  await p.waitForTimeout(800);
  ok((await tabs()) === 2 && (await p.isVisible(dialog)), "middle-click opens 2024\\Data\\voters.csv and keeps the list open");
  ok((await marked(3)) && !(await marked(2)), "only the 2024 file is marked open, not 2025's file of the same name");
  await p.click(item(2), { button: "middle" });
  await p.waitForTimeout(800);
  ok((await tabs()) === 3, "middle-click opens 2025\\Data\\voters.csv too (not mistaken for the open 2024 one)");
  await p.click(item(3), { button: "middle" });
  await p.click(item(3), { button: "middle" });
  await p.waitForTimeout(800);
  ok((await tabs()) === 3, "middle-clicking an open file again does not open it twice");
  await p.screenshot({ path: path.join(out, "open-recent.png") });
  await p.keyboard.press("Escape");
  await p.waitForTimeout(200);
  ok(!(await p.isVisible(dialog)), "Esc closes the list after middle-clicks");
  await closeAll();

  // From the File menu, by keyboard: Down moves, Enter opens the focused file.
  await p.click("button:has-text('File')");
  ok(await p.isVisible("[role=menuitem]:has-text('Open recent')"), "File menu has Open recent");
  await p.click("[role=menuitem]:has-text('Open recent')");
  await p.waitForTimeout(300);
  ok(await p.isVisible(dialog), "the recent files list opens");
  ok(await p.isDisabled(`${dialog} button.primary`), "Open waits for a tick");
  await p.focus(`${item(1)} input`);
  await p.keyboard.press("ArrowDown");
  await p.keyboard.press("ArrowDown");
  ok(await p.$eval(`${item(3)} input`, (e) => e === document.activeElement), "Down moves to the next file");
  await p.keyboard.press("End");
  ok(await p.$eval(`${item(4)} input`, (e) => e === document.activeElement), "End moves to the last file");
  await p.keyboard.press("Home");
  await p.keyboard.press("ArrowDown");
  await p.keyboard.press("Enter");
  await p.waitForTimeout(800);
  ok((await tabs()) === 1 && !(await p.isVisible(dialog)), "Enter with nothing ticked opens the focused file");

  // Ticks: two files, one already open.
  await p.keyboard.press("Control+Shift+O");
  await p.waitForTimeout(300);
  ok(await marked(2), "the file opened by Enter is marked open");
  await p.click(`${item(2)} input`);
  await p.click(`${item(1)} input`);
  await p.click(`${dialog} button.primary`);
  await p.waitForTimeout(3000);
  ok((await tabs()) === 2 && !(await p.isVisible(dialog)), "ticked files open, the open one is not opened twice");

  // Double-click an open file: switches to it with a note.
  await p.keyboard.press("Control+Shift+O");
  await p.waitForTimeout(300);
  await p.dblclick(item(2));
  await p.waitForTimeout(800);
  ok((await tabs()) === 2, "double-clicking an open file switches to it, no second tab");
  ok(/already open/.test(await p.textContent("body")), "it says the file was already open");

  // Help > Mouse lists the middle-click.
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("mouse");
  await p.waitForTimeout(200);
  await p.keyboard.press("Enter");
  await p.waitForTimeout(300);
  ok(/Middle-click them in the recent files list/.test(await p.textContent("body")), "Help > Mouse lists middle-click on recent files");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(200);

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
