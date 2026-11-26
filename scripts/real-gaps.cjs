// Drives the REAL built app to check Find gaps in a sequence: the dialog on a made-up invoice file (with the engine's
// own rules: file order, each shape), the rows next to each gap, the result tab and the file left untouched.
// The app's own session, recent-file and recovery data are set aside and put back.
// Usage: NODE_PATH=<playwright> node scripts/real-gaps.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-gaps.cjs <exe> <evidence folder>");
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
const FILE = path.join(OUT, "invoices.csv");
// Made-up invoices: 00103 and 00106-00107 missing, 00104 twice, one drop, one other shape, one blank.
const NOS = ["INV-00101", "INV-00102", "INV-00104", "INV-00104", "INV-00109", "INV-00105", "INV-00108", "PO-7", ""];
const BODY = ["Invoice,Clerk", ...NOS.map((n, i) => `${n},clerk${i % 3}`), ""].join("\r\n");
fs.writeFileSync(FILE, BODY);
const status = async (m) => ((await m.textContent("footer.status")) || "").replace(/\s+/g, " ");
const body = async (m) => ((await m.textContent("body")) || "").replace(/\s+/g, " ");

const proof = async () => {
  const m = await start([FILE]);
  await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed, 60000, 300);
  await sleep(1200);
  await dismiss(m);
  await cmd(m, "Find gaps in a sequence");
  const dlg = m.getByRole("dialog", { name: "Find gaps in a sequence" });
  await dlg.waitFor({ timeout: 8000 });
  ok(await waitFor(async () => /3 missing in 2 ranges/.test(await body(m))), "the dialog audits the column");
  let t = await body(m);
  ok(/INV-00103 \(1 missing\)/.test(t) && /INV-00106 to INV-00107 \(2 missing\)/.test(t), "the ranges");
  ok(/1 repeated number \(1 extra row\)/.test(t) && /1 drop in file order \(file row 7\)/.test(t) && /1 row of a different shape \(file row 9\)/.test(t) && /1 blank/.test(t), "repeats, drop, other shape and blank with file rows");
  await m.screenshot({ path: path.join(evidence, "gaps-dialog.png") });
  await dlg.getByLabel("Start at (optional)").fill("PO-100");
  await dlg.getByRole("button", { name: "Find gaps" }).click();
  ok(await waitFor(async () => /type the start as a number, or the form INV-#####/.test(await body(m))), "a start in the wrong form is refused");
  await dlg.getByLabel("Start at (optional)").fill("INV-00099");
  await dlg.getByRole("button", { name: "Find gaps" }).click();
  ok(await waitFor(async () => /5 missing in 3 ranges/.test(await body(m))), "a typed start adds the gap before the first");
  await dlg.getByLabel("Start at (optional)").fill("");
  await dlg.getByRole("button", { name: "Find gaps" }).click();
  await waitFor(async () => /3 missing in 2 ranges/.test(await body(m)));
  await dlg.getByRole("button", { name: "Show the rows next to each gap" }).click();
  await sleep(900);
  let s = await status(m);
  ok(/Showing 5 rows next to a gap/.test(s), `rows beside the gaps: ${s.slice(0, 200)}`);
  await m.screenshot({ path: path.join(evidence, "gaps-rows.png") });
  await cmd(m, "Clear filter");
  await cmd(m, "Find gaps in a sequence");
  await waitFor(async () => /3 missing in 2 ranges/.test(await body(m)));
  await dlg.getByRole("button", { name: "Open as a tab" }).click();
  await sleep(1200);
  t = await body(m);
  ok(/Gaps in Invoice \(invoices\)/.test(t), "the result tab names the column and the file");
  const info = await invoke(m, "doc_info", { doc: 2 });
  ok(info.rows === 15 && info.cols === 3, `the result table: ${info.rows} rows x ${info.cols}`);
  await m.screenshot({ path: path.join(evidence, "gaps-tab.png") });
  ok(fs.readFileSync(FILE, "utf8") === BODY, "the source file is untouched");
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
