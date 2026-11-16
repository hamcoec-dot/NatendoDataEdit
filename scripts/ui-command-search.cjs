// UI test of command search (Alt+Q) against the vite dev server (browser stub backend).
// Run: (cd ui && npx vite) then NODE_PATH=<folder with playwright> node scripts/ui-command-search.cjs <screenshot dir>
// Uses the installed Edge, so no browser download is needed.
const { chromium } = require("playwright");
const out = process.argv[2];
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const p = await b.newPage({ viewport: { width: 1100, height: 720 } });
  const errors = [];
  p.on("pageerror", (e) => errors.push(String(e)));
  await p.goto(process.argv[3] || "http://localhost:5173");
  await p.waitForTimeout(1500);
  const ok = (cond, msg) => console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  const names = () => p.$$eval("#command-results button .name", (els) => els.map((e) => e.textContent));

  // 1. Hotkey opens, typing filters instantly.
  await p.keyboard.press("Alt+q");
  ok(await p.isVisible("[aria-label='Search for a command']"), "Alt+Q opens the pop-up");
  ok(await p.evaluate(() => document.activeElement?.getAttribute("aria-label") === "Command"), "typing box has focus");
  for (const ch of "dedupe") await p.keyboard.type(ch);
  let n = await names();
  ok(n.some((x) => /duplicates/i.test(x)), `dedupe -> ${n.slice(0, 3).join(" | ")}`);
  const desc = await p.textContent("#command-results li:first-child .desc");
  ok(/Data menu/.test(desc ?? "") && (desc ?? "").length > 20, `description line: ${desc}`);
  await p.screenshot({ path: `${out}/launcher-dedupe.png` });

  // 2. Half-typed phrase keeps results.
  await p.fill("input[aria-label=Command]", "");
  for (const ch of "dark mo") {
    await p.keyboard.type(ch);
    const c = (await names()).length;
    if (!c) ok(false, `no results while typing at "${await p.inputValue("input[aria-label=Command]")}"`);
  }
  n = await names();
  ok(n[0] === "Theme: dark", `dark mo -> ${n[0]}`);

  // 3. Enter runs it and closes.
  await p.keyboard.press("Enter");
  await p.waitForTimeout(200);
  ok(!(await p.isVisible("[aria-label='Search for a command']")), "Enter closes the pop-up");
  ok(await p.evaluate(() => document.documentElement.dataset.theme === "dark"), "theme command ran (data-theme=dark)");

  // 4. Arrows move the selection, Esc closes.
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("theme");
  await p.keyboard.press("ArrowDown");
  const sel = await p.$eval("[aria-selected=true] .name", (e) => e.textContent);
  ok(!!sel, `ArrowDown selects: ${sel}`);
  await p.keyboard.press("Enter");
  await p.waitForTimeout(200);
  await p.keyboard.press("Alt+q");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(100);
  ok(!(await p.isVisible("[aria-label='Search for a command']")), "Esc closes");

  // 4b. Enter on a greyed-out result says why in the status bar.
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("remove duplicates");
  await p.keyboard.press("Enter");
  await p.waitForTimeout(200);
  const status = await p.evaluate(() => document.body.innerText);
  ok(/needs a file: open one first/.test(status), "greyed result explains itself");
  await p.keyboard.press("Escape");

  // 4c. Review regressions: top results.
  for (const [q, want] of [["trim", "Add or remove spaces"], ["insert row", "Rows above"], ["add column", "Columns left"], ["text to columns", "Split column (text to columns)"]]) {
    await p.keyboard.press("Alt+q");
    await p.keyboard.type(q);
    const n2 = await names();
    ok(n2.slice(0, 2).includes(want), `${q} -> ${n2.slice(0, 2).join(" | ")}`);
    await p.keyboard.press("Escape");
  }

  // 5. Click runs.
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("keyboard shortcuts");
  await p.click("#command-results li:first-child button");
  await p.waitForTimeout(300);
  ok(!(await p.isVisible("[aria-label='Search for a command']")), "click runs and closes");
  ok(await p.isVisible("[role=dialog]"), "keyboard shortcuts dialog opened by click");
  await p.keyboard.press("Escape");

  // 6. Speed: a key press updates results quickly.
  await p.keyboard.press("Alt+q");
  const t = await p.evaluate(async () => {
    const i = document.querySelector("input[aria-label=Command]");
    const t0 = performance.now();
    for (const q of ["r", "re", "rem", "remo", "remove", "remove b", "remove bl", "remove bla", "remove blank"]) {
      i.value = q;
      i.dispatchEvent(new Event("input", { bubbles: true }));
      await new Promise((r) => requestAnimationFrame(r));
    }
    return (performance.now() - t0) / 9;
  });
  ok(t < 50, `average ${t.toFixed(1)} ms per key press (incl. a frame)`);
  await p.screenshot({ path: `${out}/launcher-remove-blank.png` });

  // 7. Alt+Q again closes it (a toggle), also when focus left the typing box.
  await p.keyboard.press("Alt+q");
  await p.waitForTimeout(200);
  ok(!(await p.isVisible("[aria-label='Search for a command']")), "Alt+Q again closes the pop-up");
  await p.keyboard.press("Alt+q");
  await p.waitForTimeout(200);
  ok(await p.isVisible("[aria-label='Search for a command']"), "Alt+Q opens it again");
  await p.evaluate(() => document.activeElement?.blur());
  await p.keyboard.press("Alt+q");
  await p.waitForTimeout(200);
  ok(!(await p.isVisible("[aria-label='Search for a command']")), "Alt+Q closes it with focus outside the box");
  ok(errors.length === 0, `no page errors ${errors.join("; ")}`);
  await b.close();
})();
