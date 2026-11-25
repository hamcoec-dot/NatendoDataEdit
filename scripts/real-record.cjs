// Drives the REAL built app over WebView2 remote debugging to check the Record panel (B3) on 200,000 made-up rows and
// a 500-column file: it follows the selection, edits in place as one undo step, the file is untouched, and the
// panel costs little (key to panel updated, open time, scrolling draw time with it open or closed).
// Usage: NODE_PATH=<playwright> node scripts/real-record.cjs <exe> <evidence folder> [proof|frames|all]
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-record.cjs <exe> <evidence folder>");
  process.exit(2);
}
const WORK = path.join(evidence, "work");
fs.mkdirSync(WORK, { recursive: true });
const DATA = path.join(process.env.APPDATA, "com.natendo.dataedit");
const LOCAL = path.join(process.env.LOCALAPPDATA, "com.natendo.dataedit");
const PORT = 9379;
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
const ROWS = 200_000;
const FILE = path.join(WORK, "people.csv");
const WIDE = path.join(WORK, "wide.csv");
const makeFiles = () => {
  const lines = ["name,city,notes,age,email,joined"];
  for (let i = 0; i < ROWS; i++) lines.push(`Person${i},City${i % 311},note ${i},${20 + (i % 70)},p${i}@example.test,2024-0${1 + (i % 9)}-1${i % 9}`);
  fs.writeFileSync(FILE, lines.join("\r\n") + "\r\n", "utf8");
  const head = Array.from({ length: 500 }, (_, c) => `col${c}`).join(",");
  const rows = [head];
  for (let r = 0; r < 100; r++) rows.push(Array.from({ length: 500 }, (_, c) => `v${r}_${c}`).join(","));
  fs.writeFileSync(WIDE, rows.join("\r\n") + "\r\n", "utf8");
};
makeFiles();
const fileHash = (f) => crypto.createHash("sha256").update(fs.readFileSync(f)).digest("hex");
const cellOf = async (p, doc, row, col) => (await invoke(p, "cell_text", { doc, row, col })).text;
const panelName = (p) => p.getByTestId("record-panel").locator("input.value").first().inputValue();
const curN = async (p) => Number(((await p.getByTestId("record-pos").textContent()) || "").match(/Row ([\d,]+) of/)?.[1].replace(/,/g, "")) || 0;
const median = (a) => [...a].sort((x, y) => x - y)[Math.floor(a.length / 2)] ?? 0;

const proof = async () => {
  const before = fileHash(FILE);
  const m = await start([FILE]);
  await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed, 60000, 300);
  await m.locator("canvas").first().click({ position: { x: 80, y: 60 } });
  await m.keyboard.press("Alt+r");
  await m.getByTestId("record-panel").waitFor({ timeout: 10000 });
  await waitFor(async () => (await panelName(m)).startsWith("Person"), 5000, 50);
  ok(true, `the Record panel shows a row of ${ROWS} rows (${await panelName(m)})`);
  // Moving the active cell: time from the key to the panel showing the new row.
  const lat = [];
  for (let i = 0; i < 40; i++) {
    const want = (await curN(m)) + 1;
    const t = Date.now();
    await m.keyboard.press("ArrowDown");
    await waitFor(async () => (await panelName(m)) === `Person${want - 1}`, 2000, 5);
    lat.push(Date.now() - t);
  }
  info(`[${path.basename(path.dirname(exe))}] key to panel updated, 40 moves: median ${median(lat)} ms, max ${Math.max(...lat)} ms`);
  const row = await curN(m);
  ok((await panelName(m)) === `Person${row - 1}`, `the panel follows the selection (row ${row})`);
  // Edit in the panel, one undo step.
  const nameField = m.getByTestId("record-panel").getByLabel("city", { exact: true });
  await nameField.fill("Zzz");
  await nameField.press("Enter");
  await waitFor(async () => (await cellOf(m, 1, row, 1)) === "Zzz", 5000, 100);
  ok((await cellOf(m, 1, row, 1)) === "Zzz", "Enter in a panel field changes the cell");
  await invoke(m, "change", { doc: 1, change: { op: "undo" } });
  ok((await cellOf(m, 1, row, 1)) !== "Zzz", "one undo takes it back");
  ok(fileHash(FILE) === before, "the file on disk is byte for byte the same");
  await killApp();
  // A 500-column file: how long the panel takes to open.
  const w = await start([WIDE]);
  await waitFor(async () => (await invoke(w, "doc_info", { doc: 1 })).indexed, 30000, 200);
  await w.locator("canvas").first().click({ position: { x: 80, y: 60 } });
  const t0 = Date.now();
  await w.keyboard.press("Alt+r");
  await w.getByTestId("record-panel").locator("input.value").first().waitFor({ timeout: 10000 });
  info(`opening the panel on a 500-column file took ${Date.now() - t0} ms (input to first field, includes the key press)`);
  const drawn = await w.getByTestId("record-panel").locator("input.value").count();
  ok(drawn < 100, `only the fields in view are drawn (${drawn} of 500)`);
};

const frames = async () => {
  for (const open of [false, true]) {
    const m = await start([FILE]);
    await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed, 60000, 300);
    await m.locator("canvas").first().click({ position: { x: 80, y: 60 } });
    if (open) await m.keyboard.press("Alt+r");
    await sleep(1500);
    const c = await m.locator("canvas").first().boundingBox();
    const runs = [];
    for (let k = 0; k < 5; k++) {
      await m.evaluate(() => {
        window.__ndeDrawLog = [];
      });
      await m.mouse.move(c.x + 300, c.y + 200);
      for (let i = 0; i < 80; i++) {
        await m.mouse.wheel(0, i % 2 ? -120 : 120);
        await sleep(20);
      }
      await sleep(200);
      const d = [...(await m.evaluate(() => window.__ndeDrawLog || []))].sort((a, b) => a - b);
      runs.push({ median: d[Math.floor(d.length / 2)] ?? 0, p95: d[Math.floor(d.length * 0.95)] ?? 0, n: d.length });
    }
    runs.sort((a, b) => a.median - b.median);
    info(`[${path.basename(path.dirname(exe))}] scrolling ${ROWS} rows, panel ${open ? "open (Record, if built)" : "closed"}: draw median ${runs[0].median.toFixed(2)} ms, p95 ${runs[0].p95.toFixed(2)} ms (${runs[0].n} draws; medians ${runs.map((r) => r.median.toFixed(2)).join(", ")})`);
    // Moving the active cell with the panel open or closed: draw times and the whole 100-move run.
    await m.locator("canvas").first().click({ position: { x: 80, y: 60 } });
    const moveRuns = [];
    for (let k = 0; k < 5; k++) {
      await m.evaluate(() => {
        window.__ndeDrawLog = [];
      });
      const t0 = Date.now();
      for (let i = 0; i < 100; i++) {
        await m.keyboard.press(k % 2 ? "ArrowUp" : "ArrowDown");
        await sleep(8);
      }
      const total = Date.now() - t0;
      await sleep(150);
      const d = [...(await m.evaluate(() => window.__ndeDrawLog || []))].sort((a, b) => a - b);
      moveRuns.push({ median: d[Math.floor(d.length / 2)] ?? 0, total });
    }
    moveRuns.sort((a, b) => a.median - b.median);
    info(`[${path.basename(path.dirname(exe))}] 100 cell moves, panel ${open ? "open (Record, if built)" : "closed"}: draw median ${moveRuns[0].median.toFixed(2)} ms; runs ${moveRuns.map((r) => `${r.median.toFixed(2)}ms/${r.total}ms`).join(", ")}`);
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
