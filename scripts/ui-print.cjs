// UI test: Print the selection and the two fits. The selected block prints
// only its shown rows and columns (Save as PDF gets them too); several
// blocks turn the choice off with a note; the preview frame stays fully
// sandboxed and its page starts with the content policy; fit to width
// shrinks Print's text by the PDF's rule: laid out in a real page, the
// 30-column sample fits the page, no cell wraps, the text size is within
// 0.5 pt of the PDF's (crates/engine/tests/print-fit/thirty.json), and the
// canvas widths are within 3% of the laid-out ones; a 60-character email
// in a narrow column stays in its own cell; fit on one page puts 40 rows
// on one page and says when 2,000 rows cannot fit. Browser stub, made-up data.
// Run: (cd ui && npx vite --port 5395) then
// NODE_PATH=<folder with playwright> node scripts/ui-print.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const fs = require("fs");
const path = require("path");
const out = process.argv[2] && !process.argv[2].startsWith("http") ? process.argv[2] : null;
const url = process.argv.find((a) => a.startsWith("http")) || "http://localhost:5395";
const fixture = path.join(__dirname, "..", "crates", "engine", "tests", "print-fit");
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1360, height: 900 }, permissions: ["clipboard-read", "clipboard-write"] });
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
  const run = (id) => p.evaluate((x) => window.__nde.reg.run(x), id).then(() => settle(500));
  const clip = (t) => p.evaluate((x) => navigator.clipboard.writeText(x), t);
  const table = async (text) => {
    await run("file.new");
    await settle(500);
    await sel(0, 0);
    await clip(text);
    await p.keyboard.press("Control+v");
    await settle(800);
    // The pasted first row is the column titles.
    const yes = p.getByRole("button", { name: /Yes, column names/ });
    if (await yes.count()) {
      await yes.first().click();
      await settle(400);
    }
    if (!(await p.evaluate(() => window.__nde.tab.header))) await run("format.header");
  };
  const dlg = () => p.locator("[role=dialog]").filter({ has: p.locator("iframe[title=Preview]") });
  const frame = () => dlg().locator("iframe[title=Preview]");
  const srcdoc = () => frame().getAttribute("srcdoc");
  const close = async () => {
    await p.keyboard.press("Escape");
    await settle(300);
  };
  // Print and PDF ask first on a file that looks private: go on.
  const openP = async (id) => {
    await run(id);
    const go = p.getByRole("button", { name: "Continue", exact: true });
    if (await go.count()) {
      await go.click();
      await settle(600);
    }
  };
  const tsv = (rows) => rows.map((r) => r.join("\t")).join("\r\n") + "\r\n";

  await p.goto(url);
  await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });

  // ---- 1. Print the selection: the block's shown rows and columns ----
  const names = ["Avery Ashdown", "Blake Brightwell", "Casey Coldfield", "Devon Dunmore", "Emery Elmsworth", "Finley Fairbank"];
  const rows = [["id", "name", "secret", "city", "amount", "status"]];
  for (let i = 0; i < 40; i++) rows.push([String(100 + i), names[i % 6], `hide${i}`, ["Springvale", "Oakridge"][i % 2], `${i}.50`, i % 3 ? "Open" : "Closed"]);
  await table(tsv(rows));
  await sel(0, 2, 40, 2);
  await run("format.hideCols");
  // Rows 3-6, columns name..city (secret is hidden).
  await sel(3, 1, 6, 3);
  await openP("file.printSel");
  ok(await dlg().isVisible(), "Print the selection opens Print");
  const selRadio = dlg().locator("input[type=radio]").nth(1);
  ok(await selRadio.isChecked(), "...with the selection chosen");
  const text1 = await dlg().textContent();
  ok(/the selection \(4 rows × 2 columns\)/.test(text1), `it names the block: 4 rows × 2 columns (${(text1.match(/the selection[^)]*\)/) || [""])[0]})`);
  ok((await frame().getAttribute("sandbox")) === "", "the preview frame is fully sandboxed (sandbox=\"\")");
  let html = await srcdoc();
  ok(html.startsWith('<!doctype html><html><head><meta http-equiv="Content-Security-Policy" content="default-src \'none\'; style-src \'unsafe-inline\'">'), "the preview page starts with the content policy");
  ok(html.includes("<th>name</th><th>city</th></tr>") && !html.includes("secret") && !html.includes("hide3") && !html.includes("<th>id</th>"), "the preview shows only the block's shown columns");
  ok(html.includes(names[2]) && !html.includes(">100<") && (html.match(/<tr>/g) || []).length === 5, "...and only its rows (4 and the titles)");
  await shot("print-selection.png");
  await dlg().getByRole("button", { name: "Save as PDF…" }).click();
  await settle(600);
  const pdf = await p.evaluate(() => window.__nde.backend.lastPdf);
  ok(pdf && pdf.rows === 4 && JSON.stringify(pdf.cols) === "[1,3]", `Save as PDF gets the block's rows and columns (${JSON.stringify(pdf)})`);
  const status = await p.locator(".status, footer, [role=status]").allTextContents();
  ok(status.some((s) => /4 rows × 2 columns/.test(s)), "the status line says what was saved");

  // Whole rows: every shown column.
  await sel(2, 0, 3, 5);
  await openP("file.printSel");
  html = await srcdoc();
  ok(/the selection \(2 rows × 5 columns\)/.test(await dlg().textContent()) && html.includes("<th>id</th>") && !html.includes("secret"), "whole rows print every shown column");
  await close();

  // Several blocks: the choice is off, with a note.
  await sel(3, 1, 4, 1);
  await p.evaluate(() => (window.__nde.tab.extra = [{ row: 8, col: 3, rows: 2, cols: 1 }]));
  await openP("file.print");
  ok(await dlg().isVisible(), "Print still opens with several blocks selected");
  ok(await dlg().locator("input[type=radio]").nth(1).isDisabled(), "...the selection choice is off");
  ok(/Several blocks are selected: select one block to print it\./.test(await dlg().textContent()), "...with a note saying why");
  await close();
  await p.evaluate(() => (window.__nde.tab.extra = []));

  // ---- 2. Fit to width: the 30-column sample, laid out for real ----
  const rec = JSON.parse(fs.readFileSync(path.join(fixture, "thirty.json"), "utf8"));
  const csv = fs.readFileSync(path.join(fixture, "thirty.csv"), "utf8").trim().split("\n").map((l) => l.split(","));
  await table(tsv(csv));
  await sel(1, 0);
  await openP("file.print");
  await dlg().locator("select").nth(1).selectOption({ label: "Landscape" });
  await settle(300);
  html = await srcdoc();
  const size = Number((html.match(/body \{ font: ([\d.]+)pt\//) || [])[1]);
  // The record was made with Segoe UI (Windows); elsewhere the fonts differ.
  if (process.platform === "win32") ok(Math.abs(size - rec.pdf_size) <= 0.5, `Print's size ${size.toFixed(3)} pt is within 0.5 pt of the PDF's ${rec.pdf_size} pt`);
  else console.log(`SKIP size against the PDF's: the record is for Segoe UI on Windows (here ${size.toFixed(3)} pt)`);
  ok(new RegExp(`Text will print at ${size.toFixed(1).replace(/\.0$/, "")} pt \\(small\\)\\.`).test(await dlg().textContent()), "the dialog says the size, and that it is small (already landscape: no hint)");
  ok(html.includes("table-layout: fixed"), "shrunk text uses fixed column widths");
  await shot("print-fit-30.png");
  await close();
  // Laid out in a normal page (not the sandboxed frame).
  const lay = await ctx.newPage();
  await lay.setContent(html);
  await lay.waitForTimeout(300);
  const m = await lay.evaluate(() => {
    const px = 96 / 25.4;
    const body = getComputedStyle(document.body);
    const content = parseFloat(body.width) - parseFloat(body.paddingLeft) - parseFloat(body.paddingRight);
    const t = document.querySelector("table");
    const fontPx = parseFloat(getComputedStyle(t).fontSize);
    const one = fontPx * (1.25 + 0.3) + 2;
    const trs = [...document.querySelectorAll("tbody tr, thead tr")];
    const tallest = Math.max(...trs.map((r) => r.getBoundingClientRect().height));
    // Canvas against layout for every cell's text.
    const c = document.createElement("canvas").getContext("2d");
    let worst = 0;
    for (const cell of document.querySelectorAll("td, th")) {
      if (!cell.textContent) continue;
      const cs = getComputedStyle(cell);
      c.font = `${cs.fontWeight} ${cs.fontSize} ${cs.fontFamily}`;
      const r = document.createRange();
      r.selectNodeContents(cell);
      const laid = r.getBoundingClientRect().width;
      if (laid > 4) worst = Math.max(worst, Math.abs(c.measureText(cell.textContent).width - laid) / laid);
    }
    const wrapped = [...document.querySelectorAll("td, th")].filter((x) => {
      const r = document.createRange();
      r.selectNodeContents(x);
      return new Set([...r.getClientRects()].map((q) => Math.round(q.top))).size > 1;
    }).slice(0, 3).map((x) => {
      c.font = `${getComputedStyle(x).fontWeight} ${getComputedStyle(x).fontSize} ${getComputedStyle(x).fontFamily}`;
      return `${x.tagName} "${x.textContent}" cell ${x.getBoundingClientRect().width.toFixed(1)} canvas ${c.measureText(x.textContent).width.toFixed(1)}`;
    });
    return { content, table: t.getBoundingClientRect().width, tallest, one, worst, px, wrapped };
  });
  ok(m.table <= m.content + 0.5, `the table (${m.table.toFixed(1)} px) fits the page's content width (${m.content.toFixed(1)} px)`);
  ok(m.tallest <= m.one + 1, `no cell wraps: the tallest row is ${m.tallest.toFixed(1)} px, one line is ${m.one.toFixed(1)} px ${m.wrapped.join("; ")}`);
  ok(m.worst <= 0.03, `canvas widths are within 3% of the laid-out ones (worst ${(m.worst * 100).toFixed(2)}%)`);

  // ---- 3. A long unbroken value in a narrow column stays in its cell ----
  const email = "averyashdown.quarterly.reports.springvale@example-mail.test";
  ok(email.length >= 59, `the email is ${email.length} characters`);
  const narrow = [["id", "email", "next", "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q"]];
  for (let i = 0; i < 6; i++) narrow.push([String(i), i === 2 ? email : "x", "NEXT", ...Array.from({ length: 17 }, () => "some words here")]);
  await table(tsv(narrow));
  await sel(1, 0);
  await openP("file.print");
  html = await srcdoc();
  await close();
  await lay.setContent(html);
  await lay.waitForTimeout(300);
  const e = await lay.evaluate(() => {
    const row = document.querySelectorAll("tbody tr")[2];
    const [, mail, next] = row.querySelectorAll("td");
    const a = mail.getBoundingClientRect();
    const r = document.createRange();
    r.selectNodeContents(mail);
    const rects = [...r.getClientRects()];
    return { right: Math.max(...rects.map((x) => x.right)), cellRight: a.right, nextLeft: next.getBoundingClientRect().left, nextText: next.textContent, lines: rects.length, colW: a.width };
  });
  ok(e.right <= e.cellRight + 0.5 && e.right <= e.nextLeft + 0.5, `the email breaks inside its own column (text ends ${e.right.toFixed(1)}, cell ${e.cellRight.toFixed(1)}, ${e.lines} lines, column ${e.colW.toFixed(0)} px)`);
  ok(e.nextText === "NEXT", "the next cell's text is untouched");

  // ---- 4. Fit on one page ----
  const forty = [["id", "name", "city", "amount"]];
  for (let i = 0; i < 40; i++) forty.push([String(i + 1), names[i % 6], "Oakridge", `${i}.25`]);
  await table(tsv(forty));
  await sel(1, 0);
  await openP("file.print");
  await dlg().locator("label:has-text('Text size') input").fill("24");
  await dlg().locator("select[aria-label=Fit]").selectOption("page");
  await settle(300);
  html = await srcdoc();
  const t4 = await dlg().textContent();
  const s4 = Number((html.match(/body \{ font: ([\d.]+)pt\//) || [])[1]);
  ok(s4 < 24 && s4 > 4, `40 rows at 24 pt shrink to ${s4} pt for one page`);
  ok(/Text will print at [\d.]+ pt\./.test(t4) && !/pages even at 4 pt/.test(t4), "the dialog says the size and no extra pages");
  await shot("print-one-page.png");
  await close();
  await lay.setContent(html);
  await lay.waitForTimeout(300);
  const h4 = await lay.evaluate(() => {
    const body = getComputedStyle(document.body);
    const content = parseFloat(body.height) - parseFloat(body.paddingTop) - parseFloat(body.paddingBottom);
    return { content, table: document.querySelector("table").getBoundingClientRect().height };
  });
  ok(h4.table <= h4.content, `the 40-row table (${h4.table.toFixed(0)} px high) fits one page's content (${h4.content.toFixed(0)} px)`);
  // A value held to 40% of the page and then shrunk no longer fits its
  // column: on one page it is cut with "…", so the table still fits.
  const capped = [["id", "name", "note"]];
  for (let i = 0; i < 40; i++) capped.push([String(i + 1), names[i % 6], "a fairly long note about the visit and what came of it"]);
  await table(tsv(capped));
  await sel(1, 0);
  await openP("file.print");
  await dlg().locator("label:has-text('Text size') input").fill("24");
  await dlg().locator("select[aria-label=Fit]").selectOption("page");
  await settle(300);
  html = await srcdoc();
  await close();
  await lay.setContent(html);
  await lay.waitForTimeout(300);
  const h5 = await lay.evaluate(() => {
    const body = getComputedStyle(document.body);
    const content = parseFloat(body.height) - parseFloat(body.paddingTop) - parseFloat(body.paddingBottom);
    const note = document.querySelectorAll("tbody tr")[0].querySelectorAll("td")[2];
    return { content, table: document.querySelector("table").getBoundingClientRect().height, cut: note.scrollWidth > note.clientWidth };
  });
  ok(h5.cut, "the long note is held to its column (cut with …, not wrapped)");
  ok(h5.table <= h5.content, `with values at the 40% cap the 40-row table (${h5.table.toFixed(0)} px high) still fits one page (${h5.content.toFixed(0)} px)`);
  const many = [["id", "name", "note"]];
  for (let i = 0; i < 2000; i++) many.push([String(i + 1), names[i % 6], "ok"]);
  // Far down (past the 60 rows the dialog opens on), very long values with
  // no spaces, wider than the page: the widths measured just after opening
  // take it in, as the PDF would.
  many[1500] = ["L".repeat(160), "Long".repeat(40), "Wide".repeat(40)];
  await table(tsv(many));
  await sel(1, 0);
  await openP("file.print");
  await settle(500);
  ok(/Text will print at [\d.]+ pt/.test(await dlg().textContent()), "widths from beyond the first 60 rows arrive just after the dialog opens (row 1,500's long value shrinks the text)");
  await dlg().locator("select[aria-label=Fit]").selectOption("page");
  await settle(300);
  ok(/This needs \d+ pages even at 4 pt; it will print on \d+ pages\./.test(await dlg().textContent()), "2,000 rows: the dialog says how many pages it needs even at 4 pt");
  await dlg().getByRole("button", { name: "Save as PDF…" }).click();
  await settle(600);
  ok((await p.evaluate(() => window.__nde.backend.lastPdf)).fit_page === true, "Save as PDF is asked to fit on one page");

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  await lay.close();
  await b.close();
  console.log(failed ? `${failed} FAILED` : "ALL PASSED");
  process.exit(failed ? 1 : 0);
})();
