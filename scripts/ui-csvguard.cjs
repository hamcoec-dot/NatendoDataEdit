// UI test of the CSV export safety option in the browser stand-in: the setting (on to start with, in Options and
// command search), the share-safe copy's tick preset from it, the Split dialog's own tick (off, remembered), what each
// of the four outputs asks the app to do, and the message counting the changed cells.
// Run: (cd ui && npx vite --port 5301) then NODE_PATH=<folder with playwright> node scripts/ui-csvguard.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5301";
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
  const TAB = String.fromCharCode(9);
  const EOL = String.fromCharCode(13, 10);
  const run = async (id) => {
    await p.evaluate((id) => window.__nde.reg.run(id), id);
    await p.waitForTimeout(500);
  };
  const asked = (name) => p.evaluate((n) => window.__nde.backend.calls.filter((c) => c.startsWith(`${n}:`)).pop() || "", name);
  const pageText = async () => ((await p.locator("body").textContent()) || "").replace(/\s+/g, " ");
  const close = async () => {
    await p.keyboard.press("Escape");
    await p.waitForTimeout(300);
  };

  await run("file.new");
  await p.evaluate(
    async (text) => {
      const t = window.__nde.tab;
      t.update(await window.__nde.backend.dataOp(t.id, { op: "paste", spec: { row: 0, col: 0, op: "none", skip_blanks: false, transpose: false }, text }).then((r) => r.info), true);
      t.header = true;
      t.bump();
    },
    [["name", "note"], ["ann", "=1+1"], ["bo", "-5"]].map((r) => r.join(TAB)).join(EOL),
  );
  await p.waitForTimeout(400);

  // The setting: on to start with, in Options, switched by the command, kept.
  await run("file.options");
  const opt = p.getByTestId("opt-csv-guard");
  ok(await opt.isChecked(), "Options: the setting is on to start with");
  if (out) await p.screenshot({ path: path.join(out, "csvguard-options.png") });
  await close();
  await run("file.csvGuard");
  await run("file.options");
  ok(!(await opt.isChecked()), "the command switches it off, and Options shows it");
  await close();

  // Share-safe copy: the tick follows the setting; changing it there changes that copy only.
  await run("file.safeCopy");
  const safe = p.getByTestId("safe-guard");
  await safe.waitFor({ timeout: 5000 });
  ok(!(await safe.isChecked()), "share-safe copy: the tick starts as the setting (off)");
  await safe.check();
  await p.getByRole("button", { name: "Save a share-safe copy…" }).click();
  await p.waitForTimeout(600);
  ok((await asked("saveSafeCopy")) === "saveSafeCopy:guard=true", "ticking it asks for the guard on this copy");
  ok(/2 cells that Excel could run as a formula got an apostrophe/.test(await pageText()), "the message counts the changed cells");
  await run("file.options");
  ok(!(await opt.isChecked()), "and the setting itself did not change");
  await opt.check();
  await close();
  await run("file.safeCopy");
  await safe.waitFor({ timeout: 5000 });
  ok(await safe.isChecked(), "with the setting on the tick starts ticked");
  if (out) await p.screenshot({ path: path.join(out, "csvguard-safecopy.png") });
  await safe.uncheck();
  await p.getByRole("button", { name: "Save a share-safe copy…" }).click();
  await p.waitForTimeout(600);
  ok((await asked("saveSafeCopy")) === "saveSafeCopy:guard=false", "unticked, the copy is written as it is");

  // Split: its own tick, off although the setting is on, remembered.
  await run("tools.split");
  const split = p.getByTestId("split-guard");
  await split.waitFor({ timeout: 5000 });
  ok(!(await split.isChecked()), "split: the tick is off to start with, whatever the setting says");
  await p.getByRole("dialog").getByRole("button", { name: "Choose…" }).click();
  await p.waitForTimeout(600);
  ok((await asked("splitToFiles")) === "splitToFiles:guard=false", "split asks for no guard");
  await close();
  await run("tools.split");
  await split.check();
  if (out) await p.screenshot({ path: path.join(out, "csvguard-split.png") });
  await p.getByRole("dialog").getByRole("button", { name: "Choose…" }).click();
  await p.waitForTimeout(600);
  ok((await asked("splitToFiles")) === "splitToFiles:guard=true", "ticked, split asks for the guard");
  await close();
  await run("tools.split");
  ok(await split.isChecked(), "and the tick is remembered");
  await close();

  // The two reports follow the setting.
  await p.keyboard.press("Control+Alt+k");
  await p.getByRole("dialog").getByRole("textbox").fill("begin");
  await p.keyboard.press("Enter");
  await p.waitForTimeout(400);
  await p.evaluate(async () => window.__nde.tab.update(await window.__nde.backend.setCells(window.__nde.tab.id, [[1, 0, "=cmd"]]), true));
  await p.waitForTimeout(300);
  for (const on of [true, false]) {
    await run("edit.changes");
    await p.getByRole("button", { name: "Save as a report…" }).click();
    await p.waitForTimeout(600);
    ok((await asked("saveChangeReport")) === `saveChangeReport:guard=${on}`, `list of changes report, guard ${on}`);
    const said = await pageText();
    ok(on ? /as a CSV report. 2 cells that Excel/.test(said) : /as a CSV report./.test(said) && !/as a CSV report. 2 cells/.test(said), `its message ${on ? "counts the cells" : "says nothing about them"}`);
    await close();
    await p.getByRole("button", { name: "What changed since begin" }).click();
    await p.getByTestId("since-summary").waitFor({ timeout: 5000 });
    await p.getByRole("button", { name: "Save as a report…" }).click();
    await p.waitForTimeout(600);
    ok((await asked("checkpointSinceReport")) === `checkpointSinceReport:guard=${on}`, `since-checkpoint report, guard ${on}`);
    ok(/got an apostrophe/.test((await p.getByTestId("since").textContent()) || "") === on, `its saved line ${on ? "counts the cells" : "says nothing about them"}`);
    await close();
    await run("file.csvGuard");
  }
  ok(errors.length === 0, `no page errors (${errors.join("; ").slice(0, 200)})`);
  await b.close();
  process.exit(failed ? 1 : 0);
})();
