// UI test: custom sort orders. An order made from the selected cells (shown
// cells only, a warning on a private tab, nothing saved before Add); sorting
// by it; Manage custom orders: a sort level keeps its order through rename
// and move, and falls back to Automatic (with a note) when it is deleted;
// the 60-order cap; and the fill handle following an order (only from one
// cell when the order is ticked). Browser stub, made-up data.
// Run: (cd ui && npx vite --port 5393) then
// NODE_PATH=<folder with playwright> node scripts/ui-orders.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2] && !process.argv[2].startsWith("http") ? process.argv[2] : null;
const url = process.argv.find((a) => a.startsWith("http")) || "http://localhost:5393";
const KEY = "natendo.prefs.v1";
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
  const shot = (n) => (out ? p.screenshot({ path: path.join(out, n) }) : null);
  const settle = (ms = 250) => p.waitForTimeout(ms);
  const sel = (r, c, r2 = r, c2 = c) =>
    p.evaluate(([a, b2, c3, d]) => {
      window.__nde.grid.select({ row: a, col: b2 }, { row: c3, col: d });
      window.__nde.grid.focus();
    }, [r, c, r2, c2]).then(() => settle(150));
  const run = (id) => p.evaluate((x) => window.__nde.reg.run(x), id).then(() => settle(400));
  const clip = (t) => p.evaluate((x) => navigator.clipboard.writeText(x), t);
  const col = (c, n) => p.evaluate(([c, n]) => Array.from({ length: n }, (_, r) => window.__nde.tab.cache.get(r, c) ?? "?"), [c, n]);
  const saved = () => p.evaluate((k) => JSON.parse(localStorage.getItem(k) || "{}").sortLists || [], KEY);
  const dialog = () => p.locator(".dialog[aria-labelledby=sort-title]");
  const levelSelect = () => dialog().locator("select[aria-label='Compare as']").first();
  const selectedText = (loc) => loc.evaluate((s) => s.options[s.selectedIndex]?.textContent?.trim() ?? "");
  const pickOrder = async (name) => {
    const v = await levelSelect().evaluate((s, n) => [...s.options].find((o) => o.textContent.trim() === n)?.value ?? "", name);
    await levelSelect().selectOption(v);
    await settle(150);
  };
  const table = async (text) => {
    await run("file.new");
    await settle(500);
    await sel(0, 0);
    await clip(text);
    await p.keyboard.press("Control+v");
    await settle(600);
  };

  await p.goto(url);
  await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });
  await p.evaluate((k) => localStorage.removeItem(k), KEY);
  await p.reload();
  await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });

  // ---- 1. An order from the selected cells, then a sort by it ----
  await table("level	name\r\nHigh	Ada Quill\r\nLow	Bo Frost\r\nMedium	Cy Marsh\r\nLow	Di Fen\r\nHigh	Ed Moor\r\n");
  await sel(1, 0, 5, 0);
  await run("data.orderFromCells");
  ok(await dialog().isVisible(), "New custom order from the selected cells opens the Sort dialog");
  const box = dialog().locator(".newlist textarea");
  ok((await box.inputValue()) === "High\nLow\nMedium", `its values are the cells' different values, row by row (${JSON.stringify(await box.inputValue())})`);
  ok(/3 different values/.test(await dialog().textContent()), "...and it says how many");
  ok(!(await dialog().locator(".warn").count()), "no private warning on an ordinary tab");
  ok((await saved()).length === 0, "nothing is saved before Add");
  await dialog().locator(".newlist input[type=text]").fill("Levels");
  await shot("orders-from-cells.png");
  await dialog().getByRole("button", { name: "Add the order" }).click();
  await settle();
  let s = await saved();
  ok(s.length === 1 && s[0].name === "Levels" && s[0].fromCells === true && s[0].id, `Add saves it with an id, marked as from cells (${JSON.stringify(s.map((o) => [o.id, o.name, o.fromCells]))})`);
  await pickOrder("Levels");
  await dialog().locator("label.hdr input").first().check();
  await dialog().getByRole("button", { name: "Sort", exact: true }).click();
  await settle(600);
  ok(JSON.stringify(await col(0, 6)) === JSON.stringify(["level", "High", "High", "Low", "Low", "Medium"]), `sorting by it puts the rows in its order (${await col(0, 6)})`);

  // ---- 2. A level keeps its order through rename and move; delete falls back ----
  await run("data.sort");
  await pickOrder("Levels");
  // A second order, typed, so there is something to move past.
  await levelSelect().selectOption("new");
  await settle(150);
  await dialog().locator(".newlist input[type=text]").fill("Sizes");
  await dialog().locator(".newlist textarea").fill("Small\nLarge");
  await dialog().getByRole("button", { name: "Add the order" }).click();
  await settle();
  ok((await selectedText(levelSelect())) === "Sizes", "a new order typed from Compare as is used by that level");
  await pickOrder("Levels");
  await dialog().getByRole("button", { name: "Manage custom orders…" }).click();
  await settle();
  const names = dialog().locator(".manage input[aria-label='Name of the order']");
  await names.first().fill("Levels renamed");
  await names.first().dispatchEvent("change");
  await settle();
  ok((await selectedText(levelSelect())) === "Levels renamed", `after a rename the level still uses the order (${await selectedText(levelSelect())})`);
  await dialog().locator(".manage .order").first().getByRole("button", { name: "Move down" }).click();
  await settle();
  s = await saved();
  ok(s.map((o) => o.name).join("|") === "Sizes|Levels renamed", `Move down reorders the saved orders (${s.map((o) => o.name)})`);
  ok((await selectedText(levelSelect())) === "Levels renamed", "after a move the level still uses the order");
  await shot("orders-manage.png");
  await dialog().locator(".manage .order").nth(1).getByRole("button", { name: "Delete…" }).click();
  await settle(150);
  ok(/Delete "Levels renamed"\?/.test(await dialog().textContent()), "Delete asks first");
  await dialog().locator(".manage .order").nth(1).getByRole("button", { name: "Delete", exact: true }).click();
  await settle();
  ok((await saved()).map((o) => o.name).join("|") === "Sizes", "Delete removes it from the saved orders");
  ok((await selectedText(levelSelect())) === "Automatic", `the level that used it falls back to Automatic (${await selectedText(levelSelect())})`);
  ok(/Its custom order was deleted/.test(await dialog().textContent()), "...and says so");
  await dialog().getByRole("button", { name: "Cancel", exact: true }).last().click();
  await settle();

  // ---- 3. A private tab warns; hidden cells are left out and counted ----
  await p.evaluate(() => {
    window.__nde.tab.private = { file: true, folder: false };
  });
  await sel(1, 1, 5, 1);
  await run("data.orderFromCells");
  ok((await dialog().locator(".warn").textContent().catch(() => "")).includes("kept in this app's settings, outside the file"), "on a private tab it warns that the values are kept in the app's settings");
  await shot("orders-private-warning.png");
  await dialog().getByRole("button", { name: "Cancel", exact: true }).first().click();
  await dialog().getByRole("button", { name: "Cancel", exact: true }).last().click();
  await settle();
  ok((await saved()).map((o) => o.name).join("|") === "Sizes", "Cancel saves nothing");
  await p.evaluate(() => {
    window.__nde.tab.private = { file: false, folder: false };
  });
  // Hide the second data row by hand, then take rows 1-5 of the names.
  const hiddenName = await p.evaluate(() => window.__nde.tab.cache.get(2, 1));
  await sel(2, 0);
  await p.keyboard.press("Control+9");
  await settle(500);
  await sel(1, 1, 4, 1);
  await run("data.orderFromCells");
  ok(/1 hidden cell was left out/.test(await dialog().textContent()), `hidden cells in the selection are left out and counted (${(await dialog().locator(".newlist .note").first().textContent()).trim()})`);
  ok(!!hiddenName && !(await dialog().locator(".newlist textarea").inputValue()).includes(hiddenName), `...the hidden row's value (${hiddenName}) is not in the order`);
  await dialog().getByRole("button", { name: "Cancel", exact: true }).last().click();
  await settle();

  // ---- 4. The 60-order cap says so instead of dropping the oldest ----
  await p.evaluate((k) => {
    const prefs = JSON.parse(localStorage.getItem(k) || "{}");
    prefs.sortLists = Array.from({ length: 60 }, (_, i) => ({ id: `cap${i}`, name: `Order ${i}`, items: ["a", "b"] }));
    localStorage.setItem(k, JSON.stringify(prefs));
  }, KEY);
  await p.reload();
  await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });
  await table("x\r\nq\r\nr\r\n");
  await sel(1, 0, 2, 0);
  await run("data.orderFromCells");
  await dialog().getByRole("button", { name: "Add the order" }).click();
  await settle();
  ok(/60 custom orders is the most: delete one first/.test(await dialog().textContent()), "the 61st order says the cap");
  s = await saved();
  ok(s.length === 60 && s[0].name === "Order 0", "...and the oldest order is kept");
  await dialog().getByRole("button", { name: "Cancel", exact: true }).last().click();

  // ---- 5. The fill handle follows an order only when it should (O1) ----
  await p.evaluate((k) => {
    const prefs = JSON.parse(localStorage.getItem(k) || "{}");
    prefs.sortLists = [
      // Made from a column of names, not ticked: never followed, from one cell or two.
      { id: "names", name: "Names", items: ["Ada Quill", "Bo Frost", "Cy Marsh"], fromCells: true },
      // Typed, no tick saved yet: on to start with (up to 100 values).
      { id: "prio", name: "Priority", items: ["Urgent", "Normal", "Later"] },
      // Ticked, but over 100 values: not followed from one cell either.
      { id: "big", name: "Big", items: ["Alpha", ...Array.from({ length: 100 }, (_, i) => `word ${String.fromCharCode(97 + (i % 26))}${i}`.replace(/\d/g, "x"))], fill: true },
    ];
    localStorage.setItem(k, JSON.stringify(prefs));
  }, KEY);
  await p.reload();
  await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });
  await settle(500);
  const sent = await p.evaluate(() => window.__nde.backend.calls.filter((c) => c.startsWith("setFillOrders")).pop());
  ok(sent === "setFillOrders:3", `the app sends the orders it may follow (and the built-in Low, Medium, High), not the unticked one made from cells (${sent})`);
  await table("pri\tname\tbig\r\nUrgent\tAda Quill\tAlpha\r\nx\tx\tx\r\nx\tx\tx\r\nx\tx\tx\r\n");
  const cellBox = (row, c) =>
    p.evaluate(([r, c]) => {
      const g = window.__nde.grid;
      const t = window.__nde.tab;
      const a = g.cellPoint({ row: r, col: c });
      const below = g.cellPoint({ row: r + 1, col: c });
      const h = below.y - a.y || 25;
      return { x: a.x - 4, y: a.y - h, w: t.columns.width(c), h };
    }, [row, c]);
  const status = () => p.locator("footer.status .msg").textContent().catch(() => "");
  /** Drag the fill handle of the selection ending at (row, c) down `n` rows. */
  const dragDown = async (row, c, n, ctrl = false) => {
    const bx = await cellBox(row, c);
    await p.mouse.move(bx.x + bx.w - 3, bx.y + bx.h - 3);
    if (ctrl) await p.keyboard.down("Control");
    await p.mouse.down();
    await p.mouse.move(bx.x + bx.w - 10, bx.y + bx.h * (n + 0.5), { steps: 8 });
    await p.mouse.up();
    if (ctrl) await p.keyboard.up("Control");
    await settle(700);
  };
  await sel(1, 0);
  await dragDown(1, 0, 3);
  ok(JSON.stringify(await col(0, 5)) === JSON.stringify(["pri", "Urgent", "Normal", "Later", "Urgent"]), `one cell on a typed order follows it (${await col(0, 5)})`);
  ok(/following the custom order Priority/.test(await status()), `...and the status line names the order (${(await status()).trim()})`);
  await shot("orders-fill-follows.png");
  // Ctrl while dragging copies (it is put back after).
  await sel(1, 0);
  await dragDown(1, 0, 1, true);
  ok((await col(0, 3))[2] === "Urgent", `Ctrl while dragging copies (${await col(0, 3)})`);

  await sel(1, 1);
  await dragDown(1, 1, 3);
  ok(JSON.stringify(await col(1, 5)) === JSON.stringify(["name", "Ada Quill", "Ada Quill", "Ada Quill", "Ada Quill"]), `one name on an order made from cells copies, never other people's names (${await col(1, 5)})`);
  await sel(1, 2);
  await dragDown(1, 2, 2);
  ok(JSON.stringify(await col(2, 4)) === JSON.stringify(["big", "Alpha", "Alpha", "Alpha"]), `one cell on a ticked order of over 100 values copies (${await col(2, 4)})`);
  // Two neighbouring names on an unticked order made from cells copy: never the next names.
  await p.evaluate(() => window.__nde.reg.run("edit.undo"));
  await settle(500);
  await sel(2, 1);
  await clip("Cy Marsh");
  await p.keyboard.press("Control+v");
  await settle(500);
  await sel(1, 1, 2, 1);
  await dragDown(2, 1, 2);
  const two = await col(1, 5);
  ok(!two.includes("Bo Frost") && two[3] === "Ada Quill" && two[4] === "Cy Marsh", `two names stepping evenly on an unticked order made from cells are copied, not followed (${two})`);
  // The tick off: one cell copies.
  await run("data.sort");
  await dialog().getByRole("button", { name: "Manage custom orders…" }).click();
  await settle();
  const tick = dialog().locator(".manage label.fill input").nth(1);
  ok(await tick.isChecked(), "a typed order's tick is on to start with");
  ok(!(await dialog().locator(".manage label.fill input").nth(0).isChecked()), "an order made from cells is not ticked");
  ok(await dialog().locator(".manage label.fill input").nth(2).isDisabled(), "an order of over 100 values cannot be ticked");
  ok(/uses the higher one/.test(await dialog().textContent()), "the panel says the higher order wins");
  await shot("orders-fill-tick.png");
  await tick.uncheck();
  await settle(300);
  await dialog().getByRole("button", { name: "Cancel", exact: true }).last().click();
  await settle(300);
  await sel(3, 0);
  await clip("Urgent");
  await p.keyboard.press("Control+v");
  await settle(400);
  await sel(3, 0);
  await dragDown(3, 0, 1);
  ok((await col(0, 5))[4] === "Urgent", `with the tick off, one cell copies (${await col(0, 5)})`);
  ok(errors.length === 0, `no page errors${errors.length ? `: ${errors.join(" | ")}` : ""}`);
  await b.close();
  console.log(failed ? `${failed} FAILED` : "ALL PASSED");
  process.exit(failed ? 1 : 0);
})();
