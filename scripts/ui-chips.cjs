// UI test of Filter chips (C8) in the browser stand-in: no bar without a filter; one chip per rule with its count;
// click to switch off (one undo step) and on; Match any and Invert all with the count line; "all chips off" and
// "no row matches"; the right-click menu (Invert, Edit, Rename, greyed entries with their reasons); the keyboard
// (Tab, Delete, Shift+F10, F2, Esc); a tool's rows join as a chip beside the others with the status line; the
// filter panel says what its counts mean under Match any; View › Filter chips hides the bar. Made-up sample data.
// Run: (cd ui && npx vite --port 5321) then NODE_PATH=<folder with playwright> node scripts/ui-chips.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5321";

// The stand-in's sample file (see generated() in ui/src/lib/mock.ts).
const FIRST = ["Ada", "Bo", "Cy", "Dee", "Eli", "Fay", "Gus", "Hal", "Ivy", "Jo"];
const CITY = ["Dayton", "Lisbon", "Osaka", "Perth", "Quito", "Tromsø", "Kraków"];
const first = (r) => FIRST[r % FIRST.length];
const city = (r) => CITY[(r * 3) % CITY.length];

(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1280, height: 860 } });
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
  const cmd = (id) => p.evaluate((id) => window.__nde.reg.run(id), id);
  const bar = "[data-testid=filter-chips]";
  const chip = (i) => `${bar} [data-chip="${i}"]`;
  const countLine = async () => ((await p.locator("[data-testid=chips-count]").textContent({ timeout: 3000 }).catch(() => "")) || "").trim();
  const msg = async () => ((await p.locator(".msg").first().textContent().catch(() => "")) || "").replace(/\s+/g, " ").trim();
  const info = () => p.evaluate(() => ({ ...window.__nde.tab.info }));
  // Wait for a change to finish (the chips are greyed while one runs, and read again after).
  const wait = async (ms = 500) => {
    await p.waitForTimeout(ms);
    await p.waitForFunction(() => !document.querySelector("[data-testid=filter-chips] button:disabled") && !window.__nde.tab?.busy, null, { timeout: 30000 }).catch(() => {});
    await p.waitForTimeout(300);
  };
  const words = (s) => s.replace(/\s+/g, " ").trim();

  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => window.__nde.tab && window.__nde.tab.info.indexed && window.__nde.tab.info.rows > 1000, null, { timeout: 30000 });
  await wait(800);
  await p.getByRole("button", { name: "Yes, column names" }).click({ timeout: 1500 }).catch(() => {});
  const total = (await info()).total_rows - 1;
  const rowsWhere = (f) => {
    let n = 0;
    for (let r = 0; r < total; r++) if (f(r)) n++;
    return n;
  };
  const isName = (r) => ["Ada", "Bo"].includes(first(r));
  const isCity = (r) => ["Dayton", "Lisbon", "Osaka"].includes(city(r));

  ok(await p.evaluate(() => window.__nde.reg.get("view.filterChips").checked()), "View › Filter chips is on at first");
  ok((await p.locator(bar).count()) === 0, "no chips bar while no filter is on");

  // Two rules: first_name is Ada or Bo, city is Dayton, Lisbon or Osaka.
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    const spec = {
      columns: [
        { col: 1, rule: { type: "values", values: ["Ada", "Bo"], blanks: false } },
        { col: 3, rule: { type: "values", values: ["Dayton", "Lisbon", "Osaka"], blanks: false } },
      ],
      keep_first: true,
    };
    t.update(await window.__nde.backend.setFilter(t.id, spec), true);
    t.filter = spec;
  });
  await p.waitForSelector(bar, { timeout: 5000 });
  await wait();
  const both = rowsWhere((r) => isName(r) && isCity(r));
  ok((await p.locator(`${bar} .chip`).count()) === 2, "one chip per rule");
  const t0 = words((await p.locator(chip(0)).textContent()) || "");
  ok(/first_name/.test(t0) && t0.includes(String(rowsWhere(isName).toLocaleString())), `the chip names its column and rule, with its own count (${t0})`);
  ok((await countLine()) === `${both.toLocaleString()} of ${total.toLocaleString()} rows show.`, `the bar counts the rows shown (${await countLine()})`);
  if (out) await p.screenshot({ path: path.join(out, "chips-bar.png"), clip: { x: 0, y: 0, width: 1280, height: 260 } });

  // Click the city chip: switched off, one undo step.
  const histBefore = (await info()).history_pos;
  await p.click(chip(1));
  await wait();
  ok((await p.locator(chip(1)).getAttribute("aria-pressed")) === "false", "a click switches the chip off");
  ok((await countLine()).startsWith(`${rowsWhere(isName).toLocaleString()} of`), `with it off, only the name rule counts (${await countLine()})`);
  ok(/^Switched the chip “city .*” off\./.test(await msg()), `the status line says so (${await msg()})`);
  ok((await info()).history_pos === histBefore + 1 && !(await info()).modified, "one undo step, and the file stays saved");
  await p.evaluate(() => window.__nde.grid?.focus?.());
  await p.keyboard.press("Control+z");
  await wait();
  ok((await p.locator(chip(1)).getAttribute("aria-pressed")) === "true" && (await countLine()).startsWith(`${both.toLocaleString()} of`), "Ctrl+Z switches it on again");

  // Match any, then Invert all.
  await p.getByRole("button", { name: "Match any" }).click();
  await wait();
  const either = rowsWhere((r) => isName(r) || isCity(r));
  ok((await countLine()).startsWith(`${either.toLocaleString()} of`), `Match any: rows either chip lets through (${await countLine()})`);
  await cmd("view.panelFilters").catch(() => {});
  await wait();
  const panelNote = ((await p.locator("[data-testid=panel-counts-note]").textContent({ timeout: 1500 }).catch(() => "")) || "").trim();
  ok(/Match any is on: each card counts every row with that value/.test(panelNote), `the filter panel says what its counts mean (${panelNote || "panel not open"})`);
  await p.getByRole("button", { name: "Invert all" }).click();
  await wait();
  ok((await countLine()).startsWith(`${(total - either).toLocaleString()} of`), `Invert all: the other rows (${await countLine()})`);
  await p.getByRole("button", { name: "Invert all" }).click();
  await wait();
  await p.getByRole("button", { name: "Match all" }).click();
  await wait();
  ok((await countLine()).startsWith(`${both.toLocaleString()} of`), "back to Match all");

  // All chips off.
  await p.click(chip(0));
  await wait();
  await p.click(chip(1));
  await wait();
  ok(/^All chips off: every row shows/.test(await countLine()), `all chips off says so (${await countLine()})`);
  ok((await info()).rows === total + 1, "and every row shows");
  await p.click(chip(0));
  await wait();
  await p.click(chip(1));
  await wait();

  // The right-click menu.
  await p.click(chip(0), { button: "right" });
  await wait(300);
  const menu = ".chip-menu";
  ok((await p.locator(menu).count()) === 1, "right-click opens the chip's menu");
  const item = (name) => p.locator(`${menu} button`, { hasText: name });
  ok(await item("Edit the rule").isEnabled(), "Edit the rule is on for a column chip");
  ok(!(await item("Show the error rows").isEnabled()) && /formula/.test((await item("Show the error rows").getAttribute("title")) || ""), "Show the error rows is greyed, with its reason");
  ok(!(await item("Run again").isEnabled()) && /worked out again by itself/.test((await item("Run again").getAttribute("title")) || ""), "Run again is greyed for a column chip, with its reason");
  await item("Invert").click();
  await wait();
  const notText = words((await p.locator(chip(0)).textContent()) || "");
  ok(/^not /.test(notText), `Invert puts "not" on the chip (${notText})`);
  ok((await countLine()).startsWith(`${rowsWhere((r) => !isName(r) && isCity(r)).toLocaleString()} of`), "and the rows are those it does not match");
  await p.keyboard.press("Control+z");
  await wait();

  // Keyboard: Shift+F10 opens the menu, Esc closes it back on the chip, F2 renames, Delete takes it out.
  await p.focus(chip(1));
  await p.keyboard.press("Shift+F10");
  await wait(300);
  ok((await p.locator(menu).count()) === 1, "Shift+F10 opens the menu");
  await p.keyboard.press("ArrowDown");
  await p.keyboard.press("Escape");
  await wait(300);
  ok((await p.locator(menu).count()) === 0 && (await p.evaluate(() => document.activeElement?.getAttribute("data-chip"))) === "1", "Esc closes it, back on the chip");
  await p.keyboard.press("F2");
  await wait(200);
  await p.keyboard.press("Control+a");
  await p.keyboard.type("Three towns");
  await p.keyboard.press("Enter");
  await wait();
  ok(words((await p.locator(chip(1)).textContent()) || "").startsWith("Three towns"), "F2 renames the chip");
  ok(/^Named the chip “Three towns”\./.test(await msg()), `the status line says so (${await msg()})`);
  const cacheKept = (await countLine()).startsWith(`${both.toLocaleString()} of`);
  ok(cacheKept, "renaming shows the same rows");

  // A tool's rows join as a chip beside the others.
  await p.evaluate(() => window.__nde.reg.run("data.showDuplicates"));
  const dlg = "[role=dialog]:has(h2:has-text('Show duplicates'))";
  await p.waitForSelector(dlg, { timeout: 5000 });
  await p.click(`${dlg} .all-none ~ div input[type=checkbox] >> nth=1`);
  await p.click(`${dlg} button.primary`);
  await wait(1500);
  ok((await p.locator(`${bar} .chip`).count()) === 3, "Show duplicates adds a third chip");
  ok(/Duplicates/.test((await p.locator(chip(2)).textContent()) || ""), "named for its tool");
  ok(/Added as a filter chip beside 2 others\./.test(await msg()), `the status line says it joined (${await msg()})`);
  await p.click(chip(2), { button: "right" });
  await wait(300);
  ok(await item("Run again").isEnabled(), "Run again is on for a tool chip made in this session");
  ok(!(await item("Edit the rule").isEnabled()) && /run the tool again/.test((await item("Edit the rule").getAttribute("title")) || ""), "Edit the rule is greyed for a tool chip, with its reason");
  await item("Run again").click();
  await wait(1500);
  ok(/^Ran it again: /.test(await msg()) && (await p.locator(`${bar} .chip`).count()) === 3, `Run again takes the chip out and runs the tool again (${await msg()})`);

  // Delete takes a chip out; × too.
  await p.focus(chip(2));
  await p.keyboard.press("Delete");
  await wait();
  ok((await p.locator(`${bar} .chip`).count()) === 2, "Delete takes the focused chip out");
  ok(/^Took out the chip “Duplicates”\./.test(await msg()), `the status line says so (${await msg()})`);

  // No row matches.
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    const spec = { columns: [...t.filter.columns, { col: 1, rule: { type: "values", values: ["Nobody"], blanks: false } }].filter((c, i, a) => a.findIndex((x) => x.col === c.col) === i || c.rule.values?.[0] === "Nobody"), keep_first: true };
    spec.columns = spec.columns.filter((c) => !(c.col === 1 && c.rule.values?.[0] !== "Nobody"));
    t.update(await window.__nde.backend.setFilter(t.id, spec), true);
  });
  await wait(800);
  ok(/^No row matches/.test(await countLine()), `no row matches says so, and the chips stay (${await countLine()})`);
  ok((await p.locator(`${bar} .chip`).count()) === 2, "both chips are still there to switch off");
  const named = words((await p.locator(`${bar}`).textContent()) || "");
  ok(/Three towns/.test(named), `a new rule elsewhere keeps the chip's name (${named})`);

  // View › Filter chips hides the bar.
  await cmd("view.filterChips");
  await wait();
  ok((await p.locator(bar).count()) === 0, "View › Filter chips hides the bar");
  await cmd("view.filterChips");
  await wait();
  ok((await p.locator(bar).count()) === 1, "and shows it again");

  // Alt+Q finds it.
  await p.evaluate(() => window.__nde.reg.run("help.search")).catch(() => {});
  await wait(300);
  const box = p.locator("[role=dialog] input[type=search], [role=dialog] input").first();
  if (await box.count()) {
    await box.fill("filter chips");
    await wait(400);
    const found = ((await p.locator("[role=dialog]").last().textContent()) || "").replace(/\s+/g, " ");
    ok(/Filter chips/.test(found), "command search finds Filter chips");
    await p.keyboard.press("Escape");
  } else ok(false, "command search opens");

  ok(errors.length === 0, `no page errors${errors.length ? `: ${errors.join(" | ")}` : ""}`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
