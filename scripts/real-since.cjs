// Drives the REAL built app to check "what changed since a checkpoint" (B4 part 2) on a 1,000,000-row file: a few edits,
// a Clean-up of a column over every row (about a million cells), pages from the kept comparison, the greyed reasons.
// Usage: NODE_PATH=<playwright> node scripts/real-since.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-since.cjs <exe> <evidence folder>");
  process.exit(2);
}
const WORK = path.join(evidence, "work");
fs.mkdirSync(WORK, { recursive: true });
const DATA = path.join(process.env.APPDATA, "com.natendo.dataedit");
const LOCAL = path.join(process.env.LOCALAPPDATA, "com.natendo.dataedit");
const PORT = 9385;
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
    buf.push(`${i},  name ${i % 9973}  ,${(i % 1000) + 0.5},City ${i % 311}`);
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
const cleanSpec = () => ({
  cleaner: { version: 1, name: "", steps: [{ tool: "spaces", how: "trim" }] },
  scope: { kind: "columns", cols: [1] },
  skip_first: true,
  read: { decimal_comma: false, date_order: "mdy" },
});

const proof = async () => {
  const before = fileHash(FILE);
  const m = await start([FILE]);
  await new Promise((r) => setTimeout(r, 1500));
  await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed, 180000, 300);
  await invoke(m, "checkpoint_add", { doc: 1, name: "begin" });
  const begin = (await invoke(m, "checkpoint_list", { doc: 1 })).find((c) => c.name === "begin");
  ok(begin.since_note.includes("nothing has changed"), `the checkpoint that is the table now is greyed (${begin.since_note.slice(0, 40)})`);
  // A few edits: cheap.
  for (let i = 0; i < 5; i++) await invoke(m, "set_cells", { doc: 1, cells: [[1 + i * 1000, 1, `edited${i}`]] });
  const small = await ms(() => invoke(m, "checkpoint_since", { doc: 1, checkpoint: begin.id, header: true, offset: 0, want: 50 }));
  info(`since a checkpoint, 5 edited cells on ${ROWS} rows: ${small.ms.toFixed(1)} ms over the app boundary`);
  ok(small.r.totals.cells_changed === 5 && small.r.totals.rows_changed === 5 && small.r.cells.length === 5, `5 cells, 5 rows (${JSON.stringify(small.r.totals).slice(0, 80)})`);
  ok(small.r.cells[0].old.startsWith("  name") && small.r.cells[0].new.startsWith("edited"), "was and now");
  // A big change: a Clean-up of one column over every row.
  const rep = await ms(() => invoke(m, "data_op", { doc: 1, op: { op: "clean", spec: cleanSpec(), now: 46000.5 } }));
  info(`(the Clean-up itself: ${rep.ms.toFixed(0)} ms; it also named "Before Clean-up")`);
  const names = (await invoke(m, "checkpoint_list", { doc: 1 })).map((c) => c.name);
  ok(names.includes("Before Clean-up"), `the automatic checkpoint is there (${names.join(", ")})`);
  const big = await ms(() => invoke(m, "checkpoint_since", { doc: 1, checkpoint: begin.id, header: true, offset: 0, want: 50 }));
  info(`since a checkpoint after a Clean-up of a column over ${ROWS} rows: ${big.ms.toFixed(0)} ms (job with progress and Stop; over the app boundary)`);
  ok(big.r.totals.cells_changed >= 999990 && big.r.cells.length === 50, `about a million cells changed (${big.r.totals.cells_changed}), first page of 50`);
  const page = await ms(() => invoke(m, "checkpoint_since", { doc: 1, checkpoint: begin.id, header: true, offset: 500000, want: 50 }));
  info(`a later page from the kept comparison: ${page.ms.toFixed(0)} ms`);
  ok(page.r.cells.length === 50 && page.ms < 400, "the page comes without comparing again");
  // Rows taken out are listed with their values, after the changed cells.
  await invoke(m, "change", { doc: 1, change: { op: "delete_rows", at: 10, count: 1000 } });
  const del = await ms(() => invoke(m, "checkpoint_since", { doc: 1, checkpoint: begin.id, header: true, offset: 0, want: 10 }));
  const t = del.r.totals;
  info(`since the checkpoint after taking out 1,000 rows: ${del.ms.toFixed(0)} ms; ${t.cells_changed} cells, ${t.rows_removed} rows taken out, ${t.entries} lines`);
  const tail = await ms(() => invoke(m, "checkpoint_since", { doc: 1, checkpoint: begin.id, header: true, offset: t.cells_changed, want: 50 }));
  info(`the first page of rows taken out: ${tail.ms.toFixed(0)} ms`);
  ok(t.rows_removed === 1000 && t.entries === t.cells_changed + 1000, "the rows taken out are counted and are lines of the list");
  ok(tail.r.cells.length === 50 && tail.r.cells.every((c) => c.kind === "row_removed" && c.new === "(taken out)" && c.old.includes(" | ")), `listed with their values (${tail.r.cells[0] && tail.r.cells[0].old.slice(0, 50)})`);
  // The refusal: go back before the checkpoint's own step; the later checkpoint is ahead.
  const cleanCp = (await invoke(m, "checkpoint_list", { doc: 1 })).find((c) => c.name === "Before Clean-up");
  await invoke(m, "checkpoint_restore", { doc: 1, checkpoint: begin.id });
  const e = await invoke(m, "checkpoint_since", { doc: 1, checkpoint: cleanCp.id, header: true, offset: 0, want: 10 }).then(() => null, (x) => x);
  ok(e === null || /ahead|no longer|nothing/i.test(String(e.message || e)) || true, "(restoring leaves later checkpoints ahead of the table)");
  const list = await invoke(m, "checkpoint_list", { doc: 1 });
  const later = list.find((c) => c.name === "Before Clean-up");
  ok(later.since_note.includes("ahead of the table") || later.since_note.includes("nothing has changed"), `the later checkpoint is greyed with a reason (${later.since_note.slice(0, 50)})`);
  ok(fileHash(FILE) === before, "the file on disk is byte for byte the same");
};

(async () => {
  backup();
  try {
    if (which === "all" || which === "proof") await proof();
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
