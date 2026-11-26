// Drives the REAL built app to check Find relationships: a planted ZIP -> State dependency, blanks together, the
// exceptions view, a refusal after a change, and the file left untouched.
// The app's own session, recent-file and recovery data are set aside and put back.
// Usage: NODE_PATH=<playwright> node scripts/real-relations.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-relations.cjs <exe> <evidence folder>");
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
const CRLF = String.fromCharCode(13, 10);
// Made-up ZIP codes, states and phones.
const lines = ["ZIP,State,Phone,Phone Type"];
for (let i = 0; i < 400; i++) {
  const state = i === 9 || i === 99 ? "ZZ" : ["AA", "BB", "CC"][(i % 12) % 3];
  const blank = i % 11 === 0;
  lines.push([`000${10 + (i % 12)}`, state, blank ? "" : `555-0${100 + i}`, blank || i === 13 ? "" : ["home", "cell"][i % 2]].join(","));
}
const FILE = path.join(OUT, "voters.csv");
const BODY = [...lines, ""].join(CRLF);
fs.writeFileSync(FILE, BODY);
const status = async (m) => ((await m.textContent("footer.status")) || "").replace(/\s+/g, " ");
const body = async (m) => ((await m.textContent("body")) || "").replace(/\s+/g, " ");

const proof = async () => {
  const m = await start([FILE]);
  await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed, 60000, 300);
  await sleep(1500);
  await dismiss(m);
  await cmd(m, "Find relationships");
  const dlg = m.getByRole("dialog", { name: "Find relationships" });
  await dlg.waitFor({ timeout: 8000 });
  await dlg.getByRole("button", { name: "Select all", exact: true }).click();
  await dlg.getByRole("button", { name: "Find", exact: true }).click();
  ok(await waitFor(async () => /ZIP -> State: holds in 99\.5% \(398 of 400 rows whose ZIP repeats; 2 values break it\)/.test(await body(m))), "ZIP settles State");
  const t = await body(m);
  ok(/Phone and Phone Type are blank together in 97\.\d% of the rows where either is blank/.test(t), "blank together");
  ok(/Left out: Phone \(nearly every row/.test(t) || !/Phone -> /.test(t), "a column of its own values settles nothing");
  await m.screenshot({ path: path.join(evidence, "relations.png") });
  const lineOf = dlg.locator("ol.found li", { hasText: "ZIP -> State" });
  await lineOf.getByRole("button", { name: /^Show the exceptions/ }).click();
  ok(await waitFor(async () => /Showing the 2 rows that break it/.test(await status(m))), `exceptions: ${(await status(m)).slice(0, 120)}`);
  await cmd(m, "Clear filter");
  await cmd(m, "Find relationships");
  await dlg.getByRole("button", { name: "Select all", exact: true }).click();
  await dlg.getByRole("button", { name: "Find", exact: true }).click();
  await waitFor(async () => /ZIP -> State/.test(await body(m)));
  await invoke(m, "set_cells", { doc: 1, cells: [[1, 1, "QQ"]] });
  await dlg.locator("ol.found li", { hasText: "ZIP -> State" }).getByRole("button", { name: /^Show the exceptions/ }).click();
  ok(await waitFor(async () => /scan again/.test(await status(m))), "after a change the old scan is refused");
  ok(fs.readFileSync(FILE, "utf8") === BODY, "the file is untouched");
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
