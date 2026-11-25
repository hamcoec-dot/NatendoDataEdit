// Drives the REAL built app (not the browser preview) over WebView2 remote debugging to check what the
// import keeps exactly (A1a): an .xlsx with whole numbers of 18-19 digits, an exponent-form number, and a zip
// column in a zero-padding format (00000); a JSON file with 1.50 and a 23-digit number. It checks the line
// under the options, the one checkbox, the cells after the import with and without it, that the answer is
// remembered for the file next time, and the "last import" command.
//
// Usage (after `npx tauri build`):
//   NODE_PATH=<folder with playwright> node scripts/real-import.cjs <exe> <evidence folder>
// The app's own data (session.json, recent.json, private.json, recovery copies) is backed up and put back; the
// app runs with its own browser profile under the evidence folder. Files are made-up and live in
// <evidence>/work. Python 3 (zipfile) writes the .xlsx. Nothing is installed.
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-import.cjs <exe> <evidence folder>");
  process.exit(2);
}
const WORK = path.join(evidence, "work");
fs.mkdirSync(WORK, { recursive: true });
const DATA = path.join(process.env.APPDATA, "com.natendo.dataedit");
const LOCAL = path.join(process.env.LOCALAPPDATA, "com.natendo.dataedit");
const PORT = 9371;
const T0 = Date.now();
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let failed = 0;
const say = (m) => console.log(`[${((Date.now() - T0) / 1000).toFixed(1)}s] ${m}`);
const ok = (cond, msg) => {
  if (!cond) failed++;
  say(`${cond ? "PASS" : "FAIL"} ${msg}`);
};
const info = (m) => say(`INFO ${m}`);
const hash = (p) => (fs.existsSync(p) ? crypto.createHash("sha256").update(fs.readFileSync(p)).digest("hex") : null);

// ---- the app's own data ----
const BACKUP = path.join(WORK, "backup");
const backup = () => {
  fs.mkdirSync(BACKUP, { recursive: true });
  for (const f of ["session.json", "recent.json"]) if (fs.existsSync(path.join(DATA, f))) fs.copyFileSync(path.join(DATA, f), path.join(BACKUP, f));
  fs.rmSync(path.join(BACKUP, "no-private-json"), { force: true });
  if (!fs.existsSync(path.join(DATA, "private.json"))) fs.writeFileSync(path.join(BACKUP, "no-private-json"), "");
  else fs.copyFileSync(path.join(DATA, "private.json"), path.join(BACKUP, "private.json"));
  const rec = path.join(LOCAL, "recovery");
  const aside = path.join(BACKUP, "recovery");
  if (fs.existsSync(rec) && !fs.existsSync(aside)) fs.renameSync(rec, aside);
};
// Nothing is deleted: copies left by the test's own stopped apps are moved into the backup folder.
const wipeRecovery = () => {
  const rec = path.join(LOCAL, "recovery");
  if (fs.existsSync(rec)) fs.renameSync(rec, path.join(BACKUP, `recovery-run-${Date.now()}`));
};
const restore = () => {
  const pj = path.join(DATA, "private.json");
  if (fs.existsSync(path.join(BACKUP, "no-private-json"))) {
    if (fs.existsSync(pj)) fs.renameSync(pj, path.join(BACKUP, `private.from-run-${Date.now()}.json`));
  } else if (fs.existsSync(path.join(BACKUP, "private.json"))) fs.copyFileSync(path.join(BACKUP, "private.json"), pj);
  wipeRecovery();
  const aside = path.join(BACKUP, "recovery");
  if (fs.existsSync(aside)) fs.renameSync(aside, path.join(LOCAL, "recovery"));
  for (const f of ["session.json", "recent.json"]) {
    const b = path.join(BACKUP, f);
    if (fs.existsSync(b)) fs.copyFileSync(b, path.join(DATA, f));
  }
  const same = ["session.json", "recent.json"].every((f) => hash(path.join(BACKUP, f)) === hash(path.join(DATA, f)));
  say(`the app's session.json and recent.json restored byte for byte: ${same}`);
};

// ---- the app ----
let browser = null;
let ctx = null;
// Only the instance this script started is ever stopped (never another one: a person's own running app is left alone).
let appPid = null;
const running = () => spawnSync("powershell", ["-NoProfile", "-Command", "(Get-Process -Name natendo-data-edit -ErrorAction SilentlyContinue | Measure-Object).Count"], { encoding: "utf8" }).stdout.trim() !== "0";
const alive = () => {
  if (appPid === null) return false;
  try {
    process.kill(appPid, 0);
    return true;
  } catch {
    return false;
  }
};
const killApp = async () => {
  if (browser) await browser.close().catch(() => {});
  browser = ctx = null;
  if (appPid !== null && alive()) spawnSync("taskkill", ["/PID", String(appPid), "/T", "/F"], { encoding: "utf8" });
  for (let i = 0; i < 20 && alive(); i++) await sleep(250);
  appPid = null;
};
const pageMain = () => ctx.pages().find((p) => !p.url().includes("window=2"));
const dismiss = async (p) => {
  const notNow = p.getByRole("button", { name: "Not now" });
  if (await notNow.isVisible().catch(() => false)) await notNow.click();
  await sleep(300);
  await p.locator(".header-ask button.close").click({ timeout: 800 }).catch(() => {});
};
const start = async (files) => {
  await killApp();
  wipeRecovery();
  const pj = path.join(DATA, "private.json");
  if (fs.existsSync(pj) && fs.existsSync(path.join(BACKUP, "no-private-json"))) fs.renameSync(pj, path.join(BACKUP, `private.from-run-${Date.now()}.json`));
  if (running()) throw new Error("another Natendo Data Edit is running: close it first (this test never stops an app it did not start)");
  const child = spawn(exe, files, {
    env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT}`, WEBVIEW2_USER_DATA_FOLDER: path.join(WORK, "wv2") },
    stdio: "ignore",
    detached: true,
  });
  appPid = child.pid;
  child.unref();
  for (let i = 0; i < 60; i++) {
    try {
      if ((await fetch(`http://localhost:${PORT}/json/version`)).ok) break;
    } catch {}
    await sleep(500);
  }
  browser = await chromium.connectOverCDP(`http://localhost:${PORT}`);
  ctx = browser.contexts()[0];
  const m = pageMain();
  await m.waitForSelector("canvas, .start, button", { timeout: 30000 });
  await sleep(1500);
  await dismiss(m);
  return m;
};
const invoke = (page, cmd, args) => page.evaluate(([c, a]) => window.__TAURI_INTERNALS__.invoke(c, a), [cmd, args || {}]);
const cmd = async (page, q) => {
  await page.keyboard.press("Alt+q");
  await page.keyboard.type(q);
  await sleep(350);
  await page.keyboard.press("Enter");
  await sleep(600);
};
const waitFor = async (fn, ms = 15000, step = 250) => {
  const end = Date.now() + ms;
  for (;;) {
    try {
      const v = await fn();
      if (v) return v;
    } catch {}
    if (Date.now() > end) return false;
    await sleep(step);
  }
};

// ---- made-up files ----
const mkXlsx = () => {
  const p = path.join(WORK, "ids.xlsx");
  const py = [
    "import zipfile,sys",
    "p=sys.argv[1]",
    "z=zipfile.ZipFile(p,'w',zipfile.ZIP_DEFLATED)",
    "ns='http://schemas.openxmlformats.org/officeDocument/2006/relationships'",
    "z.writestr('_rels/.rels','<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"'+ns+'/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>')",
    "z.writestr('xl/workbook.xml','<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"'+ns+'\"><sheets><sheet name=\"Data\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>')",
    "z.writestr('xl/_rels/workbook.xml.rels','<Relationships><Relationship Id=\"rId1\" Type=\"'+ns+'/worksheet\" Target=\"worksheets/sheet1.xml\"/><Relationship Id=\"rId2\" Type=\"'+ns+'/styles\" Target=\"styles.xml\"/></Relationships>')",
    "z.writestr('xl/styles.xml','<styleSheet><numFmts><numFmt numFmtId=\"164\" formatCode=\"00000\"/></numFmts><cellXfs><xf numFmtId=\"0\"/><xf numFmtId=\"164\"/></cellXfs></styleSheet>')",
    "def t(c,v): return '<c r=\"%s\" t=\"inlineStr\"><is><t>%s</t></is></c>'%(c,v)",
    "s='<worksheet><sheetData><row r=\"1\">'+t('A1','Account')+t('B1','Zip')+t('C1','Rate')+t('D1','Old')+'</row>'",
    "data=[('1234567890123456789','2134','1.5','1.2345678901234567E+18'),('9876543210987654321','90210','2.25','5'),('1000000000000000001','501','3','7')]",
    "for i,(a,b,c,d) in enumerate(data,start=2):",
    "  s+='<row r=\"%d\"><c r=\"A%d\"><v>%s</v></c><c r=\"B%d\" s=\"1\"><v>%s</v></c><c r=\"C%d\"><v>%s</v></c><c r=\"D%d\"><v>%s</v></c></row>'%(i,i,a,i,b,i,c,i,d)",
    "s+='</sheetData></worksheet>'",
    "z.writestr('xl/worksheets/sheet1.xml',s)",
    "z.close()",
  ].join("\n");
  const r = spawnSync("python", ["-c", py, p], { encoding: "utf8" });
  if (r.status !== 0) throw new Error("python could not write the xlsx: " + r.stderr);
  return p;
};
const XLSX = mkXlsx();
const JSONF = path.join(WORK, "rows.json");
fs.writeFileSync(JSONF, '[{"id":12345678901234567890123,"price":1.50,"name":"Ada"},{"id":-0.10,"price":2.0,"name":"Bo"}]');

/** The text of a cell of tab `doc` (a fresh app: the first import is tab 1), through the app's own command. */
const cellOf = async (p, doc, row, col) => (await invoke(p, "cell_text", { doc, row, col })).text;
const importDialog = (p) => p.locator(".shell[role=dialog]");
const doImport = async (m) => {
  await m.getByRole("button", { name: "Import", exact: true }).click();
  await sleep(3000);
};
/** Wait for the import screen of an .xlsx; tick or untick the question when `want` is true/false; what the screen shows. */
const xlsxScreen = async (m, want) => {
  await importDialog(m).waitFor({ timeout: 25000 });
  await waitFor(async () => (await m.locator("[data-testid=import-notes]").count()) > 0, 20000);
  await sleep(500);
  const note = ((await m.locator("[data-testid=import-notes]").textContent()) || "").replace(/\s+/g, " ").trim();
  const box = m.getByLabel(/Write numbers with leading zeros/);
  const shown = await box.count();
  let checked = null;
  if (shown) {
    checked = await box.isChecked();
    if (want !== null && want !== checked) {
      await box.click();
      await sleep(1200);
      checked = await box.isChecked();
    }
  }
  const noteAfter = ((await m.locator("[data-testid=import-notes]").textContent().catch(() => "")) || "").replace(/\s+/g, " ").trim();
  return { note, noteAfter, shown, checked };
};

(async () => {
  backup();
  try {
    // 1. xlsx, the default answer: numbers stay numbers, long ones exact.
    let m = await start([XLSX]);
    let r = await xlsxScreen(m, null);
    info(`import screen line: ${r.note}`);
    ok(r.shown === 1, "the import screen asks the zero-padding question once");
    ok(r.checked === false, "the question starts unticked on a first open");
    ok(/kept exactly/.test(r.note) && /“Account”/.test(r.note), "the line names the column of the long numbers (counts and column names only)");
    ok(/had digits the file no longer holds/.test(r.note), "the line says plainly that the exponent-form number's digits are gone");
    ok(!/1234567890123456789/.test(r.note) && !/2134/.test(r.note), "the line holds no cell values");
    await doImport(m);
    ok((await cellOf(m, 1, 1, 0)) === "1234567890123456789", `18-digit account number kept exactly (got ${await cellOf(m, 1, 1, 0)})`);
    ok((await cellOf(m, 1, 2, 0)) === "9876543210987654321", "second 19-digit number kept exactly");
    ok((await cellOf(m, 1, 1, 1)) === "2134", `zip stays the number without the answer (got ${await cellOf(m, 1, 1, 1)})`);
    ok((await cellOf(m, 1, 1, 3)) === "1.23456789012346E+18", `exponent-form number shown as before (got ${await cellOf(m, 1, 1, 3)})`);
    await cmd(m, "Show what the last import kept exactly");
    const status = (await m.locator(".statusbar, .status, [role=status]").allTextContents().catch(() => [])).join(" ");
    ok(/kept exactly/.test(status), "the last-import command says what was kept in the status bar");

    // 2. the same file again, ticking the box: padded.
    await killApp();
    m = await start([XLSX]);
    r = await xlsxScreen(m, true);
    ok(r.checked === true, "the question can be ticked");
    ok(/written with leading zeros/.test(r.noteAfter), "the line then says how many numbers were written padded");
    await doImport(m);
    ok((await cellOf(m, 1, 1, 1)) === "02134", `zip written padded when asked (got ${await cellOf(m, 1, 1, 1)})`);
    ok((await cellOf(m, 1, 2, 1)) === "90210", "a five-digit zip is left as it is");
    ok((await cellOf(m, 1, 3, 1)) === "00501", "a short zip is padded");
    ok((await cellOf(m, 1, 1, 0)) === "1234567890123456789", "the long account number is still exact");

    // 3. next time the answer is remembered for the file.
    await killApp();
    m = await start([XLSX]);
    r = await xlsxScreen(m, null);
    ok(r.checked === true, "the answer is remembered for this file (box already ticked)");
    const keys = await m.evaluate(() => Object.entries(localStorage).filter(([k]) => k.startsWith("nde.padzeros")));
    info(`stored: ${JSON.stringify(keys).slice(0, 200)}`);
    ok(keys.length === 1 && !/ids\.xlsx|2134|Account/i.test(JSON.stringify(keys)), "only a salted fingerprint and yes/no are stored (no path, no values)");
    await m.getByRole("button", { name: "Cancel", exact: true }).click();

    // 4. JSON
    await killApp();
    m = await start([JSONF]);
    await importDialog(m).waitFor({ timeout: 25000 });
    await sleep(1000);
    await doImport(m);
    ok((await cellOf(m, 1, 1, 0)) === "12345678901234567890123", `JSON 23-digit number exact (got ${await cellOf(m, 1, 1, 0)})`);
    ok((await cellOf(m, 1, 1, 1)) === "1.50", `JSON 1.50 keeps its zero (got ${await cellOf(m, 1, 1, 1)})`);
    ok((await cellOf(m, 1, 2, 0)) === "-0.10", `JSON -0.10 exact (got ${await cellOf(m, 1, 2, 0)})`);
  } catch (e) {
    failed++;
    say(`FAIL unexpected: ${e && e.stack ? e.stack : e}`);
  } finally {
    await killApp();
    restore();
    say(failed ? `${failed} FAILED` : "ALL PASSED");
    process.exit(failed ? 1 : 0);
  }
})();
