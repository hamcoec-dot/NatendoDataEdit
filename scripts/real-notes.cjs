// Drives the REAL built app to check report notes (C4a): Headers start here, Set aside rows as footer notes, the
// Report notes panel, a save that writes the notes back byte for byte, the keep switch and its question, and a
// recovery copy that holds every line (then opens with the notes after a simulated crash).
// The app's own session, recent-file and recovery data are set aside and put back. Made-up data only.
// Usage: NODE_PATH=<playwright> node scripts/real-notes.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-notes.cjs <exe> <evidence folder>");
  process.exit(2);
}
const WORK = path.join(evidence, "work");
fs.mkdirSync(WORK, { recursive: true });
const DATA = path.join(process.env.APPDATA, "com.natendo.dataedit");
const LOCAL = path.join(process.env.LOCALAPPDATA, "com.natendo.dataedit");
const PORT = 9389;
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
const dismiss = async (p) => {
  const notNow = p.getByRole("button", { name: "Not now" });
  if (await notNow.isVisible().catch(() => false)) await notNow.click();
  await sleep(300);
  await p.locator(".header-ask button.close").click({ timeout: 800 }).catch(() => {});
};
const start = async (files) => {
  await killApp();
  wipeRecovery();
  if (running()) throw new Error("another Natendo Data Edit is running: close it first (this test never stops an app it did not start)");
  const child = spawn(exe, files, {
    env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT}`, WEBVIEW2_USER_DATA_FOLDER: path.join(WORK, `wv2-${Date.now()}`) },
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
  const m = ctx.pages().find((p) => !p.url().includes("window=2"));
  await m.waitForSelector("canvas, .start, button", { timeout: 30000 });
  await sleep(1500);
  await dismiss(m);
  return m;
};
const invoke = (page, cmd, args) => page.evaluate(([c, a]) => window.__TAURI_INTERNALS__.invoke(c, a).catch((e) => { throw new Error(typeof e === "string" ? e : JSON.stringify(e)); }), [cmd, args || {}]);
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

const OUT = path.join(WORK, `out-${Date.now()}`);
fs.mkdirSync(OUT, { recursive: true });
const FILE = path.join(OUT, "shop-report.csv");
const LINES = ["Sales by shop", "Printed: 2026-05-01", "", "Region: North", "shop,manager,month,amount", "Harbour St,Ada Lind,May,1520.40", "Mill Lane,Bo Varga,May,980.10", "Old Quay,Cy Okafor,May,1210.00", "Station Rd,Dee Rivera,May,760.25", "Park Row,Eli Stone,May,1302.75", "Total,,,5773.50", ""];
const BODY = LINES.join("\r\n");
const TABLE = LINES.slice(4, 10).join("\r\n") + "\r\n";
const fresh = () => fs.writeFileSync(FILE, BODY);
const read = () => fs.readFileSync(FILE, "utf8");
const status = async (m) => ((await m.textContent("footer.status")) || "").replace(/\s+/g, " ");
const body = async (m) => ((await m.textContent("body")) || "").replace(/\s+/g, " ");
const save = (m) => invoke(m, "save", { doc: 1, saveAs: false, overwriteChanged: true, allowLossy: false });
const facts = (m, doc = 1) => invoke(m, "doc_info", { doc });

/** Open the file and set its title lines aside above the table and the total line below (not saved). */
const setAside = async (m) => {
  await waitFor(async () => (await facts(m)).indexed, 60000, 300);
  await sleep(1200);
  await dismiss(m);
  let f = await facts(m);
  info(`opened: ${f.rows} rows, ${f.preamble_lines} lines above the table, notes ${JSON.stringify(f.notes)}`);
  if (f.notes.above_file > 0) {
    // Detection already kept the title lines above the table: put them back first, so the test sets them aside itself.
    await cmd(m, "Put the report notes back");
    f = await facts(m);
    ok(f.notes.above_file === 0 && f.rows === 11, `put back: the title lines are rows again (${f.rows})`);
  }
  // The cursor on the column-titles row (row 5).
  await m.keyboard.press("Control+Home");
  for (let i = 0; i < 4; i++) await m.keyboard.press("ArrowDown");
  await sleep(500);
  await cmd(m, "Headers start here");
  f = await facts(m);
  ok(f.notes.above_set === 4 && f.rows === 7, `Headers start here: 4 lines above, ${f.rows} rows`);
  ok(/Set 4 rows aside above the table as report notes/.test(await status(m)), `the status says so: ${(await status(m)).slice(0, 90)}`);
  // The total line: the last row.
  await m.keyboard.press("Control+End");
  await m.keyboard.press("Home");
  await sleep(600);
  await cmd(m, "Set aside rows as footer notes");
  f = await facts(m);
  ok(f.notes.below_set === 1 && f.rows === 6, `footer: 1 line below, ${f.rows} rows`);
};

/** Start the app with no file, leaving the recovery folder as it is (start() moves it aside). */
const startKeepingCopies = async () => {
  const child = spawn(exe, [], {
    env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT}`, WEBVIEW2_USER_DATA_FOLDER: path.join(WORK, `wv2-${Date.now()}`) },
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
  const pg = ctx.pages().find((p) => !p.url().includes("window=2"));
  await pg.waitForSelector("canvas, .start, button", { timeout: 30000 });
  await sleep(1500);
  return pg;
};

const proof = async () => {
  fresh();
  let m = await start([FILE]);
  await setAside(m);
  await cmd(m, "Report notes (lines above");
  await sleep(800);
  const b = await body(m);
  ok(/Set aside above the table \(4 lines\)/.test(b) && /Set aside below the table \(1 line\)/.test(b), "the panel shows both parts");
  ok(/Region\s*North/.test(b), "report info lists the label line");
  await m.screenshot({ path: path.join(evidence, "report-notes.png") });
  await save(m);
  ok(read() === BODY, `saved byte for byte with the notes where they were: ${JSON.stringify(read().slice(0, 60))}`);
  let f = await facts(m);
  ok(f.rows === 6 && f.notes.above_file === 4 && f.notes.below_file === 1, `after the save the notes are still beside the table (${JSON.stringify(f.notes)})`);
  // An edit and a second save keep them in place.
  await invoke(m, "set_cells", { doc: 1, cells: [[2, 3, "999.99"]] });
  await save(m);
  ok(read() === BODY.replace("980.10", "999.99"), "an edit, saved again: notes unchanged");
  // The keep switch asks first, with the real counts; off and on again loses nothing.
  await m.getByLabel("Save report notes with the file").click();
  await sleep(500);
  ok(/Leave out the report notes when saving: the 4 lines above the table when the file was opened and the 1 line below the table in the file/.test(await body(m)), "the question names what goes, with the counts");
  await m.getByRole("button", { name: "Leave them out" }).click();
  await sleep(500);
  ok((await facts(m)).notes.keep === false, "switched off");
  await m.getByLabel("Save report notes with the file").click();
  await sleep(500);
  ok((await facts(m)).notes.keep === true, "back on (no question)");
  await save(m);
  ok(read() === BODY.replace("980.10", "999.99"), "off and on again: the save still has every line");
  await m.getByLabel("Save report notes with the file").click();
  await sleep(500);
  await m.getByRole("button", { name: "Leave them out" }).click();
  await sleep(500);
  await save(m);
  ok(read() === TABLE.replace("980.10", "999.99"), `switched off: the save holds the table only: ${JSON.stringify(read().slice(0, 40))}`);

  // A recovery copy holds every line, and opens with the notes after a crash.
  fresh();
  m = await start([FILE]);
  await setAside(m);
  const r = await invoke(m, "autosave", { doc: 1 });
  info(`autosave: ${JSON.stringify(r)}`);
  const rec = path.join(LOCAL, "recovery");
  const copies = fs.existsSync(rec) ? fs.readdirSync(rec).filter((n) => n.endsWith(".ndrec")).map((n) => path.join(rec, n)) : [];
  ok(copies.length === 1 && fs.readFileSync(copies[0], "utf8") === BODY, `the recovery copy is the whole file, notes included (${copies.length})`);
  // Simulated crash: the app is stopped without saving, then started again.
  if (appPid !== null) spawnSync("taskkill", ["/PID", String(appPid), "/T", "/F"], { encoding: "utf8" });
  await sleep(1500);
  if (browser) await browser.close().catch(() => {});
  browser = ctx = null;
  appPid = null;
  m = await startKeepingCopies();
  const list = await invoke(m, "recovery_list", {});
  info(`recovery list: ${JSON.stringify(list).slice(0, 200)}`);
  ok(list.length >= 1, "the copy is offered after the crash");
  const opened = await invoke(m, "recovery_open", { index: 0 });
  f = await facts(m, opened.id);
  ok(f.rows === 6 && f.notes.above_file === 4 && f.notes.below_file === 1, `the recovered tab has the table and the notes (${f.rows} rows, ${JSON.stringify(f.notes)})`);
};

(async () => {
  backup();
  try {
    await proof();
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
