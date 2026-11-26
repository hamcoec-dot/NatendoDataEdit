// Drives the REAL built app to check Fill blank cells with stops (C5): the dialog's stop group and count line, the
// fill stopping at a row with no ID, a spacer and a subtotal row, the status, the amber marks (engine side), F7 to the
// next one, Esc, a byte-for-byte save of the result, and one Ctrl+Z.
// The app's own session, recent-file and recovery data are set aside and put back. Made-up data only.
// Usage: NODE_PATH=<playwright> node scripts/real-fillguard.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-fillguard.cjs <exe> <evidence folder>");
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
const FILE = path.join(OUT, "region-sales.csv");
const SRC = ["region,id,amount", "North,101,5", ",102,7", "Subtotal,,12", ",,", ",103,2", "South,104,9", ",105,1", ",,10", ""].join("\r\n");
const WANT = ["region,id,amount", "North,101,5", "North,102,7", "Subtotal,,12", ",,", ",103,2", "South,104,9", "South,105,1", ",,10", ""].join("\r\n");
const body = async (m) => ((await m.textContent("body")) || "").replace(/\s+/g, " ");
const facts = (m, doc = 1) => invoke(m, "doc_info", { doc });
const opened = async (m) => {
  await waitFor(async () => (await facts(m)).indexed, 60000, 300);
  await sleep(1500);
  await dismiss(m);
};
const dlg = "[role=dialog]:has(h2:has-text('Fill blank'))";

const proof = async () => {
  fs.writeFileSync(FILE, SRC);
  const m = await start([FILE]);
  await opened(m);
  await cmd(m, "Fill blank cells");
  await m.waitForSelector(dlg, { timeout: 10000 });
  await m.locator(`${dlg} .all-none ~ div label`).filter({ hasText: /^\s*region\s*$/ }).first().locator("input").click();
  const fs2 = `${dlg} [data-testid=fill-stops]`;
  await m.selectOption(`${fs2} select`, { label: "id" });
  await m.click(`${fs2} label:has-text('Blank rows') input`);
  await m.click(`${fs2} label:has-text('Total rows') input`);
  const counted = await waitFor(async () => {
    const t = await m.textContent(`${dlg} [data-testid=fill-count]`).catch(() => "");
    return /Will fill/.test(t || "") && t;
  });
  ok(/Will fill 2 blank cells; 3 left blank by the stops\./.test(counted || ""), `the count line before Run: ${counted}`);
  await m.screenshot({ path: path.join(evidence, "fill-stops-real-dialog.png") });
  await m.click(`${dlg} button.primary`);
  await sleep(1500);
  ok(/Filled 2 blank cells in 2 rows\. 3 blank cells left blank on purpose by the stops \(marked amber/.test(await body(m)), "the status says 2 filled and 3 left blank on purpose");
  const marks = await invoke(m, "fill_marks", { doc: 1, row: 0, rows: 50 });
  ok(JSON.stringify(marks) === JSON.stringify([[4, 0], [5, 0], [8, 0]]), `the engine marks the cells left blank (${JSON.stringify(marks)})`);
  await m.screenshot({ path: path.join(evidence, "fill-stops-real-marks.png") });
  await m.keyboard.press("F7");
  await sleep(600);
  ok(/A cell the fill left blank on purpose/.test(await body(m)), "F7 goes to a cell left blank");
  await m.keyboard.press("Escape");
  await sleep(500);
  ok((await invoke(m, "fill_marks", { doc: 1, row: 0, rows: 50 })).length === 0, "Esc takes the marks away");
  ok((await facts(m)).can_undo, "the fill can be undone");
  await m.keyboard.press("Control+z");
  await sleep(1000);
  let f = await facts(m);
  ok(!f.can_undo && f.can_redo, "one Ctrl+Z undoes the whole fill (one step)");
  await m.keyboard.press("Control+y");
  await sleep(1000);
  await m.keyboard.press("Control+s");
  await sleep(1500);
  ok(fs.readFileSync(FILE, "utf8") === WANT, "redone and saved: the label stops at the row with no ID, the spacer and the subtotal; other rows byte for byte");
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
