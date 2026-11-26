// UI test of the value canonicalizer (C6) in the browser stand-in: drag a value onto another (the chip with its
// cells; Esc cancels with nothing merged), tick several and drag them, Undo merge, Merge into… with Ctrl+M (only in
// the values list), the Abbreviations groups with "maybe" state codes apart and left out of Put all, Save as
// dictionary (and the private-file question), Load with a conflict kept and listed, values not in the dictionary,
// and no change to the data until Change (then one undo step). Made-up data.
// Run: (cd ui && npx vite --port 5321) then NODE_PATH=<folder with playwright> node scripts/ui-canon.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5321";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1280, height: 900 } });
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
  const dlg = "[role=dialog]:has(h2:has-text('Edit values'))";
  const row = (v) => `${dlg} tr[data-value="${v}"]`;
  const changeTo = (v) => p.inputValue(`${row(v)} input[type=text]`);
  const rev = () => p.evaluate(() => window.__nde.tab.info.data_revision);
  const center = async (sel) => {
    const bx = await p.locator(sel).first().boundingBox();
    return { x: bx.x + bx.width / 2, y: bx.y + bx.height / 2 };
  };
  /** Press on a row's grip, move onto another row, and (unless `hold`) let go. */
  const dragOnto = async (from, to, hold = false) => {
    const a = await center(`${row(from)} .grip`);
    const z = await center(`${row(to)} td:nth-child(2)`);
    await p.mouse.move(a.x, a.y);
    await p.mouse.down();
    await p.mouse.move(a.x + 10, a.y + 10, { steps: 3 });
    await p.mouse.move(z.x, z.y, { steps: 8 });
    if (!hold) await p.mouse.up();
    await p.waitForTimeout(200);
  };
  const open = async () => {
    await p.evaluate(() => window.__nde.reg.run("data.values"));
    await p.waitForSelector(dlg);
    await p.selectOption(`${dlg} select:near(:text("Column"))`, { label: "city" }).catch(async () => {
      await p.locator(`${dlg} label:has-text('Column') select`).selectOption({ label: "city" });
    });
    await p.waitForSelector(row("Perth"), { timeout: 20000 });
  };

  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => window.__nde.tab && window.__nde.tab.info.indexed && window.__nde.tab.info.rows > 1000, null, { timeout: 30000 });
  await p.waitForTimeout(800);
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    const v = ["NY", "N.Y.", "New York", "Main St", "Main Street", "Main St", "OK"];
    t.update(await window.__nde.backend.setCells(t.id, v.map((x, i) => [i + 1, 3, x])), true);
    t.header = true;
  });
  await p.waitForTimeout(400);
  const r0 = await rev();
  await open();

  // Drag one: Perth onto Dayton.
  await dragOnto("Perth", "Dayton", true);
  const chip = await p.textContent(`[data-testid=drag-chip]`).catch(() => "");
  ok(/1 value \([\d,]+ cells\) → Dayton/.test(chip || ""), `the chip shows the value, its cells and the target (${chip})`);
  ok(await p.$eval(row("Dayton"), (r) => r.classList.contains("over")), "the target row is outlined");
  if (out) await p.screenshot({ path: path.join(out, "canon-drag.png") });
  await p.mouse.up();
  await p.waitForTimeout(200);
  ok((await changeTo("Perth")) === "Dayton", "dropped: Perth becomes Dayton");

  // Esc cancels a drag: nothing merged, the dialog stays.
  await dragOnto("Lisbon", "Quito", true);
  await p.keyboard.press("Escape");
  await p.mouse.up();
  await p.waitForTimeout(200);
  ok((await p.isVisible(dlg)) && !(await p.isVisible("[data-testid=drag-chip]")), "Esc ends the drag and leaves the dialog open");
  ok((await changeTo("Lisbon")) === "", "Esc merged nothing");

  // Tick two and drag them together.
  await p.click(`${row("Lisbon")} input[type=checkbox]`);
  await p.click(`${row("Quito")} input[type=checkbox]`);
  await dragOnto("Lisbon", "Kraków");
  ok((await changeTo("Lisbon")) === "Kraków" && (await changeTo("Quito")) === "Kraków", "two ticked values dragged together");
  // Undo merge: one step back.
  await p.click(`${dlg} button:has-text('Undo merge')`);
  await p.waitForTimeout(150);
  ok((await changeTo("Lisbon")) === "" && (await changeTo("Quito")) === "" && (await changeTo("Perth")) === "Dayton", "Undo merge steps back one merge");

  // Ctrl+M only in the values list.
  await p.click(`${row("Tromsø")} input[type=checkbox]`);
  await p.focus(`${dlg} input[type=search]`);
  await p.keyboard.press("Control+m");
  await p.waitForTimeout(150);
  ok(!(await p.isVisible(`${dlg} .merge-box`)), "Ctrl+M outside the values list does nothing");
  await p.focus(`${dlg} .values`);
  await p.keyboard.press("Control+m");
  await p.waitForSelector(`${dlg} .merge-box`);
  await p.fill(`${dlg} .merge-box input`, "Tromso");
  await p.keyboard.press("Enter");
  await p.waitForTimeout(150);
  ok((await changeTo("Tromsø")) === "Tromso", "Merge into… with a typed value (keyboard)");
  ok((await rev()) === r0, "nothing changed in the data yet");

  // Abbreviations.
  await p.locator(`${dlg} select[aria-label='Group look-alikes by']`).selectOption("abbreviations");
  await p.waitForSelector(`${dlg} [aria-label='Look-alike groups']`, { timeout: 20000 });
  const normal = await p.textContent(`${dlg} [aria-label='Look-alike groups']`);
  ok(/Main Street[^]*Main St/.test(normal), "Main St and Main Street are a group, the full form first");
  ok(await p.isVisible(`${dlg} [data-testid=maybe-note]`), "state codes in a column that is mostly not states are listed apart as maybe");
  const maybe = await p.textContent(`${dlg} [aria-label='Maybe states or countries']`);
  ok(/New York[^]*NY/.test(maybe), `New York / NY / N.Y. is a maybe group (${maybe.replace(/\s+/g, " ").slice(0, 80)})`);
  if (out) await p.screenshot({ path: path.join(out, "canon-abbreviations.png") });
  await p.click(`${dlg} button:has-text('Put all groups in the list')`);
  await p.click(`${dlg} button:has-text('Put them in the list')`);
  await p.waitForTimeout(200);
  ok((await changeTo("Main St")) === "Main Street" && (await changeTo("NY")) === "", "Put all groups leaves the maybe groups out");

  // Save as dictionary; then the same from a private file asks first.
  await p.click(`${dlg} button:has-text('Save as dictionary')`);
  await p.fill(`${dlg} input[aria-label='Dictionary name']`, "Cities");
  await p.click(`${dlg} button:has-text('Save…')`);
  await p.waitForTimeout(400);
  ok(/Saved “Cities” \(\d+ pairs?\)/.test(await p.textContent(`${dlg} [data-testid=dictionary]`)), "Save as dictionary saves the merges");
  const saved = await p.evaluate(() => [...window.__nde.backend.dictFiles.values()][0]);
  ok(saved && saved.pairs.some(([a, b]) => a === "Perth" && b === "Dayton"), "the dictionary holds the merges");
  await p.evaluate(async () => {
    await window.__nde.backend.setPrivate(window.__nde.tab.id, false, true);
    window.__nde.backend.dictFiles.clear();
  });
  await p.click(`${dlg} button:has-text('Save as dictionary')`);
  await p.click(`${dlg} button:has-text('Save…')`);
  await p.waitForSelector(`${dlg} [data-testid=dict-private]`);
  ok((await p.evaluate(() => window.__nde.backend.dictFiles.size)) === 0, "a private file's dictionary is not saved without a yes");
  await p.click(`${dlg} button:has-text('Save anyway')`);
  await p.waitForTimeout(300);
  ok((await p.evaluate(() => window.__nde.backend.dictFiles.size)) === 1, "Save anyway saves it");
  await p.evaluate(() => window.__nde.backend.setPrivate(window.__nde.tab.id, false, false));

  // Load into a fresh dialog, with Perth merged elsewhere by hand first.
  await p.click(`${dlg} button:has-text('Cancel')`);
  await p.waitForTimeout(300);
  ok(!(await p.isVisible(dlg)), "Cancel closes the dialog with nothing changed");
  await open();
  await p.fill(`${row("Perth")} input[type=text]`, "Osaka");
  await p.locator(`${dlg} [data-testid=dictionary] button:has-text('Load')`).click();
  await p.waitForSelector(`${dlg} [data-testid=dict-note]`);
  const note = await p.textContent(`${dlg} [data-testid=dict-note]`);
  ok(/Loaded “Cities”: \d+ pairs? put in the list/.test(note), `Load puts the dictionary's merges in the list (${note})`);
  ok((await changeTo("Lisbon")) === "" && (await changeTo("Main St")) === "Main Street", "its merges are there");
  ok((await changeTo("Perth")) === "Osaka" && /Perth \(yours: Osaka; dictionary: Dayton\)/.test(await p.textContent(`${dlg} [data-testid=dict-conflicts]`)), "a value merged differently by hand keeps the person's merge and is listed");
  await p.click(`${dlg} button:has-text("Use the dictionary's for these")`);
  await p.waitForTimeout(150);
  ok((await changeTo("Perth")) === "Dayton", "Use the dictionary's for these");
  await p.waitForSelector(`${dlg} [data-testid=dict-new]`);
  await p.click(`${dlg} [data-testid=dict-new] input`);
  await p.waitForTimeout(200);
  ok(!(await p.isVisible(row("Perth"))) && (await p.isVisible(row("Lisbon"))), "Show only values not in the dictionary");
  await p.click(`${dlg} [data-testid=dict-new] input`);

  // Change: one step, then one Ctrl+Z.
  await p.click(`${dlg} button.primary:has-text('Change')`);
  await p.waitForTimeout(1000);
  ok((await rev()) !== r0 && !(await p.isVisible(dlg)), "Change applies the merges");
  await p.evaluate(() => window.__nde.reg.run("edit.undo"));
  await p.waitForTimeout(800);
  ok((await rev()) !== null, "and one Ctrl+Z takes them back");
  const back = await p.evaluate(async () => (await window.__nde.backend.valueTally(window.__nde.tab.id, { col: 3, keep_first: true })).values.some((v) => v.value === "Perth"));
  ok(back, "Perth is back after the undo");

  ok(errors.length === 0, `no page errors (${errors.join("; ")})`);
  console.log(failed ? `${failed} FAILED` : "ALL PASSED");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
