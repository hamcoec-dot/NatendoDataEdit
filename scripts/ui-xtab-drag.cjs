// UI test of dragging whole columns (by their headings) and whole rows (by their numbers) onto another
// file's tab, against the vite dev server (browser stand-in backend). While over a tab the status bar
// says what will happen and where, and changes with Shift; a column goes in to the right of that file's
// selected column (copy), Shift moves it; a tab with rows hidden refuses columns with the reason; rows
// go under the matching titles; wider rows ask; dropping on its own tab says so; the Edit and right-click
// entries are there and greyed with the reason when only one file is open; Alt+Q finds the feature.
// Made-up data only.
// Run: (cd ui && npx vite --port 5321) then
// NODE_PATH=<folder with playwright> node scripts/ui-xtab-drag.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5321";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1360, height: 860 } });
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
  const status = async () => ((await p.textContent("footer.status").catch(() => "")) || "").replace(/\s+/g, " ");
  const settle = () => p.waitForTimeout(500);

  // Only our tabs: close what the page opened with.
  await p.evaluate(async () => {
    const n = window.__nde;
    const add = (name, rows) => n.addTab(n.backend.mockRowsTab(name, rows));
    add("people", [["id", "city", "zip"], ["1", "Ames", "50010"], ["2", "Bly", "97622"], ["3", "Cody", "82414"]]);
    add("march", [["name", "state"], ["Ann", "IA"], ["Bo", "OR"], ["Cy", "WY"]]);
    add("april", [["name", "state"], ["Ann", "IA"], ["Bo", "OR"], ["Cy", "WY"]]);
    add("may", [["name", "state"], ["Ann", "IA"], ["Bo", "OR"], ["Cy", "WY"]]);
    add("other", [["zip", "id", "city"], ["10001", "9", "Nyc"]]);
    add("narrow", [["x"], ["9"]]);
    add("june", [["id", "city", "zip"], ["7", "Gil", "11111"], ["8", "Hy", "22222"]]);
  });
  await settle();
  const idx = async (name) => p.evaluate((name) => window.__nde.tabs.findIndex((t) => t.info.name === name), name);
  const show = async (name) => {
    const i = await idx(name);
    await p.evaluate((i) => {
      const n = window.__nde;
      n.setActive ? n.setActive(i) : null;
    }, i);
    const bx = await p.locator(`[data-tab-index="${i}"]`).boundingBox();
    await p.mouse.click(bx.x + bx.width / 2 - 8, bx.y + bx.height / 2);
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
      return Array.from({ length: n.backend.rowCount(d) }, (_, r) => Array.from({ length: t.info.cols }, (_, c) => n.backend.value(d, r, c)).join(","));
    }, name);
  const select = (name, row, col) =>
    p.evaluate(
      ([name, row, col]) => {
        const n = window.__nde;
        n.grid?.select({ row, col });
      },
      [name, row, col],
    );
  for (const n of ["other", "march", "april", "may", "narrow", "june"]) {
    await show(n);
    await select(n, 1, 0);
  }
  await show("people");

  const canvas = await (await p.$("canvas")).boundingBox();
  const HEAD = 25;
  const ROW = 25;
  // Where the row numbers end.
  let left = 0;
  for (let x = 30; x < 120; x += 2) {
    await p.mouse.click(canvas.x + x, canvas.y + HEAD + ROW + 12);
    if (/C1$/.test(await p.inputValue("input.goto"))) {
      left = x;
      break;
    }
  }
  ok(left > 0, `row numbers end at ${left}px`);
  const W = 130;
  const cx = (c) => canvas.x + left + c * W + W / 2;
  const hy = canvas.y + HEAD / 2;
  const ry = (r) => canvas.y + HEAD + r * ROW + ROW / 2;
  const nx = canvas.x + left / 2;
  const tabXY = async (name) => {
    const bx = await p.locator(`[data-tab-index="${await idx(name)}"]`).boundingBox();
    return [bx.x + bx.width / 2 - 8, bx.y + bx.height / 2];
  };
  /** Press at (x, y), drag onto a tab, look, then let go (Shift held when asked). */
  const dragTo = async (x, y, name, o = {}) => {
    const [tx, ty] = await tabXY(name);
    await p.mouse.move(x, y);
    await p.mouse.down();
    await p.mouse.move(x + 6, y + 2, { steps: 2 });
    await p.mouse.move(tx, ty, { steps: 10 });
    await p.waitForTimeout(150);
    o.over = await status();
    if (o.shiftLive) {
      await p.keyboard.down("Shift");
      await p.waitForTimeout(150);
      o.overShift = await status();
      await p.keyboard.up("Shift");
      await p.waitForTimeout(150);
      o.overAfter = await status();
    }
    if (o.shot) await p.screenshot({ path: path.join(out, o.shot) });
    if (o.shift) await p.keyboard.down("Shift");
    await p.mouse.up();
    if (o.shift) await p.keyboard.up("Shift");
    await p.waitForTimeout(700);
    return o;
  };
  const undoIn = async (name) => {
    await show(name);
    await p.evaluate(() => window.__nde.grid?.focus());
    await p.keyboard.press("Control+z");
    await settle();
  };

  // The city column has a display format and a highlight here (review note 2).
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.columns.formats.set(1, { kind: "number", decimals: 2, thousands: true, symbol: "$", date: "iso" });
    t.highlights = [{ col: 1, rule: { type: "values", values: ["Ames"], blanks: false }, color: 1, whole_row: false }];
    t.bump();
  });
  // ---- a column, copied: the words while over the tab, live with Shift ----
  await p.mouse.click(cx(1), hy); // the city heading: the whole column
  await settle();
  const o = await dragTo(cx(1), hy, "march", { shiftLive: true, shot: out ? "xtab-over.png" : undefined });
  ok(/Drop to copy 1 column into march, after its column 1 \(name\) \(Shift: move\)/.test(o.over), `the status bar says what and where while over the tab (${o.over})`);
  ok(/Drop to move 1 column into march/.test(o.overShift), `pressing Shift changes it at once (${o.overShift})`);
  ok(/Drop to copy/.test(o.overAfter), "and letting go of Shift changes it back");
  ok((await rows("march")).join("|") === "name,city,state|Ann,Ames,IA|Bo,Bly,OR|Cy,Cody,WY", `the column went in right of name, with its title (${(await rows("march")).join("|")})`);
  ok((await rows("people"))[0] === "id,city,zip", "and stayed here (a copy)");
  let st = await status();
  ok(/Copied 1 column into march, after its column 1 \(name\)\. (Highlights and checks stay in people\. )?Undo there with Ctrl\+Z\./.test(st), `the message (${st})`);
  ok((await p.getByRole("button", { name: /^Go to march/ }).count()) === 1, "with Go to march");
  ok(/Highlights and checks stay in people\./.test(st), `the message says the highlight stayed behind (${st})`);
  const fmtIn = (name, c) => p.evaluate(([name, c]) => window.__nde.tabs.find((x) => x.info.name === name).columns.formats.get(c)?.kind ?? null, [name, c]);
  ok((await fmtIn("march", 1)) === "number", "its display format went with it");
  ok((await p.evaluate(() => window.__nde.tabs.find((x) => x.info.name === "march").highlights.length)) === 0, "its highlight did not");
  // One Ctrl+Z there takes the format out with the column.
  await show("march");
  await p.evaluate(() => window.__nde.grid?.focus());
  await p.keyboard.press("Control+z");
  await settle();
  ok((await rows("march"))[0] === "name,state" && (await fmtIn("march", 1)) === null, `Undo there takes the column and its format out (${(await rows("march"))[0]}; ${await fmtIn("march", 1)})`);
  await p.keyboard.press("Control+y");
  await settle();
  ok((await rows("march"))[0] === "name,city,state" && (await fmtIn("march", 1)) === "number", "and Ctrl+Y puts both back");
  await p.keyboard.press("Control+z");
  await settle();
  await show("people");
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.columns.formats.delete(1);
    t.highlights = [];
    t.bump();
  });
  // (One Ctrl+Z in each file is checked on the real app: the stand-in makes several steps.)

  // ---- Shift: moved ----
  await p.mouse.click(cx(2), hy);
  await settle();
  await dragTo(cx(2), hy, "april", { shift: true });
  ok((await rows("april"))[0] === "name,zip,state" && (await rows("people"))[0] === "id,city", `Shift moves it (${(await rows("april"))[0]} / ${(await rows("people"))[0]})`);
  st = await status();
  ok(/Moved 1 column into april.*Ctrl\+Z in each file undoes its half/.test(st), `the message (${st})`);
  await undoIn("people");
  ok((await rows("people"))[0] === "id,city,zip", "Ctrl+Z here puts the moved column back");
  await show("people");

  // ---- a tab with rows hidden refuses columns ----
  await p.evaluate(async () => {
    const n = window.__nde;
    const t = n.tabs.find((x) => x.info.name === "may");
    t.update(await n.backend.change(t.id, { op: "hide_rows", at: 2, count: 1 }), true);
  });
  await p.mouse.click(cx(1), hy);
  await settle();
  await dragTo(cx(1), hy, "may");
  st = await status();
  ok(/may has a filter on or rows hidden: show every row there first, so you can see where the values go\./.test(st), `refused with the reason (${st})`);
  ok((await rows("may"))[0] === "name,state", "and nothing went in");
  await p.evaluate(async () => {
    const n = window.__nde;
    const t = n.tabs.find((x) => x.info.name === "may");
    t.update(await n.backend.change(t.id, { op: "show_rows", which: "all" }), true);
  }).catch(() => {});

  // ---- rows, by the matching titles ----
  await show("people");
  await p.mouse.click(nx, ry(1)); // rows 3 and 4 (Bly, Cody) by their numbers
  await p.keyboard.down("Shift");
  await p.mouse.click(nx, ry(2));
  await p.keyboard.up("Shift");
  await settle();
  const r = await dragTo(nx, ry(1), "other");
  ok(/Drop to copy 2 rows into other, below its row 2 \(Shift: move\)/.test(r.over), `rows: the words while over the tab (${r.over})`);
  ok((await rows("other")).join("|") === "zip,id,city|10001,9,Nyc|97622,2,Bly|82414,3,Cody", `under the matching titles (${(await rows("other")).join("|")})`);
  st = await status();
  ok(/Copied 2 rows into other, below its row 2 under the matching titles\./.test(st), `the message says how (${st})`);

  // ---- rows onto a tab with rows hidden: they go in and show, and the message says so (review note 3) ----
  await p.evaluate(async () => {
    const n = window.__nde;
    const t = n.tabs.find((x) => x.info.name === "june");
    t.update(await n.backend.change(t.id, { op: "hide_rows", at: 2, count: 1 }), true);
  });
  await show("people");
  await p.mouse.click(nx, ry(1));
  await p.keyboard.down("Shift");
  await p.mouse.click(nx, ry(2));
  await p.keyboard.up("Shift");
  await settle();
  await dragTo(nx, ry(1), "june");
  st = await status();
  ok(/Copied 2 rows into june/.test(st) && /The new rows are shown even if the filter there would hide them\./.test(st), `rows on a tab with rows hidden: the message says they show (${st})`);
  const june = await p.evaluate(() => window.__nde.tabs.find((x) => x.info.name === "june").info);
  ok(june.total_rows === 5 && june.rows === 4, `they went in and show: 4 of 5 rows shown, the hidden one still hidden (${june.rows} of ${june.total_rows})`);
  ok((await rows("june")).join("|") === "id,city,zip|7,Gil,11111|2,Bly,97622|3,Cody,82414", `the shown rows (${(await rows("june")).join("|")})`);

  // ---- wider rows ask ----
  await show("people");
  await p.mouse.click(nx, ry(0)); // the row of Ames
  await settle();
  const [tx, ty] = await tabXY("narrow");
  await p.mouse.move(nx, ry(0));
  await p.mouse.down();
  await p.mouse.move(nx + 6, ry(0) + 2, { steps: 2 });
  await p.mouse.move(tx, ty, { steps: 10 });
  await p.mouse.up();
  await settle();
  const ask = ((await p.textContent('[role="dialog"], [role="alertdialog"]').catch(() => "")) || "").replace(/\s+/g, " ");
  ok(/These rows are wider/.test(ask) && /Add 2 columns/.test(ask) && /Cut them to 1/.test(ask), `wider rows ask (${ask.slice(0, 120)})`);
  await p.getByRole("button", { name: "Cut them to 1" }).click();
  await settle();
  ok((await rows("narrow")).join("|") === "x|9|1", `cut to the file's width (${(await rows("narrow")).join("|")})`);

  // ---- its own tab ----
  await show("people");
  await p.mouse.click(cx(1), hy);
  await settle();
  await dragTo(cx(1), hy, "people");
  st = await status();
  ok(/That is this file's tab/.test(st), `dropping on its own tab says so (${st})`);
  ok((await rows("people"))[0] === "id,city,zip", "and changes nothing");

  // ---- the right-click entries and Alt+Q ----
  await p.mouse.click(cx(1), hy, { button: "right" });
  await settle();
  const menu = (await p.textContent('[role="menu"]').catch(() => "")) || "";
  ok(/Copy selection to another file/.test(menu) && /Move selection to another file/.test(menu), "the heading's right-click menu has both entries");
  await p.keyboard.press("Escape");
  await p.evaluate(() => window.__nde.grid?.focus());
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("drag whole columns");
  await p.waitForTimeout(500);
  ok(/Copy selection to another file/.test(await p.evaluate(() => document.body.innerText)), "Alt+Q finds it");
  await p.keyboard.press("Escape");

  ok(errors.length === 0, `no page errors (${errors.join("; ")})`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
