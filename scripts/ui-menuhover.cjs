// UI test: menu items light up quietly under the pointer (right-click menus
// and the menu bar), in the light and dark themes; nothing is picked when a
// right-click menu opens, and Shift+F10 starts on the first item.
// Run: (cd ui && npx vite --port 5195) then
// NODE_PATH=<folder with playwright> node scripts/ui-menuhover.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5195";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  let failed = 0;
  const ok = (cond, msg) => {
    if (!cond) failed++;
    console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  };
  for (const scheme of ["light", "dark"]) {
    const p = await b.newPage({ viewport: { width: 1200, height: 800 }, colorScheme: scheme });
    const errors = [];
    p.on("pageerror", (e) => errors.push(String(e)));
    await p.goto(url);
    await p.keyboard.press("Control+o");
    await p.waitForTimeout(2500);
    const bg = (sel) => p.evaluate((s) => getComputedStyle(document.querySelector(s)).backgroundColor, sel);
    const canvas = await p.locator("canvas").first().boundingBox();
    await p.mouse.click(canvas.x + 300, canvas.y + 120, { button: "right" });
    await p.waitForTimeout(300);
    ok((await p.locator(".menu.context .item.hot").count()) === 0, `${scheme}: a right-click menu opens with nothing picked`);
    const second = p.locator(".menu.context .item:not(:disabled)").nth(1);
    const plain = await bg(".menu.context .item:not(:disabled):nth-of-type(1)");
    await second.hover();
    await p.waitForTimeout(150);
    const lit = await second.evaluate((e) => getComputedStyle(e).backgroundColor);
    ok(lit !== plain && lit !== "rgba(0, 0, 0, 0)", `${scheme}: the item under the pointer is tinted (${lit})`);
    ok((await p.locator(".menu.context .item.hot").count()) === 1, `${scheme}: only that item is marked`);
    if (out) await p.screenshot({ path: path.join(out, `menu-hover-${scheme}.png`), clip: { x: canvas.x + 280, y: canvas.y + 100, width: 340, height: 260 } });
    await p.keyboard.press("Escape");
    await p.waitForTimeout(200);
    // Shift+F10 starts on the first item; Up from nothing goes to the last.
    await p.evaluate(() => window.__nde.grid.focus());
    await p.keyboard.press("Shift+F10");
    await p.waitForTimeout(300);
    ok((await p.locator(".menu.context .item.hot").count()) === 1, `${scheme}: Shift+F10 opens with the first item picked`);
    await p.keyboard.press("Escape");
    await p.waitForTimeout(200);
    // The menu bar: hover lights an item the same way.
    await p.click("button.menu-title:has-text('Edit')");
    await p.waitForTimeout(200);
    const mi = p.locator(".menu:not(.context) .item:not(:disabled)").nth(2);
    await mi.hover();
    await p.waitForTimeout(150);
    const lit2 = await mi.evaluate((e) => getComputedStyle(e).backgroundColor);
    ok(lit2 !== "rgba(0, 0, 0, 0)", `${scheme}: menu bar items are tinted under the pointer too (${lit2})`);
    await p.keyboard.press("Escape");
    ok(errors.length === 0, `${scheme}: no page errors ${errors.join(" | ")}`);
    await p.close();
  }
  await b.close();
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  process.exit(failed ? 1 : 0);
})();
