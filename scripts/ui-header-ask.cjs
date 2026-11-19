// UI test of v0.4.7: a file opened with no remembered choice asks, in a
// bar, whether its first row holds the column names; the answer is kept,
// the table doesn't move when it is answered, and it asks only once.
// Run: (cd ui && npx vite --port 5207) then
// NODE_PATH=<folder with playwright> node scripts/ui-header-ask.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5207";
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
  const header = () => p.evaluate(() => window.__nde.tab.header);
  const open = async () => {
    await p.keyboard.press("Control+o");
    await p.waitForFunction(() => window.__nde?.tab?.info?.indexed, null, { timeout: 5000 }).catch(() => {});
    await wait(700);
  };
  const close = async () => {
    await p.keyboard.press("Control+w");
    await wait(500);
  };
  const command = async (q) => {
    await p.keyboard.press("Alt+q");
    await p.keyboard.type(q);
    await wait(300);
    await p.keyboard.press("Enter");
    await wait(500);
  };
  // Private mode on asks about copies kept from before: keep them.
  const keepCopies = async () => {
    const d = p.locator(".dialog[role=alertdialog]");
    if (await d.count()) await d.getByRole("button", { name: "Keep them" }).click();
    await wait(300);
  };
  const forgetAll = () => p.evaluate(() => localStorage.removeItem("nde.headers.v1"));
  const question = p.locator(".notices .notice:has(button:has-text('Yes, column names'))");
  const answered = p.locator(".notices .notice:has-text('remembered for this file')");
  const gridTop = async () => (await (await p.$("canvas")).boundingBox()).y;

  await p.goto(url);
  await p.waitForFunction(() => window.__nde, null, { timeout: 5000 });
  await open();
  ok(await question.isVisible(), "a new file asks about its first row");
  ok(/looks like column names, so it is used as titles/.test(await question.textContent()) && (await header()) === true, "with the guess applied");
  if (out) await question.screenshot({ path: path.join(out, "header-ask.png") });
  ok((await question.locator("button[aria-keyshortcuts='Alt+Shift+N']").count()) === 1, "the buttons name their keys");

  // 1. Answered from the keyboard: the line stays, so the table doesn't move.
  let top = await gridTop();
  await p.keyboard.press("Alt+Shift+N");
  await wait(400);
  ok((await header()) === false && !(await question.count()), "Alt+Shift+N: the first row is data");
  ok(/read as data; remembered for this file/.test((await answered.textContent()) ?? ""), "the line now says what was kept");
  ok((await gridTop()) === top, "the table did not move");
  ok(/remembered for this file/.test(await p.locator("footer").textContent()), "the status bar says the answer is kept");
  await close();
  await open();
  ok(!(await question.count()) && !(await answered.count()) && (await header()) === false, "opened again: no question, the answer kept");

  // 2. Ignored, then closed: the guess applied is kept, so it asks once.
  await close();
  await forgetAll();
  await open();
  ok(await question.isVisible(), "forgotten: it asks again");
  await close();
  await open();
  ok(!(await question.count()) && (await header()) === true, "ignored and closed: not asked again, the guess kept");

  // 3. The switch (or any other way of setting the title row) closes the
  // question in place; Ask again brings it back.
  await close();
  await forgetAll();
  await open();
  top = await gridTop();
  await command("First row is header");
  ok((await header()) === false && !(await question.count()) && (await answered.isVisible()), "the switch answers the question in place");
  ok((await gridTop()) === top, "and the table did not move");
  await command("Ask about the title row again");
  ok(await question.isVisible(), "Format › Ask about the title row again brings it back");
  await p.keyboard.press("Alt+Shift+Y");
  await wait(400);
  ok((await header()) === true && (await answered.isVisible()), "Alt+Shift+Y: column names");

  // 4. A private file is not asked (it remembers nothing).
  await close();
  await forgetAll();
  await open();
  await command("Private file mode (this file)");
  await keepCopies();
  await close();
  await open();
  const priv = await p.evaluate(() => window.__nde.tab.private.file || window.__nde.tab.private.folder);
  ok(priv && !(await question.count()), `a private file is not asked (${priv})`);
  await command("Private file mode (this file)");
  await keepCopies();
  await close();

  // 5. Turned off (Format menu, as in Options): no bar for a new file.
  await forgetAll();
  await command("Ask about the title row when a file opens");
  const off = await p.evaluate(() => JSON.parse(localStorage.getItem("natendo.prefs.v1") ?? "{}").askHeader);
  ok(off === false, "the Format command turns the question off (saved)");
  await open();
  ok(!(await question.count()) && (await header()) === true, "off: no question, the guess applied");

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
