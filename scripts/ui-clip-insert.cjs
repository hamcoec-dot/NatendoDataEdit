// UI test of the table a Copy puts on the clipboard (for e-mail and Word) and of Insert copied rows or columns
// (Ctrl+Shift++), against the vite dev server (browser stand-in backend).
// The clipboard holds text and an HTML table: values escaped, every cell that is not a plain number marked as text
// for Excel, display formats as the grid shows them; none from a private tab; none over 20,000 cells (and the
// message says so). Ctrl+Shift++ after copying whole rows puts them in above the selected row, whole columns
// before the selected column (titles in the title row, widths and formats carried), one undo step each;
// Ctrl+NumpadAdd does the same; Ctrl+Alt (AltGr) never does. After another program puts something on the
// clipboard there is no offer (the menu entry is greyed with the reason). A cut here moves; a cut in another
// tab is refused with words and nothing moves. Screenshots of the clipboard table in a browser (made-up data).
// Run: (cd ui && npx vite --port 5321) then
// NODE_PATH=<folder with playwright> node scripts/ui-clip-insert.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5321";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1360, height: 860 } });
  await ctx.grantPermissions(["clipboard-read", "clipboard-write"], {
    origin: new URL(url).origin,
  });
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
  const status = async () =>
    ((await p.textContent("footer.status").catch(() => "")) || "").replace(
      /\s+/g,
      " ",
    );
  const settle = () => p.waitForTimeout(400);

  await p.evaluate(async () => {
    const n = window.__nde;
    const add = (name, rows) => n.addTab(n.backend.mockRowsTab(name, rows));
    add("people", [
      ["id", "city", "zip", "paid"],
      ["1", "Ames <b>&", "01234", "1234.5"],
      ["2", "=1+1", "1-2", "20"],
      ["3", "Cody", "82414", "7"],
      ["4", "Dell", "50010", "8"],
    ]);
    add("march", [
      ["name", "state"],
      ["Ann", "IA"],
      ["Bo", "OR"],
      ["Cy", "WY"],
      ["Di", "TX"],
    ]);
    add("secret", [["ssn"], ["000-00-0000"]]);
    const big = [["a", "b"]];
    for (let i = 0; i < 10_001; i++) big.push([String(i), "x"]);
    add("big", big);
  });
  await settle();
  const show = async (name) => {
    await p.evaluate((name) => {
      const n = window.__nde;
      const i = n.tabs.findIndex((t) => t.info.name === name);
      n.activate(i);
    }, name);
    await settle();
    await p.evaluate(() => {
      window.__nde.tab.header = true;
    });
  };
  const rows = (name) =>
    p.evaluate((name) => {
      const n = window.__nde;
      const t = n.tabs.find((x) => x.info.name === name);
      const d = n.backend.get(t.id);
      return Array.from({ length: n.backend.rowCount(d) }, (_, r) =>
        Array.from({ length: t.info.cols }, (_, c) =>
          n.backend.value(d, r, c),
        ).join(","),
      );
    }, name);
  const select = (a, z) =>
    p.evaluate(
      ([a, z]) => {
        window.__nde.grid.select(a, z);
        window.__nde.grid.focus();
      },
      [a, z],
    );
  const clip = () =>
    p.evaluate(async () => {
      const items = await navigator.clipboard.read();
      const it = items[0];
      const text = it.types.includes("text/plain")
        ? await (await it.getType("text/plain")).text()
        : null;
      const html = it.types.includes("text/html")
        ? await (await it.getType("text/html")).text()
        : null;
      return { text, html };
    });
  const key = (k) => p.keyboard.press(k).then(settle);

  // ---- the table on the clipboard ----
  await show("people");
  await p.evaluate(() => {
    window.__nde.tab.columns.formats.set(3, {
      kind: "currency",
      decimals: 2,
      thousands: true,
      symbol: "$",
      date: "iso",
    });
  });
  await select({ row: 1, col: 0 }, { row: 2, col: 3 });
  await key("Control+Shift+C");
  let c = await clip();
  ok(
    c.text && c.text.startsWith("id\tcity\tzip\tpaid\r\n1\t"),
    "Copy with column names: the text is on the clipboard as before",
  );
  // (The browser's clipboard read drops the <head>; the character set is checked in cliphtml.test.ts and in the real app.)
  ok(c.html && c.html.includes("<table"), "and an HTML table");
  ok(
    c.html.includes("<th") && c.html.includes(">city</th>"),
    "the column names are the table's header row",
  );
  ok(
    c.html.includes("Ames &lt;b&gt;&amp;") && !c.html.includes("<b>"),
    "values are escaped",
  );
  const cell = (v) =>
    new RegExp(`<td style="([^"]*)">${v.replace(/[$]/g, "\\$")}</td>`).exec(
      c.html,
    )?.[1] ?? "";
  ok(
    cell("01234").includes("mso-number-format:'\\@'") &&
      cell("=1\\+1").includes("mso-number-format") &&
      cell("1-2").includes("mso-number-format"),
    "leading zeros, =1+1 and 1-2 are marked as text for Excel",
  );
  ok(
    cell("1").includes("text-align:right") &&
      !cell("1").includes("mso-number-format"),
    "a plain number stays a number",
  );
  ok(
    c.html.includes(">$1,234.50</td>"),
    "a currency column shows as the grid shows it",
  );
  if (out) {
    const v = await ctx.newPage();
    await v.setViewportSize({ width: 640, height: 220 });
    await v.setContent(c.html);
    await v.screenshot({ path: path.join(out, "clip-table.png") });
    await v.close();
  }

  // Over the cap: text only, and the message says so.
  await show("big");
  await select({ row: 1, col: 0 }, { row: 10_001, col: 1 });
  await key("Control+c");
  c = await clip();
  ok(
    c.html === null &&
      c.text &&
      (await status()).includes("Too many cells for an e-mail table"),
    `over 20,000 cells: text only, said (${await status()})`,
  );

  // A private tab: no table.
  await show("secret");
  await p.evaluate(async () => {
    const n = window.__nde;
    await n.backend.setPrivate(n.tab.id, false, true);
    n.tab.private = { file: true, folder: false };
  });
  await select({ row: 1, col: 0 }, { row: 1, col: 0 });
  await key("Control+c");
  c = await clip();
  ok(
    c.html === null && /000-00-0000/.test(c.text ?? ""),
    "a private tab puts only the text on the clipboard, no table",
  );

  // ---- insert copied rows ----
  await show("people");
  const before = await rows("people");
  await select({ row: 1, col: 0 }, { row: 2, col: 3 });
  await key("Control+c");
  await select({ row: 4, col: 1 }, { row: 4, col: 1 });
  await key("Control+Shift+Equal");
  let r = await rows("people");
  ok(
    r.length === 7 &&
      r[4] === "1,Ames <b>&,01234,1234.5" &&
      r[5] === "2,=1+1,1-2,20" &&
      r[6] === "4,Dell,50010,8",
    `Ctrl+Shift++ puts the copied rows in above the selected row (${r.slice(3).join(" | ")})`,
  );
  ok(
    (await status()).includes("Inserted 2 copied rows above row 5"),
    `the message says what went where (${await status()})`,
  );
  await key("Control+z");
  ok(
    JSON.stringify(await rows("people")) === JSON.stringify(before),
    "one Ctrl+Z takes them out",
  );

  // Ctrl+NumpadAdd, and never AltGr.
  await select({ row: 1, col: 0 }, { row: 1, col: 3 });
  await key("Control+c");
  await select({ row: 2, col: 0 }, { row: 2, col: 0 });
  await key("Control+Alt+Shift+Equal");
  ok(
    (await rows("people")).length === 5,
    "Ctrl+Alt (AltGr) +: nothing goes in",
  );
  await key("Control+NumpadAdd");
  r = await rows("people");
  ok(
    r.length === 6 && r[2] === "1,Ames <b>&,01234,1234.5",
    "Ctrl+NumpadAdd puts the copied row in",
  );
  await key("Control+z");

  // Another program puts something on the clipboard: no offer, and the menu says why.
  await p.evaluate(() => navigator.clipboard.writeText("from another program"));
  await select({ row: 3, col: 1 }, { row: 3, col: 1 });
  const canvas = await (await p.$("canvas")).boundingBox();
  await p.mouse.click(canvas.x + 200, canvas.y + 25 + 25 * 3 + 12, {
    button: "right",
  });
  await p.waitForTimeout(400);
  const item = await p.evaluate(() => {
    const b = [...document.querySelectorAll(".menu.context button")].find((x) =>
      /copied/i.test(x.textContent ?? ""),
    );
    return b
      ? { disabled: b.disabled, title: b.title, label: b.textContent.trim() }
      : null;
  });
  ok(
    item && item.disabled && /another program/.test(item.title),
    `the menu entry is greyed: "${item?.title}"`,
  );
  await p.keyboard.press("Escape");
  await settle();
  const n0 = (await rows("people")).length;
  await select({ row: 3, col: 0 }, { row: 3, col: 3 });
  await key("Control+Shift+Equal");
  r = await rows("people");
  ok(
    r.length === n0 + 1 && r[3] === ",,,",
    "with something else on the clipboard Ctrl+Shift++ inserts a blank row, as before",
  );
  await key("Control+z");

  // ---- insert copied columns, with titles, widths and formats ----
  await p.evaluate(() => {
    window.__nde.tab.columns.setWidth(1, 211);
  });
  await select({ row: 0, col: 1 }, { row: 4, col: 1 });
  await key("Control+c");
  await show("march");
  await select({ row: 1, col: 1 }, { row: 1, col: 1 });
  await key("Control+Shift+Equal");
  r = await rows("march");
  ok(
    r[0] === "name,city,state" &&
      r[1] === "Ann,Ames <b>&,IA" &&
      r[2] === "Bo,=1+1,OR",
    `copied columns go in before the selected column with their title (${r.join(" | ")})`,
  );
  const w = await p.evaluate(() => window.__nde.tab.columns.customWidth(1));
  ok(w === 211, `its width came with it (${w})`);
  ok(
    (await status()).includes("Inserted 1 copied column before column 2"),
    `said (${await status()})`,
  );
  await key("Control+z");
  ok((await rows("march"))[0] === "name,state", "one Ctrl+Z takes it out");

  // ---- cuts ----
  await show("people");
  const p0 = await rows("people");
  await select({ row: 1, col: 0 }, { row: 1, col: 3 });
  await key("Control+x");
  await show("march");
  await select({ row: 2, col: 0 }, { row: 2, col: 1 });
  const m0 = await rows("march");
  await key("Control+Shift+Equal");
  ok(
    JSON.stringify(await rows("march")) === JSON.stringify(m0) &&
      JSON.stringify(await rows("people")) === JSON.stringify(p0),
    "a cut in another tab: nothing moves",
  );
  ok(
    (await status()).includes(
      "moves only inside that file, so nothing was moved",
    ),
    `and it says so (${await status()})`,
  );
  await show("people");
  await select({ row: 1, col: 0 }, { row: 1, col: 3 });
  await key("Control+x");
  await select({ row: 4, col: 0 }, { row: 4, col: 3 });
  await key("Control+Shift+Equal");
  r = await rows("people");
  ok(
    r[1].startsWith("2,") && r[3].startsWith("1,") && r.length === 5,
    `a cut here moves the rows (${r.join(" | ")})`,
  );
  await key("Control+z");

  ok(!errors.length, `no page errors ${errors.join("; ")}`);
  await b.close();
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  process.exit(failed ? 1 : 0);
})();
