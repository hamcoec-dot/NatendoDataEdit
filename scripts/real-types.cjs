// Drives the REAL built app (not the browser preview) over WebView2 remote debugging to check the column type
// overlay (A1b) on a file of 200,000 made-up rows: a column typed Id sorts as codes (2 before 10, 007 before 0007),
// is never counted as numbers by a filter, a total or the column summary, the type is not a change of the file
// (modified stays false, the file's bytes are the same), it is remembered for the file after a restart, and a
// file in private mode remembers nothing. With the frames scenario it also times the draw of a colour scale on a
// typed column.
//
// Usage (after `npx tauri build`):
//   NODE_PATH=<folder with playwright> node scripts/real-types.cjs <exe> <evidence folder> [proof|frames|all]
// The app's own data is backed up and put back; files are made-up and live in <evidence>/work.
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-types.cjs <exe> <evidence folder>");
  process.exit(2);
}
const WORK = path.join(evidence, "work");
fs.mkdirSync(WORK, { recursive: true });
const DATA = path.join(process.env.APPDATA, "com.natendo.dataedit");
const LOCAL = path.join(process.env.LOCALAPPDATA, "com.natendo.dataedit");
const PORT = 9373;
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


const which = process.argv[4] || "all";
const ROWS = 200_000;
const FILE = path.join(WORK, "codes.csv");
const makeFile = () => {
  const lines = ["zip,amount,joined"];
  const ids = ["10", "2", "007", "0007", "7", "100", "02134", "2134"];
  for (let i = 0; i < ROWS; i++) {
    const zip = i < ids.length ? ids[i] : String(100000 + ((i * 7919) % 899999));
    lines.push(`${zip},${(i % 1000) + 0.5},2024-0${1 + (i % 9)}-1${i % 9}`);
  }
  fs.writeFileSync(FILE, lines.join("\r\n") + "\r\n");
};
makeFile();
const fileHash = () => crypto.createHash("sha256").update(fs.readFileSync(FILE)).digest("hex");
const cellOf = async (p, doc, row, col) => (await invoke(p, "cell_text", { doc, row, col })).text;
const sortSpec = (col) => ({ keys: [{ col, descending: false, kind: "auto", match_case: false }], keep_first: true, lists: [], read: { decimal_comma: false, date_order: "mdy" } });

const proof = async () => {
  const before = fileHash();
  let m = await start([FILE]);
  await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed, 60000, 300);
  let info = await invoke(m, "set_column_type", { doc: 1, col: 0, kind: "id" });
  ok(JSON.stringify(info.typed) === '[[0,"id"]]', `the first column is typed Id (${JSON.stringify(info.typed)})`);
  ok(info.modified === false && info.can_undo === false, "a type change is not an edit: the file is not marked changed and there is nothing in the engine's undo");
  ok(info.data_revision > 0, "the data revision moved on, so results made from numbers are out of date");
  // Sort by the Id column: codes by value, more leading zeros after.
  info = await invoke(m, "sort_rows", { doc: 1, spec: sortSpec(0) });
  const first = [];
  for (let r = 1; r <= 8; r++) first.push(await cellOf(m, 1, r, 0));
  ok(JSON.stringify(first) === JSON.stringify(["2", "7", "007", "0007", "10", "100", "2134", "02134"]), `Id sorts as codes (${first.join(" ")})`);
  // A number filter finds nothing in an Id column; the column summary counts no numbers.
  const filt = await invoke(m, "set_filter", { doc: 1, spec: { columns: [{ col: 0, rule: { type: "conditions", first: { type: "number", op: "gt", value: 50 } } }], keep_first: true, read: { decimal_comma: false, date_order: "mdy" } } });
  ok(filt.rows <= 1, `a number filter on an Id column shows only the title row (${filt.rows} rows)`);
  await invoke(m, "clear_filter", { doc: 1 });
  const tot = await invoke(m, "selection_totals", { doc: 1, row: 1, rows: 1000, col: 0, cols: 1, read: { decimal_comma: false, date_order: "mdy" }, distinct: false });
  ok(tot.numbers === 0 && tot.sum === null, `totals of an Id column hold no numbers (${tot.numbers})`);
  const tot2 = await invoke(m, "selection_totals", { doc: 1, row: 1, rows: 1000, col: 1, cols: 1, read: { decimal_comma: false, date_order: "mdy" }, distinct: false });
  ok(tot2.numbers === 1000, `the untyped amount column still totals (${tot2.numbers})`);
  // Back to Auto: sorts as numbers again.
  await invoke(m, "set_column_type", { doc: 1, col: 0, kind: "auto" });
  info = await invoke(m, "sort_rows", { doc: 1, spec: sortSpec(0) });
  const autoFirst = [];
  for (let r = 1; r <= 3; r++) autoFirst.push(await cellOf(m, 1, r, 0));
  info = await invoke(m, "set_column_type", { doc: 1, col: 0, kind: "id" });
  ok(true, `Auto sorted ${autoFirst.join(" ")} (numbers), Id as codes: the overlay changes only the reading`);
  ok(fileHash() === before, "the file on disk is byte for byte the same");
  // Remembered for the file after a restart (types only: a fingerprint, never values or titles).
  await sleep(8000);
  const kept = await m.evaluate(() => Object.entries(localStorage).filter(([k]) => k.startsWith("nde.coltypes")));
  info(`stored: ${JSON.stringify(kept).slice(0, 160)}`);
  ok(kept.length === 1 && !/codes|zip|amount|joined|02134/i.test(JSON.stringify(kept)), "only a fingerprint and the types are remembered (no path, no title, no value)");
  await killApp();
  m = await start([FILE]);
  await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).typed.length > 0, 20000, 300);
  info = await invoke(m, "doc_info", { doc: 1 });
  ok(JSON.stringify(info.typed) === '[[0,"id"]]', `the file opens again with its type (${JSON.stringify(info.typed)})`);
  // Private mode: nothing is remembered.
  await invoke(m, "set_private", { doc: 1, folder: false, on: true }).catch(() => {});
  await invoke(m, "set_column_type", { doc: 1, col: 1, kind: "number" });
  await sleep(3000);
  const kept2 = await m.evaluate(() => Object.entries(localStorage).filter(([k]) => k.startsWith("nde.coltypes")).length);
  ok(kept2 === 0, `a file in private mode keeps no remembered types (entries: ${kept2})`);
};

const frames = async () => {
  for (const typed of [false, true]) {
    const m = await start([FILE]);
    await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed, 60000, 300);
    if (typed) await invoke(m, "set_column_type", { doc: 1, col: 1, kind: "number" });
    await m.locator("canvas").first().click({ position: { x: 260, y: 60 } });
    await cmd(m, "Colour scale");
    await sleep(1500);
    const c = await m.locator("canvas").first().boundingBox();
    const runs = [];
    for (let k = 0; k < 3; k++) {
      await m.evaluate(() => {
        window.__ndeDrawLog = [];
      });
      await m.mouse.move(c.x + 300, c.y + 200);
      for (let i = 0; i < 60; i++) {
        await m.mouse.wheel(0, i % 2 ? -120 : 120);
        await sleep(20);
      }
      await sleep(200);
      const d = [...(await m.evaluate(() => window.__ndeDrawLog || []))].sort((a, b) => a - b);
      runs.push({ median: d[Math.floor(d.length / 2)] ?? 0, p95: d[Math.floor(d.length * 0.95)] ?? 0, n: d.length });
    }
    runs.sort((a, b) => a.median - b.median);
    info(`[${path.basename(path.dirname(exe))}] colour scale on the amount column, ${typed ? "typed Number" : "Auto"}: draw median ${runs[0].median.toFixed(2)} ms, p95 ${runs[0].p95.toFixed(2)} ms (${runs[0].n} draws)`);
    await killApp();
  }
};

(async () => {
  backup();
  try {
    if (which === "all" || which === "proof") await proof();
    if (which === "all" || which === "frames") await frames();
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
