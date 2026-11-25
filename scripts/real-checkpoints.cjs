// Drives the REAL built app to check Checkpoints (B4) on a 1,000,000-row file: naming a moment, going back across
// 1,000 steps (target under 500 ms) and forward, the file untouched, and remove-duplicates time (compare two builds).
// Usage: NODE_PATH=<playwright> node scripts/real-checkpoints.cjs <exe> <evidence folder> [proof|bulk|all]
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-checkpoints.cjs <exe> <evidence folder>");
  process.exit(2);
}
const WORK = path.join(evidence, "work");
fs.mkdirSync(WORK, { recursive: true });
const DATA = path.join(process.env.APPDATA, "com.natendo.dataedit");
const LOCAL = path.join(process.env.LOCALAPPDATA, "com.natendo.dataedit");
const PORT = 9381;
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
const FILE = path.join(WORK, "big.csv");
const makeFile = () => {
  if (fs.existsSync(FILE) && fs.statSync(FILE).size > 1000) return;
  const fd = fs.openSync(FILE, "w");
  fs.writeSync(fd, "id,name,amount,city\r\n");
  let buf = [];
  for (let i = 0; i < ROWS; i++) {
    // Every 5th row repeats the one before it (duplicates to remove).
    const k = i % 5 === 4 ? i - 1 : i;
    buf.push(`${k},name ${k % 9973},${(k % 1000) + 0.5},City ${k % 311}`);
    if (buf.length >= 50000) {
      fs.writeSync(fd, buf.join("\r\n") + "\r\n");
      buf = [];
    }
  }
  if (buf.length) fs.writeSync(fd, buf.join("\r\n") + "\r\n");
  fs.closeSync(fd);
};
makeFile();
const fileHash = (f) => crypto.createHash("sha256").update(fs.readFileSync(f)).digest("hex");
const ms = async (f) => {
  const t = process.hrtime.bigint();
  const r = await f();
  return { r, ms: Number(process.hrtime.bigint() - t) / 1e6 };
};
const median = (a) => [...a].sort((x, y) => x - y)[Math.floor(a.length / 2)] ?? 0;
const hasCheckpoints = async (m) => (await invoke(m, "checkpoint_list", { doc: 1 }).then(() => true, () => false));

const proof = async () => {
  const before = fileHash(FILE);
  const m = await start([FILE]);
  await new Promise((r) => setTimeout(r, 1500));
  await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed, 120000, 300);
  ok(await hasCheckpoints(m), "the build has checkpoints");
  // Naming a moment: one call (includes the app round trip).
  const t = [];
  for (let i = 0; i < 20; i++) t.push((await ms(() => invoke(m, "checkpoint_add", { doc: 1, name: `m${i}` }))).ms);
  info(`checkpoint_add round trip over the app boundary: median ${median(t).toFixed(2)} ms, max ${Math.max(...t).toFixed(2)} ms (marks cost nothing: 1M marks < 1 ms in the engine)`);
  await invoke(m, "checkpoint_add", { doc: 1, name: "start" });
  let list = await invoke(m, "checkpoint_list", { doc: 1 });
  const first = list.find((c) => c.name === "start");
  // 1,000 edits as separate undo steps.
  for (let i = 0; i < 1000; i++) await invoke(m, "set_cells", { doc: 1, cells: [[1 + ((i * 7919) % (ROWS - 1)), 1, `edit${i}`]] });
  await invoke(m, "checkpoint_add", { doc: 1, name: "after 1000" });
  list = await invoke(m, "checkpoint_list", { doc: 1 });
  const end = list.find((c) => c.name === "after 1000");
  const back = await ms(() => invoke(m, "checkpoint_restore", { doc: 1, checkpoint: first.id }));
  info(`restore across 1,000 steps on ${ROWS} rows: ${back.ms.toFixed(0)} ms (target under 500)`);
  ok(back.ms < 500, "restore across 1,000 steps is under 500 ms");
  const cell1 = await invoke(m, "cell_text", { doc: 1, row: 1, col: 1 });
  ok(cell1.text === "name 0", `the table is back to the start (${cell1.text})`);
  const fwd = await ms(() => invoke(m, "checkpoint_restore", { doc: 1, checkpoint: end.id }));
  info(`and forward again: ${fwd.ms.toFixed(0)} ms`);
  const cell2 = await invoke(m, "cell_text", { doc: 1, row: 1 + ((0 * 7919) % (ROWS - 1)), col: 1 });
  ok(cell2.text === "edit0", `forward again shows the edits (${cell2.text})`);
  // Restoring made a "Before restoring" moment only when needed (here the moment left was named).
  list = await invoke(m, "checkpoint_list", { doc: 1 });
  ok(list.every((c) => c.reachable), "every checkpoint is still reachable");
  ok(fileHash(FILE) === before, "the file on disk is byte for byte the same");
};

const bulk = async () => {
  // Remove duplicates on 1M rows: best of 3, each on a fresh app. With checkpoints the app also names a moment first.
  const times = [];
  for (let k = 0; k < 3; k++) {
    const m = await start([FILE]);
    await new Promise((r) => setTimeout(r, 1500));
    await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed, 120000, 300);
    const r = await ms(() => invoke(m, "data_op", { doc: 1, op: { op: "remove_duplicates", spec: { cols: [0], match_case: false, trim: false, keep_first: true, keep: "first" } } }));
    times.push(r.ms);
    const auto = (await hasCheckpoints(m)) ? (await invoke(m, "checkpoint_list", { doc: 1 })).map((c) => c.name).join(",") : "(no checkpoints in this build)";
    if (k === 0) info(`[${path.basename(path.dirname(exe))}] remove duplicates ${ROWS} rows: automatic checkpoint: ${auto}`);
    await killApp();
  }
  info(`[${path.basename(path.dirname(exe))}] remove duplicates ${ROWS} rows (1 key column): best ${Math.min(...times).toFixed(0)} ms, runs ${times.map((x) => x.toFixed(0)).join(", ")}`);
};

(async () => {
  backup();
  try {
    if (which === "all" || which === "proof") await proof();
    if (which === "all" || which === "bulk") await bulk();
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
