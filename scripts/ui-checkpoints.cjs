// UI test of Checkpoints in the browser stand-in: naming a moment by key and by button, the list at the top of the
// Steps panel, going back and forward in one jump, the "Before restoring" moment kept, greyed after a newer change
// replaced what was undone and after a save, forgetting one, and keyboard use.
// Run: (cd ui && npx vite --port 5271) then NODE_PATH=<folder with playwright> node scripts/ui-checkpoints.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5271";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1280, height: 820 } });
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
  await p.evaluate(() => window.__nde.reg.run("file.new"));
  await p.waitForTimeout(500);
  const TAB = String.fromCharCode(9);
  const EOL = String.fromCharCode(13, 10);
  const rows = [["name", "city"]];
  for (let i = 0; i < 10; i++) rows.push([`Person${i}`, `City${i}`]);
  await p.evaluate(
    async (text) => {
      const t = window.__nde.tab;
      t.update(await window.__nde.backend.dataOp(t.id, { op: "paste", spec: { row: 0, col: 0, op: "none", skip_blanks: false, transpose: false }, text }).then((r) => r.info), true);
      t.header = true;
      t.bump();
    },
    rows.map((r) => r.join(TAB)).join(EOL),
  );
  await p.waitForTimeout(400);
  const cell = (r, c) => p.evaluate(async ([r, c]) => (await window.__nde.backend.cellText(window.__nde.tab.id, r, c)).text, [r, c]);
  const edit = (r, c, text) => p.evaluate(async ([r, c, text]) => window.__nde.tab.update(await window.__nde.backend.setCells(window.__nde.tab.id, [[r, c, text]]), true), [r, c, text]);
  const items = () => p.getByTestId("checkpoint");
  const item = (n) => items().filter({ has: p.locator(".n", { hasText: new RegExp(`^${n}$`) }) });
  const names = async () => (await items().locator(".n").allTextContents()).map((t) => t.trim());

  // Name a moment by key.
  await p.evaluate(() => window.__nde.grid.select({ row: 1, col: 0 }));
  await p.keyboard.press("Control+Alt+k");
  const box = p.getByRole("dialog").getByRole("textbox");
  await box.waitFor({ timeout: 5000 });
  await box.fill("clean start");
  await p.keyboard.press("Enter");
  await p.getByTestId("checkpoints").waitFor({ timeout: 5000 });
  await p.waitForTimeout(400);
  ok(JSON.stringify(await names()) === '["clean start"]', "Ctrl+Alt+K names this moment and the Steps panel lists it");
  if (out) await p.screenshot({ path: path.join(out, "checkpoints.png") });

  // Change things, name a second moment by the button.
  await edit(1, 0, "Alpha");
  await edit(2, 0, "Beta");
  await p.getByRole("button", { name: "Name this moment" }).click();
  await p.getByLabel("Checkpoint name").fill("two edits");
  await p.getByLabel("Checkpoint name").press("Enter");
  await p.waitForTimeout(400);
  ok(JSON.stringify(await names()) === '["two edits","clean start"]', "the button names a moment too (newest first)");
  ok((await items().first().innerText()).includes("now"), "the moment the table is in is marked now");

  // Go back: one jump.
  await edit(3, 0, "Gamma");
  await item("clean start").locator("button.go").click();
  await p.waitForTimeout(500);
  ok((await cell(1, 0)) === "Person0" && (await cell(2, 0)) === "Person1" && (await cell(3, 0)) === "Person2", "going back to the first checkpoint undoes every edit since in one jump");
  const after = await names();
  ok(after.some((n) => n.startsWith("Before restoring")), `the moment left is kept (${after.join(" | ")})`);

  // Forward again to "two edits" (still reachable by redo).
  await item("two edits").locator("button.go").click();
  await p.waitForTimeout(500);
  ok((await cell(1, 0)) === "Alpha" && (await cell(2, 0)) === "Beta" && (await cell(3, 0)) === "Person2", "and forward to the second one");
  // The one it left (Gamma state) is reachable again through its automatic checkpoint.
  await items().filter({ hasText: "Before restoring" }).first().locator("button.go").click();
  await p.waitForTimeout(500);
  ok((await cell(3, 0)) === "Gamma", "the way back is one click: the Before restoring checkpoint");

  // A newer change after undoing replaces the branch: checkpoints on it are greyed and say why.
  await item("clean start").locator("button.go").click();
  await p.waitForTimeout(400);
  await edit(4, 0, "Delta");
  await p.waitForTimeout(500);
  const gone = await p.locator("li.gone").count();
  ok(gone >= 1, `a new change greys the checkpoints on the replaced branch (${gone} greyed)`);
  ok((await p.locator("li.gone .tag").first().innerText()).includes("no longer reachable"), "and it says so");
  ok(await p.locator("li.gone button.go").first().isDisabled(), "a greyed checkpoint cannot be clicked");

  // A save starts the history again: every checkpoint is greyed.
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    t.update(await window.__nde.backend.save(t.id, { saveAs: "x.csv" }).catch(() => window.__nde.backend.save(t.id, {})), true);
  });
  await p.waitForTimeout(500);
  const all = await items().count();
  ok(all > 0 && (await p.locator("li.gone").count()) === all, "after a save every checkpoint is greyed");

  // Forget one (the table does not change).
  const before = await items().count();
  const cellBefore = await cell(4, 0);
  await items().first().locator("button.x").click();
  await p.waitForTimeout(300);
  ok((await items().count()) === before - 1 && (await cell(4, 0)) === cellBefore, "forgetting a checkpoint leaves the table alone");

  // Keyboard only: name with the key, Esc cancels the name box.
  await p.keyboard.press("Control+Alt+k");
  await box.waitFor({ timeout: 5000 });
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);
  ok((await p.getByRole("dialog").count()) === 0, "Esc closes the name box");
  ok(errors.length === 0, `no page errors ${errors.join(" | ").slice(0, 200)}`);
  await b.close();
  process.exit(failed ? 1 : 0);
})();
