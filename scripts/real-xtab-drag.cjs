// Drives the REAL built app to check dragging whole columns and rows onto another file's tab (made-up
// files): a column heading dragged onto a tab copies the column in to the right of that file's selected
// column, with the words on the status bar during the drag; one Ctrl+Z there takes it out and Ctrl+Y puts
// it back; Shift moves, and each file undoes its half; rows go under the matching titles and the saved
// file has exactly the expected bytes. The app's session, recent-file and recovery data are put back.
// Usage: NODE_PATH=<playwright> node scripts/real-xtab-drag.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-xtab-drag.cjs <exe> <evidence folder>");
  process.exit(2);
}
const WORK = path.join(evidence, "work");
fs.mkdirSync(WORK, { recursive: true });
const DATA = path.join(process.env.APPDATA, "com.natendo.dataedit");
const LOCAL = path.join(process.env.LOCALAPPDATA, "com.natendo.dataedit");
const PORT = 9389;
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

// ---- the app (only the instance this script started is ever stopped) ----
let browser = null;
let ctx = null;
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
  await sleep(900);
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
const S = path.join(OUT, "s.csv");
const D = path.join(OUT, "d.csv");
const O = path.join(OUT, "o.csv");
fs.writeFileSync(S, "id,city,zip\r\n1,Ames,50010\r\n2,Bly,97622\r\n3,Cody,82414\r\n");
fs.writeFileSync(D, "name,state\r\nAnn,IA\r\nBo,OR\r\nCy,WY\r\n");
fs.writeFileSync(O, "zip,id,city\r\n10001,9,Nyc\r\n");
const facts = (m, doc) => invoke(m, "doc_info", { doc });
const names = (m, doc) => invoke(m, "first_row_names", { doc });
const msg = async (m) => ((await m.locator("footer.status .msg").textContent({ timeout: 1000 }).catch(() => "")) || "").replace(/\s+/g, " ").trim();
const tabEl = (m, name) => m.locator("[data-tab-index]").filter({ hasText: name }).first();
const showTab = async (m, name) => {
  const b = await tabEl(m, name).boundingBox();
  await m.mouse.click(b.x + b.width / 2 - 8, b.y + b.height / 2);
  await sleep(500);
};
const HEAD = 25;
const ROW = 25;
const geometry = async (m) => {
  const c = await m.locator("canvas").first().boundingBox();
  const starts = [];
  let last = 0;
  for (let x = 30; x < 700 && starts.length < 3; x += 2) {
    await m.mouse.click(c.x + x, c.y + HEAD + ROW + 12);
    const r = /C(\d+)$/.exec(await m.locator("input.goto").inputValue());
    const col = r ? Number(r[1]) : 0;
    if (col > last) {
      starts.push(x);
      last = col;
    }
  }
  return { c, starts };
};
/** Press at (x, y), drag up onto a tab, read the status line, let go (Shift when asked). */
const dragTo = async (m, x, y, name, shift) => {
  const tb = await tabEl(m, name).boundingBox();
  await m.mouse.move(x, y);
  await m.mouse.down();
  await m.mouse.move(x + 6, y + 2, { steps: 2 });
  await m.mouse.move(tb.x + tb.width / 2 - 8, tb.y + tb.height / 2, { steps: 10 });
  await sleep(200);
  const words = ((await m.textContent("footer.status").catch(() => "")) || "").replace(/\s+/g, " ");
  if (shift) await m.keyboard.down("Shift");
  await m.mouse.up();
  if (shift) await m.keyboard.up("Shift");
  await sleep(600);
  return words;
};
const undoIn = async (m, name) => {
  await showTab(m, name);
  await m.locator("canvas").first().click({ position: { x: 80, y: 120 } });
  await m.keyboard.press("Control+z");
  await sleep(500);
};

const proof = async () => {
  const m = await start([S, D, O]);
  ok(await waitFor(async () => (await facts(m, 3)).indexed, 20000), "three files opened");
  for (const n of ["s.csv", "d.csv", "o.csv"]) {
    await showTab(m, n);
    const yes = m.getByRole("button", { name: "Yes, column names" });
    if (await yes.isVisible().catch(() => false)) await yes.click();
    await sleep(300);
    await dismiss(m);
    // The selected cell there: the first column of the first data row.
    await m.locator("canvas").first().click({ position: { x: 100, y: HEAD + ROW + 12 } });
  }
  await showTab(m, "s.csv");
  const g = await geometry(m);
  const head = (c) => [g.c.x + g.starts[c] + 30, g.c.y + HEAD / 2];

  // A column, copied.
  const h1 = (await facts(m, 2)).history_pos;
  await m.mouse.click(...head(1));
  await sleep(300);
  let words = await dragTo(m, ...head(1), "d.csv", false);
  ok(/Drop to copy 1 column into d\.csv, after its column 1 \(name\) \(Shift: move\)/.test(words), `the status bar says what and where during the drag (${words.slice(0, 110)})`);
  ok(await waitFor(async () => (await names(m, 2)).join() === "name,city,state", 8000), `city went in right of name (${(await names(m, 2)).join()})`);
  ok((await facts(m, 2)).history_pos === h1 + 1, "one undo step there");
  ok((await names(m, 1)).join() === "id,city,zip", "and stayed here");
  info(`message: ${await msg(m)}`);
  await m.screenshot({ path: path.join(evidence, "xtab-copied-real.png") });
  await undoIn(m, "d.csv");
  ok(await waitFor(async () => (await names(m, 2)).join() === "name,state", 8000), "one Ctrl+Z there takes it out");
  await m.keyboard.press("Control+y");
  ok(await waitFor(async () => (await names(m, 2)).join() === "name,city,state", 8000), "and Ctrl+Y puts it back");
  await m.keyboard.press("Control+z");
  await waitFor(async () => (await names(m, 2)).join() === "name,state", 8000);

  // A column, moved with Shift; each file undoes its half.
  await showTab(m, "s.csv");
  await m.mouse.click(...head(2));
  await sleep(300);
  words = await dragTo(m, ...head(2), "d.csv", true);
  ok(await waitFor(async () => (await names(m, 2)).join() === "name,zip,state" && (await names(m, 1)).join() === "id,city", 8000), `Shift moves zip (${(await names(m, 2)).join()} / ${(await names(m, 1)).join()})`);
  ok(/Ctrl\+Z in each file undoes its half/.test(await msg(m)), `the message says so (${await msg(m)})`);
  await undoIn(m, "s.csv");
  ok(await waitFor(async () => (await names(m, 1)).join() === "id,city,zip", 8000), "one Ctrl+Z here puts it back");
  ok((await invoke(m, "cell_text", { doc: 1, row: 3, col: 2 })).text === "82414", "with its values");
  await undoIn(m, "d.csv");
  ok(await waitFor(async () => (await names(m, 2)).join() === "name,state", 8000), "one Ctrl+Z there takes it out");

  // Rows, by the matching titles, then saved: the file's bytes.
  await showTab(m, "s.csv");
  await m.mouse.click(g.c.x + g.starts[0] / 2, g.c.y + HEAD + 1 * ROW + 12);
  await m.keyboard.down("Shift");
  await m.mouse.click(g.c.x + g.starts[0] / 2, g.c.y + HEAD + 2 * ROW + 12);
  await m.keyboard.up("Shift");
  await sleep(300);
  words = await dragTo(m, g.c.x + g.starts[0] / 2, g.c.y + HEAD + 1 * ROW + 12, "o.csv", false);
  ok(/Drop to copy 2 rows into o\.csv/.test(words), `rows: the words during the drag (${words.slice(0, 90)})`);
  ok(await waitFor(async () => (await facts(m, 3)).rows === 4, 8000), "two rows went in");
  ok(/under the matching titles/.test(await msg(m)), `under the matching titles (${await msg(m)})`);
  await invoke(m, "save", { doc: 3, saveAs: false, overwriteChanged: true, allowLossy: false });
  const bytes = fs.readFileSync(O, "utf8");
  ok(bytes === "zip,id,city\r\n10001,9,Nyc\r\n97622,2,Bly\r\n82414,3,Cody\r\n", `o.csv as saved (${JSON.stringify(bytes)})`);
  ok(fs.readFileSync(S, "utf8") === "id,city,zip\r\n1,Ames,50010\r\n2,Bly,97622\r\n3,Cody,82414\r\n", "s.csv untouched on disk");
  await m.screenshot({ path: path.join(evidence, "xtab-rows-real.png") });
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
