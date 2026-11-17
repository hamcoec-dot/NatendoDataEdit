// UI test of dragging file tabs against the vite dev server (browser stub).
// Run: (cd ui && npx vite --port 5193) then
// NODE_PATH=<folder with playwright> node scripts/ui-tabdrag.cjs <screenshot dir> [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5193";
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
  const names = () => p.$$eval(".tabs .tab", (e) => e.map((x) => x.querySelector("span").textContent.trim() + (x.querySelector(".tab-folder")?.textContent ?? "")));
  const activeName = () => p.$eval(".tabs .tab.active", (x) => x.querySelector("span").textContent.trim() + (x.querySelector(".tab-folder")?.textContent ?? ""));

  // Four tabs from the stub's recent files.
  await p.keyboard.press("Control+Shift+O");
  await p.waitForTimeout(300);
  for (const n of [1, 2, 3, 4]) {
    await p.click(`.recent-list li:nth-child(${n}) label .name`, { button: "middle" });
    await p.waitForTimeout(600);
  }
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);
  const before = await names();
  ok(before.length === 4, `four tabs (${before.join(", ")})`);

  const box = async (i) => (await p.$$(".tabs .tab"))[i].boundingBox();
  const b0 = await box(0);
  const b2 = await box(2);
  // Drag the first tab past the middle of the third.
  await p.mouse.move(b0.x + 30, b0.y + b0.height / 2);
  await p.mouse.down();
  await p.mouse.move(b0.x + 60, b0.y + b0.height / 2, { steps: 4 });
  await p.mouse.move(b2.x + b2.width * 0.75, b0.y + b0.height / 2, { steps: 10 });
  await p.waitForTimeout(100);
  await p.waitForTimeout(200);
  const s1 = await box(1);
  const s2 = await box(2);
  const s3 = await box(3);
  const b1 = { x: b0.x + b0.width + 2 };
  ok(Math.abs(s1.x - b0.x) < 4 && Math.abs(s2.x - (b0.x + s1.width + 2)) < 4, `the tabs it passed slid left, opening a gap (${Math.round(s1.x)}, ${Math.round(s2.x)})`);
  ok(Math.abs(s3.x - (await p.$$eval(".tabs .tab", (e) => e[3].offsetLeft)) - (await p.$eval(".tabs", (e) => e.getBoundingClientRect().left))) < 4, "the tab after the gap stays put");
  void b1;
  const dragged = await box(0);
  ok(dragged.x > b0.x + 40, "the dragged tab follows the pointer");
  ok((await names()).join() === before.join(), "the other tabs stay put until letting go");
  ok((await p.evaluate(() => String(window.getSelection()))) === "", "no text is selected while dragging");
  await p.screenshot({ path: path.join(out, "drag-tab.png"), clip: { x: 0, y: 0, width: 1360, height: 220 } });
  await p.mouse.up();
  await p.waitForTimeout(300);
  const after = await names();
  ok(after.join() === [before[1], before[2], before[0], before[3]].join(), `letting go puts it there (${after.join(", ")})`);
  ok((await activeName()) === before[0], "the tab put down is the one shown");
  ok((await p.$$eval(".tabs .tab", (e) => e.filter((x) => x.style.transform).length)) === 0, "no tab is left slid aside");

  // Esc during a drag: nothing moves.
  const c0 = await box(0);
  const c3 = await box(3);
  await p.mouse.move(c0.x + 30, c0.y + c0.height / 2);
  await p.mouse.down();
  await p.mouse.move(c3.x + c3.width * 0.8, c0.y + c0.height / 2, { steps: 10 });
  await p.keyboard.press("Escape");
  await p.mouse.up();
  await p.waitForTimeout(300);
  ok((await names()).join() === after.join(), "Esc during a drag leaves the tabs where they were");

  // Dragging back to the start.
  const d2 = await box(2);
  const d0 = await box(0);
  await p.mouse.move(d2.x + 30, d2.y + d2.height / 2);
  await p.mouse.down();
  await p.mouse.move(d0.x + 5, d0.y + d0.height / 2, { steps: 10 });
  await p.mouse.up();
  await p.waitForTimeout(300);
  ok((await names()).join() === before.join(), `dragging to the far left puts it first (${(await names()).join(", ")})`);

  // A plain click still picks a tab.
  await (await p.$$(".tabs .tab"))[3].click();
  await p.waitForTimeout(200);
  ok((await activeName()) === before[3], "a click without dragging still picks the tab");

  // A click on a tab closes an open menu (the press is not swallowed).
  await p.click("button:has-text('File')");
  await p.waitForTimeout(200);
  ok(await p.isVisible("[role=menuitem]:has-text('Open recent')"), "File menu open");
  // (The last tab: the menu covers the first ones.)
  const m3 = await box(3);
  await p.mouse.click(m3.x + 20, m3.y + m3.height / 2);
  await p.waitForTimeout(200);
  ok(!(await p.isVisible("[role=menuitem]:has-text('Open recent')")), "clicking a tab closes the menu");
  ok((await activeName()) === before[3], "and shows that tab");
  await (await p.$$(".tabs .tab"))[1].click();
  await p.waitForTimeout(200);

  // Keyboard: Ctrl+Alt+Shift+PageDown / PageUp move the shown tab.
  await p.keyboard.press("Control+Alt+Shift+PageDown");
  await p.waitForTimeout(200);
  ok((await names()).join() === [before[0], before[2], before[1], before[3]].join(), "Ctrl+Alt+Shift+PageDown moves the tab right");
  await p.keyboard.press("Control+Alt+Shift+PageUp");
  await p.waitForTimeout(200);
  ok((await names()).join() === before.join(), "Ctrl+Alt+Shift+PageUp moves it back");

  // A file opening during a drag ends the drag; nothing moves wrongly.
  const e0 = await box(0);
  const e3 = await box(3);
  await p.mouse.move(e0.x + 30, e0.y + e0.height / 2);
  await p.mouse.down();
  await p.mouse.move(e3.x + e3.width * 0.8, e0.y + e0.height / 2, { steps: 10 });
  await p.keyboard.press("Control+n");
  await p.waitForTimeout(400);
  await p.mouse.up();
  await p.waitForTimeout(300);
  const withNew = await names();
  ok(withNew.length === 5 && withNew.slice(0, 4).join() === before.join(), `a file opening mid-drag leaves the others in place (${withNew.join(", ")})`);

  ok(errors.length === 0, `no page errors ${errors.join("; ")}`);
  await b.close();
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  process.exit(failed ? 1 : 0);
})();
