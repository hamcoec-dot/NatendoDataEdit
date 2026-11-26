// Drives the REAL built app to check the clean matrix: counts, a protected column, one undo step, the saved bytes,
// the dates column (a date readable two ways stays) and a log with counts but no values.
// The app's own session, recent-file and recovery data are set aside and put back.
// Usage: NODE_PATH=<playwright> node scripts/real-matrix.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-matrix.cjs <exe> <evidence folder>");
  process.exit(2);
}
const WORK = path.join(evidence, "work");
fs.mkdirSync(WORK, { recursive: true });
const DATA = path.join(process.env.APPDATA, "com.natendo.dataedit");
const LOCAL = path.join(process.env.LOCALAPPDATA, "com.natendo.dataedit");
const PORT = 9420;
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
  // Copied, then moved aside for the run: a screenshot must never show the
  // person's own recent files or tabs.
  for (const f of ["session.json", "recent.json"]) {
    fs.rmSync(path.join(BACKUP, f), { force: true });
    if (fs.existsSync(path.join(DATA, f))) {
      fs.copyFileSync(path.join(DATA, f), path.join(BACKUP, f));
      fs.rmSync(path.join(DATA, f));
    }
  }
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
    else fs.rmSync(path.join(DATA, f), { force: true });
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
const FILE = path.join(OUT, "members.csv");
// Made-up people; the words below must never reach the log.
const LINES = ["Name,Town,Phone,Code,Joined", "  zebedee   QUILL , Yarrowby ,(555) 010-0001, k1 ,12/31/2025", "ottilie  VANTREE,Mossford ,555.010.0002, k2 ,03/04/2026", " perrin ashgrove, Hollin,555 010 0003,k3,Jan 5 2026", ""];
const BODY = LINES.join("\r\n");
fs.writeFileSync(FILE, BODY);
const read = () => fs.readFileSync(FILE, "utf8");
const status = async (m) => ((await m.textContent("footer.status")) || "").replace(/\s+/g, " ");
const body = async (m) => ((await m.textContent("body")) || "").replace(/\s+/g, " ");
const save = (m) => invoke(m, "save", { doc: 1, saveAs: false, overwriteChanged: true, allowLossy: false });
const LOG = path.join(LOCAL, "logs", "natendo.log");

const proof = async () => {
  const logStart = fs.existsSync(LOG) ? fs.statSync(LOG).size : 0;
  const m = await start([FILE]);
  await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed, 60000, 300);
  await sleep(1200);
  await dismiss(m);
  await m.keyboard.press("Control+Home");
  for (let i = 0; i < 3; i++) await m.keyboard.press("ArrowRight");
  await cmd(m, "Protect column");
  ok(await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).protected.join() === "3"), "column D protected");
  await m.keyboard.press("Control+Home");
  await cmd(m, "Clean matrix");
  const dlg = m.getByRole("dialog", { name: "Clean matrix" });
  await dlg.waitFor({ timeout: 8000 });
  ok(/Dates as yyyy-mm-dd/.test(await body(m)), "the dates column");
  const box = (name) => dlg.getByRole("checkbox", { name, exact: true });
  ok(await box("Code (protected)").isDisabled(), "the protected column is greyed");
  await dlg.getByRole("button", { name: "Select all", exact: true }).click();
  await box("Trim for every ticked column").check();
  await box("Collapse spaces: Name").check();
  await box("Digits only: Phone").check();
  await dlg.getByRole("combobox", { name: "Case: Name" }).selectOption("title");
  await box("Dates as yyyy-mm-dd: Joined").check();
  await dlg.getByRole("button", { name: "Preview" }).click();
  ok(await waitFor(async () => /Before\s*After/.test(await body(m))), "the preview answers");
  const count = async (name) => ((await dlg.locator("tbody tr", { has: m.getByRole("checkbox", { name, exact: true }) }).first().locator("td.count").textContent()) || "").trim();
  const counts = [await count("Name"), await count("Town"), await count("Phone")];
  ok(counts.join() === "3,3,3", `counts per column (${counts})`);
  ok(/^2\s*1 readable two ways, left as typed$/.test(await count("Joined")), `dates counted: ${await count("Joined")}`);
  await m.screenshot({ path: path.join(evidence, "clean-matrix.png") });
  await dlg.getByRole("button", { name: "Apply" }).click();
  ok(await waitFor(async () => /Cleaned 11 cells in 4 columns/.test(await status(m))), `applied: ${(await status(m)).slice(0, 120)}`);
  // Undo before saving (a save starts a new history): the file is written as it was.
  await invoke(m, "change", { doc: 1, change: { op: "undo" } });
  await save(m);
  ok(read() === BODY, `one undo and the file is byte for byte as it was: ${JSON.stringify(read())}`);
  await invoke(m, "change", { doc: 1, change: { op: "redo" } });
  await save(m);
  const want = ["Name,Town,Phone,Code,Joined", "Zebedee Quill,Yarrowby,5550100001, k1 ,2025-12-31", "Ottilie Vantree,Mossford,5550100002, k2 ,03/04/2026", "Perrin Ashgrove,Hollin,5550100003,k3,2026-01-05", ""].join(String.fromCharCode(13, 10));
  ok(read() === want, `redo and save: ${JSON.stringify(read())}`);
  await sleep(800);
  const added = fs.existsSync(LOG) ? fs.readFileSync(LOG).subarray(logStart).toString("utf8") : "";
  ok(/Cleaned 11 cells/.test(added) && !/zebedee|quill|ottilie|vantree|ashgrove|yarrowby|010.?000/i.test(added), `the log has counts and no values (${added.length} characters added)`);
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
