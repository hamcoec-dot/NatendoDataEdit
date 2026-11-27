// UI test of saving JSON back (C11) against the vite dev server (browser stand-in backend): Ctrl+S on a
// tab from a JSON Lines file saves back with no question when nothing is newly blank; a newly blank cell
// asks once (leave the key out preselected) and the answer is remembered for the file; another
// program's change stops the save until agreed; Save As offers JSON Lines and JSON (and from a CSV tab
// writes a copy); a file that cannot be saved back makes Save into Save As with the reason and JSON
// greyed; titles used twice are refused with the reason; Alt+Q finds the feature.
// Run: (cd ui && npx vite --port 5321) then
// NODE_PATH=<folder with playwright> node scripts/ui-jsonl.cjs [screenshot dir] [url]
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
  const status = async () => ((await p.textContent("footer.status")) || "").replace(/\s+/g, " ");
  const calls = () => p.evaluate(() => window.__nde.backend.calls.filter((c) => c.startsWith("saveJson")));
  const dialog = () => p.$('[role="dialog"], [role="alertdialog"]');
  const dialogText = async () => ((await (await dialog())?.textContent()) || "").replace(/\s+/g, " ");
  const open = (name, rows, o) =>
    p.evaluate(
      ([name, rows, o]) => {
        const n = window.__nde;
        n.addTab(n.backend.mockJsonTab(name, rows, o));
      },
      [name, rows, o],
    );
  const edit = (cells) =>
    p.evaluate(async (cells) => {
      const t = window.__nde.tab;
      t.update(await window.__nde.backend.setCells(t.id, cells), true);
    }, cells);
  const save = async (keys = "Control+s") => {
    await p.evaluate(() => window.__nde.grid?.focus());
    await p.keyboard.press(keys);
    await p.waitForTimeout(500);
  };

  // ---- a JSON Lines tab: no edits, then saving back with no question ----
  await open("people.jsonl", [["id", "name", "amount"], ["1", "Ann", "10"], ["2", "Bo", "7"]], {});
  await p.waitForTimeout(400);
  let info = await p.evaluate(() => window.__nde.tab.info);
  ok(info.json_format === "jsonl" && info.name === "people", `the tab knows its JSON Lines file (${info.json_format}, ${info.name})`);
  await edit([[1, 1, "Anne"]]);
  await save();
  ok(!(await dialog()), "a save with nothing newly blank asks nothing");
  let st = await status();
  ok(/Saved 2 records as JSON Lines\./.test(st), `the status line counts the records (${st})`);
  ok((await calls()).at(-1) === "saveJson:back:lines:leave_out", `it saved back, not Save As (${(await calls()).at(-1)})`);
  let written = await p.evaluate(() => window.__nde.backend.mockJsonWritten(window.__nde.tab.id));
  ok(written.split("\n")[0] === '{"id":1,"name":"Anne","amount":10}', `the file has the edit, numbers as numbers (${written.split("\n")[0]})`);
  ok(!(await p.evaluate(() => window.__nde.tab.info.modified)), "and the tab is saved");

  // ---- a newly blank cell: asked once, leave out preselected, remembered ----
  await edit([[2, 2, ""]]);
  await save();
  let dt = await dialogText();
  ok(/Blank cells in JSON/.test(dt), `a newly blank cell asks how blanks are written (${dt.slice(0, 60)})`);
  const pre = await p.evaluate(() => document.querySelector('input[name="json-blanks"]:checked')?.value);
  ok(pre === "leave_out", `leave the key out is preselected (${pre})`);
  if (out) await p.screenshot({ path: path.join(out, "json-blanks.png") });
  await p.check('input[name="json-blanks"][value="null"]');
  await p.click('[role="dialog"] button.primary');
  await p.waitForTimeout(500);
  written = await p.evaluate(() => window.__nde.backend.mockJsonWritten(window.__nde.tab.id));
  ok(written.split("\n")[1] === '{"id":2,"name":"Bo","amount":null}', `the answer is used (${written.split("\n")[1]})`);
  await edit([[1, 2, ""]]);
  await save();
  ok(!(await dialog()), "the next save does not ask again");
  st = await status();
  ok(/written as null, as remembered for this file/.test(st), `and says the remembered answer was used (${st})`);

  // ---- another program changed the file ----
  await edit([[1, 1, "Ann"]]);
  await p.evaluate(() => window.__nde.backend.mockJsonChanged(window.__nde.tab.id));
  await save();
  dt = await dialogText();
  ok(/The file changed on disk/.test(dt), `an outside change stops the save (${dt.slice(0, 40)})`);
  await p.click('button:has-text("Replace their changes")');
  await p.waitForTimeout(500);
  ok((await calls()).length >= 4 && /Saved 2 records/.test(await status()), "replacing them saves");

  // ---- Save As: JSON Lines preselected, JSON offered ----
  await save("F12");
  dt = await dialogText();
  const kind = await p.evaluate(() => document.querySelector('select[aria-label="File type"]')?.value);
  ok(kind === "jsonl", `Save As starts on the tab's own format (${kind})`);
  ok(/keep the kind they had in the JSON file/.test(dt), "and says values keep their kind");
  if (out) await p.screenshot({ path: path.join(out, "json-saveas.png") });
  await p.selectOption('select[aria-label="File type"]', "json");
  await p.click('[role="dialog"] button.primary');
  await p.waitForTimeout(600);
  ok((await calls()).at(-1) === "saveJson:as:pretty:leave_out", `JSON over several lines is written (${(await calls()).at(-1)})`);
  st = await status();
  ok(/as JSON\./.test(st), `the status says JSON (${st})`);
  info = await p.evaluate(() => window.__nde.tab.info);
  ok(info.json_format === "json", "the tab now saves back to the JSON file");

  // ---- a file that cannot be saved back ----
  await open("twice.jsonl", [["id", "v"], ["1", "a"]], { refuse: "record 2 has the key 'id' twice, so a value would be lost" });
  await p.waitForTimeout(400);
  await save();
  dt = await dialogText();
  ok(/Save As options/.test(dt) && /JSON is greyed out\..*twice/.test(dt), `Save is Save As, with JSON greyed and the reason (${dt.slice(0, 120)})`);
  const greyed = await p.evaluate(() => [...document.querySelectorAll('select[aria-label="File type"] option')].filter((o) => o.disabled).length);
  ok(greyed === 3, `the three JSON entries are greyed (${greyed})`);
  st = await status();
  ok(/only be saved as another file: record 2 has the key 'id' twice/.test(st), `the status line says why (${st})`);
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);

  // ---- titles used twice ----
  await open("dup.jsonl", [["a", "a"], ["1", "2"]], {});
  await p.waitForTimeout(400);
  await save();
  dt = await dialogText();
  ok(/This cannot be saved as JSON/.test(dt) && /both called 'a'/.test(dt), `titles used twice are refused with the reason (${dt.slice(0, 90)})`);
  await p.click('button:has-text("Save As delimited text")');
  await p.waitForTimeout(500);
  ok(/Save As options/.test(await dialogText()) && !(await p.$('select[aria-label="File type"]')), "Save As delimited text offers no JSON this time");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);

  // ---- a CSV tab: Save As JSON Lines writes a copy ----
  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => !/Loading|Indexing|Reading/i.test(document.querySelector("footer.status")?.textContent || ""), null, { timeout: 30000 }).catch(() => {});
  await p.waitForTimeout(800);
  const csvName = await p.evaluate(() => window.__nde.tab.info.name);
  await save("F12");
  const k2 = await p.evaluate(() => document.querySelector('select[aria-label="File type"]')?.value);
  ok(k2 === "text", `a CSV tab's Save As starts on delimited text (${k2})`);
  await p.selectOption('select[aria-label="File type"]', "jsonl");
  ok(/Every value is written as text/.test(await dialogText()), "and says every value is written as text");
  await p.click('[role="dialog"] button.primary');
  await p.waitForTimeout(600);
  st = await status();
  ok((await calls()).at(-1) === "saveJson:as:lines:leave_out" && /a copy/.test(st), `it writes a copy (${st})`);
  ok((await p.evaluate(() => window.__nde.tab.info.name)) === csvName, "and the tab keeps its own file");

  // ---- Alt+Q ----
  await p.evaluate(() => window.__nde.grid?.focus());
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("save json back");
  await p.waitForTimeout(500);
  const found = await p.evaluate(() => document.body.innerText);
  const top = found.indexOf("Ctrl+S, Shift+F12");
  ok(top >= 0 && found.indexOf("Save As", top) > 0 && /temporary file like every save/.test(found.slice(top, top + 400)), "Alt+Q lists Save and Save As first, with the JSON save-back line");
  await p.keyboard.press("Escape");

  ok(errors.length === 0, `no page errors (${errors.join("; ")})`);
  await b.close();
  console.log(failed ? `${failed} FAILED` : "ALL PASSED");
  process.exit(failed ? 1 : 0);
})();
