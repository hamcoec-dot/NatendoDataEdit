// Drives the REAL built app to check ragged rows (C1a): the open note and Review, the short and long filters, that
// editing never pads a short row (saved bytes), Go to special, and the refusal after a column change.
// The app's own session, recent-file and recovery data are set aside and put back.
// Usage: NODE_PATH=<playwright> node scripts/real-ragged.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-ragged.cjs <exe> <evidence folder>");
  process.exit(2);
}
const WORK = path.join(evidence, "work");
fs.mkdirSync(WORK, { recursive: true });
const DATA = path.join(process.env.APPDATA, "com.natendo.dataedit");
const LOCAL = path.join(process.env.LOCALAPPDATA, "com.natendo.dataedit");
const PORT = 9388;
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
const FILE = path.join(OUT, "ragged.csv");
const LINES = ["id,name,city", "1,ann,Oslo", "2,Smith, John,Rome", "3", "4,\"bo, jr\",Lyon", "5,eve,Turin", ""];
const BODY = LINES.join("\r\n");
fs.writeFileSync(FILE, BODY);
const read = () => fs.readFileSync(FILE, "utf8");
const status = async (m) => ((await m.textContent("footer.status")) || "").replace(/\s+/g, " ");
const body = async (m) => ((await m.textContent("body")) || "").replace(/\s+/g, " ");
const save = (m) => invoke(m, "save", { doc: 1, saveAs: false, overwriteChanged: true, allowLossy: false });

const proof = async () => {
  const m = await start([FILE]);
  await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed, 60000, 300);
  await sleep(1200);
  await dismiss(m);
  const facts = await invoke(m, "doc_info", { doc: 1 });
  ok(facts.usual_fields === 3 && facts.ragged_rows === 2 && !facts.first_row_ragged, `the file's facts: ${facts.ragged_rows} ragged, usual ${facts.usual_fields}`);
  ok(await waitFor(async () => /2 rows have a different number of fields than most \(usual: 3\)\./.test(await body(m))), "the open note");
  await m.screenshot({ path: path.join(evidence, "ragged-note.png") });
  await m.getByRole("button", { name: "Review" }).click();
  await sleep(900);
  let s = await status(m);
  ok(/Showing the 2 rows with a different number of fields \(1 short, 1 long\)/.test(s) && /2 of [56] rows shown/.test(s), `Review: ${s.slice(0, 200)}`);
  await m.screenshot({ path: path.join(evidence, "ragged-shown.png") });
  await cmd(m, "Show only rows with too few fields");
  s = await status(m);
  ok(/Showing the 1 row with too few fields/.test(s), `short rows: ${s.slice(0, 120)}`);
  await cmd(m, "Show only rows with too many fields");
  s = await status(m);
  ok(/Showing the 1 row with too many fields/.test(s), `long rows: ${s.slice(0, 120)}`);
  await cmd(m, "Clear filter");
  s = await status(m);
  ok(!/rows shown/.test(s), `filter cleared: ${s.slice(0, 80)}`);
  // Go to special picks the ragged rows whole.
  await cmd(m, "Go to special");
  await m.getByRole("dialog").getByRole("radio", { name: "Rows with the wrong number of fields" }).check();
  await m.getByRole("dialog").getByRole("button", { name: "OK" }).click();
  await sleep(900);
  s = await status(m);
  info(`go to special: ${s.slice(0, 200)}`);
  ok(/8 cells/.test(s), "go to special picks the two ragged rows whole (2 rows x 4 columns)");
  // Emptying cells a short row does not have changes nothing; text typed there stays, clearing it returns the row.
  await invoke(m, "set_cells", { doc: 1, cells: [[3, 1, ""], [3, 2, ""]] });
  await save(m);
  ok(read() === BODY, `empties past the end left the file as it was: ${JSON.stringify(read())}`);
  await invoke(m, "set_cells", { doc: 1, cells: [[3, 2, "z"]] });
  await invoke(m, "set_cells", { doc: 1, cells: [[3, 2, ""]] });
  await save(m);
  ok(read() === BODY, "typed and cleared again: the row has its own width");
  await invoke(m, "set_cells", { doc: 1, cells: [[3, 0, "30"]] });
  await save(m);
  ok(read() === BODY.replace("\r\n3\r\n", "\r\n30\r\n"), `editing inside the short row does not pad it: ${JSON.stringify(read())}`);
  // After a column change the op is refused with the reason.
  await invoke(m, "change", { doc: 1, change: { op: "insert_cols", at: 1, count: 1 } });
  let why = "";
  await invoke(m, "data_op", { doc: 1, op: { op: "show_ragged", keep_first: true } }).catch((e) => (why = String(e)));
  ok(/added, removed or moved/.test(why), `after a column change: ${why.slice(0, 160)}`);
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
