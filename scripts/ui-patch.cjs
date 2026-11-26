// UI test of patch files in the browser stand-in: compare by key, save the changes as a patch (phrase offered,
// one-sided columns refused until left out), then apply it to a third table: dry run, one undo step, a stale dry
// run refused, command search. (The stand-in applies over the shown rows; the real rules are the engine's tests.)
// Run: (cd ui && npx vite --port 5365) then NODE_PATH=<folder with playwright> node scripts/ui-patch.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5365";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const p = await (await b.newContext({ viewport: { width: 1360, height: 900 } })).newPage();
  const errors = [];
  p.on("pageerror", (e) => errors.push(String(e)));
  await p.goto(url);
  await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });
  let failed = 0;
  const ok = (cond, msg) => {
    if (!cond) failed++;
    console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  };
  const body = async () => ((await p.textContent("body")) || "").replace(/\s+/g, " ");
  const run = async (id) => {
    await p.evaluate((id) => void window.__nde.reg.run(id), id);
    await p.waitForTimeout(600);
  };
  const open = async (rows) => {
    await run("file.new");
    await p.evaluate(async (text) => {
      const t = window.__nde.tab;
      t.update(await window.__nde.backend.dataOp(t.id, { op: "paste", spec: { row: 0, col: 0, op: "none", skip_blanks: false, transpose: false }, text }).then((r) => r.info), true);
      t.header = true;
      t.headerTouched = true;
      t.bump();
    }, rows.map((r) => r.join("\t")).join("\r\n"));
    await p.waitForTimeout(300);
    return p.evaluate(() => window.__nde.tab.id);
  };
  const goTab = async (id) => {
    const i = await p.evaluate((x) => window.__nde.tabs.findIndex((t) => t.id === x), id);
    await p.locator("[role=tab], .tab").nth(i).click({ position: { x: 20, y: 8 } });
    await p.waitForTimeout(600);
  };
  const cells = (id) =>
    p.evaluate(async (id) => {
      const t = window.__nde.tabs.find((x) => x.id === id);
      const rows = [];
      for (let r = 0; r < t.info.rows; r++) {
        const line = [];
        for (let c = 0; c < t.info.cols; c++) line.push((await window.__nde.backend.cellText(id, r, c)).text);
        rows.push(line.join("|"));
      }
      return rows.filter((x) => x.replace(/\|/g, ""));
    }, id);
  const old = [["Id", "Name", "Town", "Old note"], ["1", "Zebedee", "Yarrow", "a"], ["2", "Ottilie", "Mossford", "b"], ["3", "Perrin", "Hollin", "c"]];
  const newer = [["Id", "Name", "Town"], ["1", "Zebedee", "Quillby"], ["3", "Perrin", "Hollin"], ["4", "Isolde", "Brackwen"]];
  const a = await open(old);
  const bb = await open(newer);
  const target = await open([["Town", "Id", "Name", "Seen"], ["Yarrow", "1", "Zebedee", "yes"], ["Mossford", "2", "Ottilie", "no"]]);
  const spare = target;

  await goTab(a);
  await run("data.patchApply");
  const dlg = p.getByRole("dialog");
  ok(await dlg.getByRole("button", { name: "Apply", exact: true }).isDisabled(), "Apply waits for a dry run");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);

  await run("tools.compare");
  await dlg.getByLabel("Compare this file with").selectOption(String(bb));
  await dlg.locator("fieldset label", { hasText: "Id" }).first().locator("input").check();
  await dlg.getByRole("button", { name: "Run" }).click();
  await p.waitForTimeout(1200);
  ok((await p.evaluate(() => window.__nde.tabs.filter((t) => t.patchFrom).length)) === 3, "the three compared tabs can save a patch");

  await run("data.patchSave");
  let said = await body();
  ok(/Save changes as a patch/.test(said) && /A patch contains your data/.test(said) && /finding rows by Id/.test(said), "the save screen names the key and says it holds data");
  ok(!(await dlg.getByLabel("Protect with a phrase").isChecked()), "nothing private: the phrase is offered, unticked");
  const save = dlg.getByRole("button", { name: "Save patch…" });
  await save.click();
  await p.waitForTimeout(700);
  ok(/only one table has these columns: Old note/.test(await body()), "a one-sided column stops it");
  await dlg.getByLabel(/Leave out columns/).check();
  await dlg.getByLabel("Protect with a phrase").check();
  ok(await save.isDisabled(), "a ticked phrase is needed");
  await dlg.getByLabel("Phrase", { exact: true }).fill("a made-up phrase 42");
  await dlg.getByLabel("Type it again").fill("a made-up phrase 43");
  ok((await save.isDisabled()) && /The two phrases differ/.test(await body()), "the two must match");
  await dlg.getByLabel("Type it again").fill("a made-up phrase 42");
  if (out) await p.screenshot({ path: path.join(out, "patch-save.png") });
  await save.click();
  await p.waitForTimeout(900);
  said = await body();
  ok((await p.getByRole("dialog").count()) === 0 && /Patch saved: 1 changed row \(1 cell\), 1 added row, 1 removed row\. It is protected/.test(said), "saved, and said so");
  ok(/Left out: Old note/.test(said), "and what was left out");

  await goTab(spare);
  await run("data.patchApply");
  await dlg.getByRole("button", { name: "Choose the patch file…" }).click();
  await p.waitForTimeout(500);
  const check = dlg.getByRole("button", { name: "Check (dry run)" });
  await dlg.getByLabel("Phrase", { exact: false }).fill("not the phrase at all");
  await check.click();
  await p.waitForTimeout(600);
  ok(/wrong password/.test(await body()), "a wrong phrase is refused");
  await dlg.getByLabel("Phrase", { exact: false }).fill("a made-up phrase 42");
  await check.click();
  await p.waitForTimeout(700);
  said = await body();
  ok(/Dry run: Apply would change 1 cell, add 1 row and remove 1 row\./.test(said), `the dry run counts (${(said.match(/Dry run:[^.]*\./) || [""])[0]})`);
  ok(/does not look at these columns: Seen./.test(said) && /private file mode/.test(said), "extra columns and privacy are said");
  ok((await dlg.locator("table tbody tr").count()) === 3, "each entry is listed");
  if (out) await p.screenshot({ path: path.join(out, "patch-apply.png") });
  const before = await cells(spare);
  await dlg.getByRole("button", { name: "Save report as CSV…" }).click();
  await p.waitForTimeout(400);
  ok(/Report saved to/.test(await body()), "the report saves");
  // The table changes under the screen.
  await p.evaluate(async (id) => void (await window.__nde.backend.setCells(id, [[2, 3, "maybe"]])), spare);
  await dlg.getByRole("button", { name: "Apply", exact: true }).click();
  await p.waitForTimeout(700);
  ok(/changed since the dry run/.test(await body()), "a changed table is refused");
  await check.click();
  await p.waitForTimeout(700);
  await dlg.getByRole("button", { name: "Apply", exact: true }).click();
  await p.waitForTimeout(900);
  said = await body();
  ok((await p.getByRole("dialog").count()) === 0 && /Patch applied: 1 cell changed, 1 row added, 1 row removed/.test(said), "applied, and said so");
  const after = await cells(spare);
  ok(after.join() === ["Town|Id|Name|Seen", "Quillby|1|Zebedee|yes", "Brackwen|4|Isolde|"].join(), `the table has the changes (${after.join(" / ")})`);
  await run("edit.undo");
  const undone = await cells(spare);
  ok(undone.length === 3 && undone[1] === before[1] && /Mossford\|2/.test(undone[2]), `one undo puts it back (${undone.join(" / ")})`);
  await run("edit.redo");
  await run("data.patchApply");
  await dlg.getByRole("button", { name: "Choose the patch file…" }).click();
  await p.waitForTimeout(500);
  await dlg.getByLabel("Phrase", { exact: false }).fill("a made-up phrase 42");
  await check.click();
  await p.waitForTimeout(700);
  ok(/Nothing to apply/.test(await body()) && /3 already applied/.test(await body()) && (await dlg.getByRole("button", { name: "Apply", exact: true }).isDisabled()), "a second time there is nothing to apply");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);
  await p.keyboard.press("Alt+q");
  await p.waitForTimeout(300);
  await p.keyboard.type("apply a patch");
  await p.waitForTimeout(500);
  ok(/Apply a patch/.test(await body()), "command search finds it");
  ok(errors.length === 0, `no page errors (${errors.slice(0, 3).join("; ")})`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
