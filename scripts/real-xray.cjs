// Drives the REAL built app over WebView2 remote debugging to check X-ray gremlins (B1) on 200,000 made-up rows:
// the report counts, the Sanitize preview page and its exact counts, one undo step, the file on disk untouched,
// and the draw time of scrolling with X-ray off and on (off must equal the build you compare with).
// Usage: NODE_PATH=<playwright> node scripts/real-xray.cjs <exe> <evidence folder> [proof|frames|all]
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-xray.cjs <exe> <evidence folder>");
  process.exit(2);
}
const WORK = path.join(evidence, "work");
fs.mkdirSync(WORK, { recursive: true });
const DATA = path.join(process.env.APPDATA, "com.natendo.dataedit");
const LOCAL = path.join(process.env.LOCALAPPDATA, "com.natendo.dataedit");
const PORT = 9377;
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
const FILE = path.join(WORK, "names.csv");
const makeFile = () => {
  const lines = ["name,amount,city"];
  for (let i = 0; i < ROWS; i++) {
    let name = `Name ${i}`;
    if (i % 100 === 1) name = `Smаrt ${i}`; // Cyrillic a inside a Latin word
    else if (i % 100 === 2) name = `O’Brien ${i}`; // curly quote
    else if (i % 100 === 3) name = `Main St ${i}`; // no-break space
    else if (i % 100 === 4) name = `Москва ${i}`; // all Cyrillic: never flagged
    lines.push(`"${name}",${(i % 1000) + 0.5},City ${i % 311}`);
  }
  fs.writeFileSync(FILE, lines.join("\r\n") + "\r\n", "utf8");
};
makeFile();
const fileHash = () => crypto.createHash("sha256").update(fs.readFileSync(FILE)).digest("hex");
const cellOf = async (p, doc, row, col) => (await invoke(p, "cell_text", { doc, row, col })).text;
const spec = () => ({
  cleaner: { version: 1, name: "", steps: [{ tool: "sanitize", zero_width: true, odd_spaces: true, smart_quotes: true, tabs_breaks: false, homoglyphs: true }] },
  scope: { kind: "columns", cols: [0] },
  skip_first: true,
  read: { decimal_comma: false, date_order: "mdy" },
});

const proof = async () => {
  const before = fileHash();
  const m = await start([FILE]);
  await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed, 60000, 300);
  const t0 = Date.now();
  const rep = await invoke(m, "xray_report", { doc: 1, cols: [], keepFirst: true });
  info(`report over ${ROWS} rows x 3 columns took ${Date.now() - t0} ms`);
  const per = ROWS / 100;
  ok(rep.cells === per * 3 && rep.rows === per * 3, `the report counts exactly ${per * 3} cells in ${per * 3} rows (${rep.cells}, ${rep.rows})`);
  const kinds = Object.fromEntries(rep.kinds);
  ok(kinds.homoglyph === per && kinds.smart_quote === per && kinds.odd_space === per, `per kind ${per} each (${JSON.stringify(rep.kinds)})`);
  const pv = await invoke(m, "clean_preview_page", { doc: 1, spec: spec(), now: Date.now() / 1000, offset: 0, want: 50 });
  ok(pv.changed === per * 3, `the preview counts every changed cell exactly (${pv.changed})`);
  ok(pv.homoglyph_cells === per, `look-alike cells counted (${pv.homoglyph_cells})`);
  const pv2 = await invoke(m, "clean_preview_page", { doc: 1, spec: spec(), now: Date.now() / 1000, offset: 100, want: 50 });
  ok(pv2.changes.length === 50 && pv2.changes[0].number !== pv.changes[0].number, "a later page lists other cells");
  const rep2 = await invoke(m, "data_op", { doc: 1, op: { op: "clean", spec: spec(), now: Date.now() / 1000 } });
  const c1 = await cellOf(m, 1, 1, 0);
  const c2 = await cellOf(m, 1, 2, 0);
  ok(c1 === "Smart 1" && c2 === "O'Brien 2", `cells fixed (${c1}, ${c2})`);
  ok((await cellOf(m, 1, 4, 0)) === "Москва 4", "an all-Cyrillic name is untouched");
  const after = await invoke(m, "xray_report", { doc: 1, cols: [], keepFirst: true });
  ok(after.cells === 0, `X-ray finds nothing after Sanitize (${after.cells})`);
  await invoke(m, "data_op", { doc: 1, op: { op: "undo" } });
  ok((await cellOf(m, 1, 1, 0)) === "Smаrt 1", "one undo puts it all back");
  ok(fileHash() === before, "the file on disk is byte for byte the same");
};

const frames = async () => {
  for (const on of [false, true]) {
    const m = await start([FILE]);
    await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed, 60000, 300);
    if (on) await m.keyboard.press("Control+Shift+X").catch(() => {});
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
    info(`[${path.basename(path.dirname(exe))}] scrolling ${ROWS} rows, X-ray ${on ? "on" : "off"}: draw median ${runs[0].median.toFixed(2)} ms, p95 ${runs[0].p95.toFixed(2)} ms (${runs[0].n} draws; all medians ${runs.map((r) => r.median.toFixed(2)).join(", ")})`);
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
