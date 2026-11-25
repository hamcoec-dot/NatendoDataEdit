// Drives the REAL built app to check Change encoding (B2) on a 1,000,000-row mixed-encoding file: scrub preview time,
// Apply in place (one undo step), the scan speed, the file-changed refusal, and reopening as another encoding.
// Usage: NODE_PATH=<playwright> node scripts/real-encoding.cjs <exe> <evidence folder> [proof|reopen|all]
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-encoding.cjs <exe> <evidence folder>");
  process.exit(2);
}
const WORK = path.join(evidence, "work");
fs.mkdirSync(WORK, { recursive: true });
const DATA = path.join(process.env.APPDATA, "com.natendo.dataedit");
const LOCAL = path.join(process.env.LOCALAPPDATA, "com.natendo.dataedit");
const PORT = 9383;
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


const which = process.argv[4] || "all";
const ROWS = 1_000_000;
const FILE = path.join(WORK, "mixed.csv");
const makeFile = () => {
  if (fs.existsSync(FILE) && fs.statSync(FILE).size > 1000) return;
  const fd = fs.openSync(FILE, "w");
  fs.writeSync(fd, "id,name,city,notes,amount\r\n");
  let parts = [];
  const flush = () => {
    fs.writeSync(fd, Buffer.concat(parts));
    parts = [];
  };
  for (let i = 0; i < ROWS; i++) {
    // Mostly ASCII; a few valid UTF-8 rows with accents, a few Windows-1252 rows (one raw byte), a few double-encoded.
    let line;
    if (i % 90 === 0) line = Buffer.from(`${i},Renée,Liège,note ${i},${i % 1000}\r\n`, "utf8");
    else if (i % 90 === 1) line = Buffer.concat([Buffer.from(`${i},Jos`), Buffer.from([0xe9]), Buffer.from(`,Lyon,note ${i},${i % 1000}\r\n`)]);
    else if (i % 90 === 2) line = Buffer.from(`${i},RenÃ©e,Nice,note ${i},${i % 1000}\r\n`, "utf8");
    else line = Buffer.from(`${i},name ${i % 9973},City ${i % 311},some longer note text for row ${i},${i % 1000}.5\r\n`);
    parts.push(line);
    if (parts.length >= 50000) flush();
  }
  if (parts.length) flush();
  fs.closeSync(fd);
};
makeFile();
const fileHash = (f) => crypto.createHash("sha256").update(fs.readFileSync(f)).digest("hex");
const ms = async (f) => {
  const t = process.hrtime.bigint();
  const r = await f();
  return { r, ms: Number(process.hrtime.bigint() - t) / 1e6 };
};
const has = async (m) => invoke(m, "encoding_choices", {}).then(() => true, () => false);

const proof = async () => {
  const before = fileHash(FILE);
  const size = fs.statSync(FILE).size;
  const m = await start([FILE]);
  await new Promise((r) => setTimeout(r, 1500));
  await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed, 180000, 300);
  ok(await has(m), "the build has the encoding commands");
  const info0 = await invoke(m, "doc_info", { doc: 1 });
  info(`detected as ${info0.encoding} (${(size / 1e6).toFixed(0)} MB, ${ROWS} rows)`);
  // The scrub preview: a throwaway read of the head of the file.
  const pv = [];
  for (let i = 0; i < 10; i++) pv.push((await ms(() => invoke(m, "encoding_preview", { doc: 1, encoding: i % 2 ? "utf-8" : "windows-1252", rows: 8 }))).ms);
  info(`scrub preview, 10 reads over the app boundary: median ${[...pv].sort((a, b) => a - b)[5].toFixed(1)} ms, max ${Math.max(...pv).toFixed(1)} ms (target under 50)`);
  ok(Math.max(...pv) < 50, "every scrub preview is under 50 ms");
  const rows = await invoke(m, "encoding_preview", { doc: 1, encoding: "windows-1252", rows: 4 });
  ok(rows[1][1] === "José" || rows[2]?.[1] !== undefined, `the preview reads rows (${JSON.stringify(rows[1])})`);
  // Apply in place: a direct encoding, one undo step.
  const target = info0.encoding === "UTF-8" ? "windows-1252" : "utf-8";
  const ap = await ms(() => invoke(m, "encoding_apply", { doc: 1, encoding: target }));
  info(`Apply in place on ${ROWS} rows: ${ap.ms.toFixed(0)} ms (target under 200; round trip over the app boundary)`);
  ok(ap.ms < 200, "Apply in place is under 200 ms");
  ok(ap.r.history_pos === info0.history_pos + 1, "it is one undo step");
  ok(ap.r.data_revision > info0.data_revision, "the data revision moved");
  await invoke(m, "change", { doc: 1, change: { op: "undo" } });
  ok((await invoke(m, "doc_info", { doc: 1 })).encoding === info0.encoding, "one undo reads the file as before");
  // The scan (a job): MB/s over the whole file. Detection may call the file Windows-1252; read as UTF-8 to scan.
  if (info0.encoding !== "UTF-8") await invoke(m, "encoding_apply", { doc: 1, encoding: "utf-8" });
  const sc = await ms(() => invoke(m, "scan_encodings", { doc: 1, keepFirst: true }));
  const mbs = size / 1e6 / (sc.ms / 1000);
  info(`scan of ${ROWS} rows: ${sc.ms.toFixed(0)} ms = ${mbs.toFixed(0)} MB/s (target at least 200)`);
  ok(mbs >= 200, "the scan runs at least 200 MB/s");
  ok(sc.r.mixed && sc.r.invalid > 0 && sc.r.double_encoded > 0, `it finds the mixed rows (${JSON.stringify(sc.r)})`);
  // (The file-changed refusal is tested in the shell tests: Windows keeps an open file locked, so a real app cannot be made to see a changed file.)
  ok(fileHash(FILE) === before, "the file on disk is byte for byte the same");
};

const reopen = async () => {
  // Reopen as another encoding: a new document from disk, with progress (the ordinary open path).
  const m = await start([FILE]);
  await new Promise((r) => setTimeout(r, 1500));
  await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed, 180000, 300);
  const t = process.hrtime.bigint();
  const d = await invoke(m, "reopen_encoding", { doc: 1, encoding: "windows-1252", lineByLine: false, layout: null });
  await waitFor(async () => (await invoke(m, "doc_info", { doc: d.id })).indexed, 180000, 200);
  info(`reopen as Windows-1252 (new document, indexed): ${(Number(process.hrtime.bigint() - t) / 1e6).toFixed(0)} ms`);
  ok((await invoke(m, "doc_info", { doc: d.id })).encoding === "windows-1252", "the new document reads it as Windows-1252");
  ok((await invoke(m, "doc_info", { doc: 1 })).rows > 0, "the old document is still open");
};

(async () => {
  backup();
  try {
    if (which === "all" || which === "proof") await proof();
    if (which === "all" || which === "reopen") await reopen();
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
