// UI test of Find and Replace v2c: numbers and dates by value, similar
// spelling, preserve case, Include hidden (rows too), by columns, Copy list
// and saved searches.
// Run: (cd ui && npx vite --port 5199) then
// NODE_PATH=<folder with playwright> node scripts/ui-findc.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5199";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1360, height: 860 }, permissions: ["clipboard-read", "clipboard-write"] });
  const p = await ctx.newPage();
  const errors = [];
  p.on("pageerror", (e) => errors.push(String(e)));
  let failed = 0;
  const ok = (cond, msg) => {
    if (!cond) failed++;
    console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  };
  const wait = (ms) => p.waitForTimeout(ms);
  const clip = (t) => p.evaluate((x) => navigator.clipboard.writeText(x), t);
  const cell = (r, c) => p.evaluate(([a, b2]) => window.__nde.tab.cache.get(a, b2), [r, c]);
  const count = () => p.textContent(".find .count").catch(() => "");
  const fstatus = () => p.textContent(".find .status").catch(() => "");
  const tick = (label) => p.locator(".find label", { hasText: label }).locator("input");
  const kind = (k) => p.selectOption("select[aria-label='Pattern kind']", k).then(() => wait(300));
  const type = async (text) => {
    await p.fill("input[aria-label='Find']", text);
    await wait(900);
  };
  const sel = (r, c, r2 = r, c2 = c) =>
    p.evaluate(([a, b2, c3, d]) => {
      window.__nde.grid.select({ row: a, col: b2 }, { row: c3, col: d });
      window.__nde.grid.focus();
    }, [r, c, r2, c2]).then(() => wait(200));
  const findAll = async () => {
    await p.focus("input[aria-label='Find']");
    await p.keyboard.press("Alt+Enter");
    await wait(900);
  };

  await p.goto(url);
  await p.keyboard.press("Control+n");
  await wait(700);
  await sel(0, 0);
  await clip(
    [
      "name\tamount\tvoted\tcity",
      "John Smith\t$5.00\t1/5/2024\tAustin",
      "Jon Smyth\t5\t2024-01-05\tBoston",
      "Jane Doe\t15\t2024-02-01\tAustin",
      "SMITH Co\t(3)\tJan 9 2024\tDallas",
      "Smith Ann\t20.5\tnot a date\tAustin",
    ].join("\r\n") + "\r\n",
  );
  await p.keyboard.press("Control+v");
  await wait(700);
  await p.evaluate(() => {
    window.__nde.tab.header = true;
    window.__nde.tab.bump();
  });
  await sel(1, 0);
  await p.keyboard.press("Control+h");
  await wait(300);

  // Numbers by value.
  await kind("number");
  ok(/A number/.test(await p.getAttribute("input[aria-label='Find']", "placeholder")), "the box says what a number search takes");
  ok(!(await p.isVisible(".find label:has-text('Whole word')")), "text-only options are put away");
  await type("5");
  ok((await count()) === "2 matches", `5 finds $5.00 and 5, not 15 (${await count()})`);
  await type("10..21");
  ok((await count()) === "2 matches", `10..21 finds 15 and 20.5 (${await count()})`);
  await findAll();
  if (out) await p.screenshot({ path: path.join(out, "find-numbers.png"), clip: { x: 0, y: 0, width: 1360, height: 400 } });
  await p.click(".find .rhead button.x");
  await type("10..20");
  ok((await count()) === "1 match", `10..20 leaves 20.5 out (${await count()})`);
  await type("<0");
  ok((await count()) === "1 match", `<0 finds (3) (${await count()})`);
  await type("abc");
  await p.keyboard.press("Enter");
  await wait(500);
  ok(/not a number/.test(await fstatus()), `a bad number says so (${await fstatus()})`);

  // Dates by value.
  await kind("date");
  await type("1/5/2024");
  ok((await count()) === "2 matches", `1/5/2024 finds 2024-01-05 too (${await count()})`);
  await type("1/1/2024..1/31/2024");
  ok((await count()) === "3 matches", `a January range finds Jan 9 2024 too (${await count()})`);
  // Day first: 5/1/2024 is 5 January.
  await type("5/1/2024");
  ok((await count()) === "No matches", `month first by default, 5/1/2024 is May 1 (${await count()})`);
  await tick("Day first").check();
  await wait(900);
  ok((await count()) === "1 match", `with Day first ticked it is 5 January (and the cell 1/5/2024 is read as 1 May) (${await count()})`);
  await tick("Day first").uncheck();
  await type(">1/31/2024");
  ok((await count()) === "1 match", `after January (${await count()})`);

  // Similar spelling.
  await kind("similar");
  await type("Jon Smyth");
  ok(/(^| of )2( matches)?$/.test(await count()), `Jon Smyth finds John Smith (${await count()})`);
  await findAll();
  const names = await p.$$eval(".find .hit .snip", (e) => e.map((x) => x.textContent));
  ok(names.join("|") === "John Smith|Jon Smyth", `the list shows both (${names.join("|")})`);
  if (out) await p.screenshot({ path: path.join(out, "find-similar.png"), clip: { x: 0, y: 0, width: 1360, height: 420 } });
  await p.click(".find .rhead button.x");

  // Preserve case.
  await kind("plain");
  await type("smith");
  await p.fill("input[aria-label='Replace with']", "jones");
  await tick("Preserve case").check();
  await p.click(".find button:has-text('Replace all…')");
  await wait(700);
  await p.click(".find .confirm button.primary");
  await wait(800);
  ok((await cell(1, 0)) === "John Jones" && (await cell(4, 0)) === "JONES Co" && (await cell(5, 0)) === "Jones Ann", `case is kept (${await cell(1, 0)}, ${await cell(4, 0)}, ${await cell(5, 0)})`);
  await tick("Preserve case").uncheck();
  await p.evaluate(() => window.__nde.grid.focus());
  await p.keyboard.press("Control+z");
  await wait(600);

  // By columns: Find next goes down a column first.
  await type("Austin");
  await tick("By columns").check();
  await type("a");
  await sel(1, 0);
  await p.focus("input[aria-label='Find']");
  const order = [];
  for (let i = 0; i < 4; i++) {
    await p.keyboard.press("Enter");
    await wait(400);
    order.push(await p.evaluate(() => `${window.__nde.tab.cell.row}:${window.__nde.tab.cell.col}`));
  }
  ok(order.join(" ") === "3:0 5:0 4:2 5:2", `Find next goes down the first column, then the next (${order.join(" ")})`);
  await tick("By columns").uncheck();

  // Include hidden: rows hidden by hand are counted and listed.
  await type("Austin");
  ok((await count()) === "3 matches", "three Austins");
  await sel(3, 0);
  await p.keyboard.press("Control+9");
  await wait(600);
  await p.focus("input[aria-label='Find']");
  await type("Austi");
  await type("Austin");
  ok((await count()) === "2 matches", `a hidden row is skipped (${await count()})`);
  await tick("Include hidden").check();
  await wait(900);
  ok((await count()) === "3 matches", `Include hidden counts it (${await count()})`);
  await findAll();
  const where = await p.$$eval(".find .hit .where", (e) => e.map((x) => x.textContent));
  ok(where.some((w) => /\(hidden\)/.test(w)), `the list marks the hidden one (${where.join(" | ")})`);
  ok(/Shown rows with a match/.test(await p.textContent(".find .acts")), "row actions say they work on shown rows");

  // Copy list.
  await p.click(".find button:has-text('Copy list')");
  await wait(500);
  const copied = await p.evaluate(() => navigator.clipboard.readText());
  ok(/^Row\tColumn\tCell\r\n/.test(copied) && copied.split("\r\n").filter(Boolean).length === 4 && /\(hidden\)/.test(copied), `Copy list copies the rows (${JSON.stringify(copied.slice(0, 80))})`);
  ok(/Copied the list of 3 matches/.test(await fstatus()), "and says so");
  await p.click(".find .rhead button.x");
  await tick("Include hidden").uncheck();

  // Saved searches.
  await kind("number");
  await type(">10");
  await p.click(".find button:has-text('Saved ▾')");
  await wait(200);
  ok(/No saved searches yet/.test(await p.textContent(".find .menu.right")), "no saved searches at first");
  await p.fill("input[aria-label='Name for this search']", "Big amounts");
  await p.keyboard.press("Enter");
  await wait(300);
  ok(/Saved "Big amounts"/.test(await fstatus()), "Save keeps the search");
  await kind("plain");
  await type("zzz");
  await p.click(".find button:has-text('Saved ▾')");
  await wait(200);
  if (out) await p.screenshot({ path: path.join(out, "find-saved.png"), clip: { x: 700, y: 0, width: 660, height: 330 } });
  await p.click(".find .menu.right button:has-text('Big amounts')");
  await wait(900);
  ok((await p.inputValue("input[aria-label='Find']")) === ">10" && (await p.inputValue("select[aria-label='Pattern kind']")) === "number" && (await count()) === "1 match", `a saved search comes back (one row is hidden) with its options (${await count()})`);
  const kept = await p.evaluate(() => JSON.parse(localStorage.getItem(Object.keys(localStorage).find((k) => /pref/i.test(k)) ?? "") || "{}").savedSearches?.length ?? -1);
  ok(kept === 1, `it is kept in Options (${kept})`);
  await p.click(".find button:has-text('Saved ▾')");
  await wait(200);
  await p.click(".find .menu.right button[aria-label='Delete Big amounts']");
  await wait(200);
  ok(/No saved searches yet/.test(await p.textContent(".find .menu.right")), "a saved search can be deleted");
  // A saved search with no replacement clears the Replace with box.
  await p.click(".find button:has-text('Saved ▾')");
  await p.fill("input[aria-label='Replace with']", "");
  await p.click(".find button:has-text('Saved ▾')");
  await p.fill("input[aria-label='Name for this search']", "No replacement");
  await p.keyboard.press("Enter");
  await wait(300);
  await p.fill("input[aria-label='Replace with']", "leftover");
  await p.click(".find button:has-text('Saved ▾')");
  await wait(200);
  await p.click(".find .menu.right button:has-text('No replacement')");
  await wait(500);
  ok((await p.inputValue("input[aria-label='Replace with']")) === "", "a saved search with no replacement clears the box");

  // Saved searches with a repeated or blank name (a damaged settings file)
  // still open the menu.
  await p.evaluate(() => {
    const k = Object.keys(localStorage).find((x) => /pref/i.test(x));
    const v = JSON.parse(localStorage.getItem(k) || "{}");
    v.savedSearches = [{ name: "Twice", text: "a" }, { name: "Twice", text: "b" }, { name: "", text: "c" }, { name: "Once", text: "d" }];
    localStorage.setItem(k, JSON.stringify(v));
  });
  await p.reload();
  await wait(1200);
  await p.keyboard.press("Control+n");
  await wait(600);
  await p.keyboard.press("Control+f");
  await wait(300);
  await p.click(".find button:has-text('Saved ▾')");
  await wait(300);
  const items = await p.$$eval(".find .menu.right .saved button[role=menuitem]", (e) => e.map((x) => x.textContent));
  ok(items.join(",") === "Twice,Once", `repeated and blank names are dropped (${items.join(",")})`);

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
