// Drives the REAL built app through the date radar: a made-up file with both orders, ambiguous dates, eight-digit
// birth dates and a registration-number column; the note, the pill, next and previous, a column set to an order, and
// that the file is not changed. The app's own session, recent-file and recovery data are set aside and put back.
// Usage: NODE_PATH=<playwright> node scripts/real-dates.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-dates.cjs <exe> <evidence folder>");
  process.exit(2);
}
const WORK = path.join(evidence, "work");
fs.mkdirSync(WORK, { recursive: true });
const DATA = path.join(process.env.APPDATA, "com.natendo.dataedit");
const LOCAL = path.join(process.env.LOCALAPPDATA, "com.natendo.dataedit");
const PORT = 9390;
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
  // The person's recent files and session are moved aside before the app starts, so no screenshot can show a real
  // file name (they were copied above and are copied back afterwards).
  for (const f of ["session.json", "recent.json"]) if (fs.existsSync(path.join(DATA, f))) fs.renameSync(path.join(DATA, f), path.join(BACKUP, `${f}.aside-${Date.now()}`));
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
const FILE = path.join(OUT, "dates.csv");
const LINES = ["born,reg_no", "25/04/2026,19750612", "03/04/2026,19800101", "04/25/2026,20010930", "05/05/2026,19991231", "19660305,19660305", "2026-01-02,19750101", "07/08/2026,19750103", ""];
const BODY = LINES.join("\r\n");
fs.writeFileSync(FILE, BODY);
const status = async (m) => ((await m.textContent("footer.status")) || "").replace(/\s+/g, " ");

const proof = async () => {
  const m = await start([FILE]);
  await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed, 60000, 300);
  await sleep(1000);
  await dismiss(m);
  const yes = m.getByRole("button", { name: "Yes, column names" });
  if (await yes.isVisible().catch(() => false)) await yes.click();
  await sleep(500);
  await cmd(m, "Check dates in this column");
  let s = await status(m);
  ok(/born: 2 dates could be read two ways, 1 only month first, 1 only day first/.test(s) || /born: 2 dates could be read two ways/.test(s), `the note: ${s.slice(0, 200)}`);
  const pill = () => m.getByTestId("date-pill").textContent().then((t) => (t || "").replace(/\s+/g, " "));
  ok(/Ambiguous dates: 2/.test(await pill()), `pill: ${await pill()}`);
  await m.screenshot({ path: path.join(evidence, "date-radar.png") });
  const scan = await invoke(m, "date_scan", { doc: 1, col: 0, keepFirst: true, sample: null });
  ok(scan.ambiguous === 2 && scan.compact === 1 && scan.compact_numbers === 0 && scan.same_either === 1, `counts: ${JSON.stringify(scan)}`);
  const reg = await invoke(m, "date_scan", { doc: 1, col: 1, keepFirst: true, sample: null });
  ok(reg.compact === 0 && reg.compact_numbers === 7 && reg.ambiguous === 0, `registration numbers stay numbers: ${JSON.stringify(reg)}`);
  ok(JSON.stringify(await invoke(m, "date_marks", { doc: 1, col: 0, keepFirst: true, row: 0, rows: 50 })) === "[2,7]", "marks on the two ambiguous rows");
  const at = async () => ((await m.textContent(".namebox, [data-testid=namebox]").catch(() => "")) || "");
  await cmd(m, "Next ambiguous date");
  ok((await invoke(m, "date_find", { doc: 1, col: 0, keepFirst: true, from: 2, forward: true })) === 7, "next ambiguous after row 3 is row 8");
  // A column set to day first answers: no amber, and the other order is counted.
  await invoke(m, "set_column_type", { doc: 1, col: 0, kind: { date: "dmy" } });
  const set = await invoke(m, "date_scan", { doc: 1, col: 0, keepFirst: true, sample: null });
  ok(set.set_order === "dmy" && set.other_order === 1 && set.ambiguous === 0, `set to day first: ${JSON.stringify(set)}`);
  await invoke(m, "set_column_type", { doc: 1, col: 0, kind: "text" });
  let why = "";
  await invoke(m, "date_scan", { doc: 1, col: 0, keepFirst: true, sample: null }).catch((e) => (why = String(e)));
  ok(/set to Text/.test(why), `Text column refused: ${why.slice(0, 120)}`);
  ok(fs.readFileSync(FILE, "utf8") === BODY && !(await invoke(m, "doc_info", { doc: 1 })).modified, "the file and the table are unchanged");
  // The review screen: counts, a choice, eight-digit values only when ticked, one undo step, the recipe.
  await invoke(m, "set_column_type", { doc: 1, col: 0, kind: "auto" });
  await sleep(500);
  const text = async (row, col = 0) => (await invoke(m, "cell_text", { doc: 1, row, col })).text;
  const column = async () => {
    const out = [];
    for (let r = 1; r <= 7; r++) out.push(await text(r));
    return JSON.stringify(out);
  };
  const before = await column();
  await cmd(m, "Review and convert dates");
  const dialog = m.getByRole("dialog", { name: /^Review dates/ });
  ok(await waitFor(async () => (await dialog.count()) === 1), "the review screen opens");
  ok(/only be read month first/.test((await dialog.textContent()) || "") && (await dialog.getByRole("radio", { name: /Leave them/ }).isChecked()), "a mixed column is warned about and opens on leave");
  const preview = async () => ((await m.getByTestId("date-review-preview").textContent()) || "").replace(/\s+/g, " ");
  await waitFor(async () => /become/.test(await preview()));
  ok(/3 cells become like 2026-04-25\. ?3 stay as typed/.test(await preview()), `leave: ${(await preview()).slice(0, 70)}`);
  await dialog.getByRole("radio", { name: /Day first/ }).check();
  await dialog.getByRole("checkbox", { name: /8-digit/ }).check();
  await sleep(800);
  ok(/6 cells become like 2026-04-25\. ?0 stay as typed/.test(await preview()), `day first with eight digits: ${(await preview()).slice(0, 70)}`);
  await m.screenshot({ path: path.join(evidence, "date-review.png") });
  await dialog.getByRole("button", { name: "Apply" }).click();
  await sleep(1500);
  const want = JSON.stringify(["2026-04-25", "2026-04-03", "2026-04-25", "2026-05-05", "1966-03-05", "2026-01-02", "2026-08-07"]);
  ok((await column()) === want, `converted: ${await column()}`);
  const recipe = JSON.stringify(await invoke(m, "recipe_get", { doc: 1, header: true }));
  ok(/"ambiguous":"dmy"/.test(recipe) && /read day first/.test(recipe), "the recipe records and says the choice");
  ok((await text(1, 1)) === "19750612", "the registration numbers are untouched");
  await invoke(m, "change", { doc: 1, change: { op: "undo" } });
  ok((await column()) === before, "one undo restores the column");
  ok(fs.readFileSync(FILE, "utf8") === BODY, "the file is not written without Save");
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
