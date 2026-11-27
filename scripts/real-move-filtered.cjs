// Drives the REAL built app: moving rows while a filter is on (gap #5).
// Usage: NODE_PATH=<folder with playwright> node real-move-filtered.cjs <exe> <evidence folder>
// The app's own data is backed up and put back. Made-up files only.

const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence, which = "all"] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-undo.cjs <exe> <evidence folder> [scenarios]");
  process.exit(2);
}
const WORK = path.join(evidence, "work");
fs.mkdirSync(WORK, { recursive: true });
const DATA = path.join(process.env.APPDATA, "com.natendo.dataedit");
const LOCAL = path.join(process.env.LOCALAPPDATA, "com.natendo.dataedit");
const PORT = 9405;
const T0 = Date.now();
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let failed = 0;
const say = (m) => console.log(`[${((Date.now() - T0) / 1000).toFixed(1)}s] ${m}`);
const ok = (cond, msg) => {
  if (!cond) failed++;
  say(`${cond ? "PASS" : "FAIL"} ${msg}`);
};
const info = (m) => say(`INFO ${m}`);

// ---- files and the app's own data ----
const csv = (name, rows) => {
  const p = path.join(WORK, name);
  fs.writeFileSync(p, "id,name,city\r\n" + Array.from({ length: rows }, (_, i) => `${i + 1},Person ${i + 1},Town ${i % 7}`).join("\r\n") + "\r\n");
  return p;
};
const hash = (p) => (fs.existsSync(p) ? crypto.createHash("sha256").update(fs.readFileSync(p)).digest("hex") : null);
const BACKUP = path.join(WORK, "backup");
const backup = () => {
  fs.mkdirSync(BACKUP, { recursive: true });
  for (const f of ["session.json", "recent.json"]) if (fs.existsSync(path.join(DATA, f))) fs.copyFileSync(path.join(DATA, f), path.join(BACKUP, f));
  // The person's own recovery copies are put aside (and put back at the end), so a run starts with none and the app's "recover unsaved work" offer does not cover the test.
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
  // private.json (the private-files list) is put back as it was; a list the test made is moved aside, not deleted.
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
// Only the instance this script started is ever stopped (never another one: a
// person's own running app is left alone).
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
const pageSecond = () => ctx.pages().find((p) => p.url().includes("window=2"));
const start = async (files, extraEnv = {}, exePath = exe, keepQuestion = false) => {
  await killApp();
  wipeRecovery();
  // A private-files list a test run made (private scenario) must not leak into the next scenario.
  const pj = path.join(DATA, "private.json");
  if (fs.existsSync(pj) && fs.existsSync(path.join(BACKUP, "no-private-json"))) fs.renameSync(pj, path.join(BACKUP, `private.from-run-${Date.now()}.json`));
  if (running()) throw new Error("another Natendo Data Edit is running: close it first (this test never stops an app it did not start)");
  const child = spawn(exePath, files, {
    env: { ...process.env, ...extraEnv, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT}`, WEBVIEW2_USER_DATA_FOLDER: path.join(WORK, "wv2") },
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
  if (!keepQuestion) await dismiss(m);
  return m;
};
const startKeeping = (files) => start(files, {}, exe, true);
const dismiss = async (p) => {
  const notNow = p.getByRole("button", { name: "Not now" });
  if (await notNow.isVisible().catch(() => false)) await notNow.click();
  await sleep(300);
  await p.locator(".header-ask button.close").click({ timeout: 800 }).catch(() => {});
};
const secondLaunch = async (file) => {
  spawnSync(exe, [file], { encoding: "utf8", timeout: 20000 });
  await sleep(2500);
};
const invoke = (page, cmd, args) => page.evaluate(([c, a]) => window.__TAURI_INTERNALS__.invoke(c, a), [cmd, args || {}]);
const tabNames = (page) => page.locator("[role=tab], .tab").allTextContents().then((a) => a.map((t) => t.replace(/\s+/g, " ").trim()));
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
/** Click the first data cell and type: an edit through the page, as a person makes it. */
const edit = async (page, text = "X") => {
  await page.locator("canvas").first().click({ position: { x: 140, y: 40 } });
  await page.keyboard.type(text);
  await page.keyboard.press("Enter");
  await sleep(500);
};
const dialog = (page) => page.locator(".dialog[role=alertdialog]");
const dialogText = async (page) => ((await dialog(page).textContent({ timeout: 800 }).catch(() => "")) || "").replace(/\s+/g, " ").trim();

// ---- Undo after save ----
const msg = async (page) => ((await page.locator("footer.status .msg").textContent({ timeout: 1000 }).catch(() => "")) || "").replace(/\s+/g, " ").trim();
const shot = (m, n) => m.screenshot({ path: path.join(evidence, n) });
const clean = () => {
  for (const f of ["session.json", "recent.json"]) {
    const mine = path.join(DATA, f);
    if (fs.existsSync(mine) && hash(path.join(BACKUP, f)) !== null) fs.rmSync(mine);
  }
};
const col = async (m, c, n = 30) => {
  const out = [];
  for (let r = 1; r <= n; r++) out.push((await invoke(m, "cell_text", { doc: 1, row: r, col: c })).text);
  return out;
};
// Keys go to the grid: a click on a data cell first (the side panel can hold the focus).
// Undo and Redo through the toolbar's own buttons; other keys as typed.
const key = async (m, k) => {
  const tb = k === "Control+z" ? "Undo" : k === "Control+y" ? "Redo" : null;
  if (tb) {
    const b = m.locator(`.toolbar button[title^="${tb}"]`);
    if (!(await b.isEnabled())) info(`toolbar ${tb} is disabled`);
    await b.click({ timeout: 3000 }).catch((e) => info(`toolbar ${tb}: ${String(e).slice(0, 80)}`));
  } else await m.keyboard.press(k);
  await sleep(700);
};
const modified = async (m) => (await invoke(m, "doc_info", { doc: 1 })).modified;

// ---- row heights while filtered ----
const goto = async (m, text) => {
  await m.keyboard.press("Control+g");
  await sleep(150);
  await m.keyboard.type(text);
  await m.keyboard.press("Enter");
  await sleep(500);
};
/** The height the Height dialog offers for document position `pos` (blank = none), or sets it to px. */
const height = async (m, pos, px) => {
  await goto(m, String(pos + 1));
  await cmd(m, "Height of the selected rows");
  const box = m.getByLabel("Height in pixels");
  await box.waitFor({ timeout: 3000 });
  const v = await box.inputValue();
  if (px === undefined) await m.getByRole("button", { name: "Cancel", exact: true }).last().click();
  else {
    await box.fill(String(px));
    await m.getByRole("button", { name: "OK", exact: true }).last().click();
  }
  await sleep(600);
  return v;
};
const name = async (m, pos) => (await invoke(m, "cell_text", { doc: 1, row: pos, col: 1 })).text;

const msgText = async (m) => ((await m.locator("footer.status").textContent({ timeout: 1000 }).catch(() => "")) || "").replace(/\s+/g, " ");
const shownNames = async (m, from, n) => {
  const out = [];
  for (let r = from; r < from + n; r++) out.push(await name(m, r));
  return out;
};

(async () => {
  while (running()) {
    say("waiting for the running app to close");
    await sleep(5000);
  }
  backup();
  clean();
  try {
    // 2,000 made-up rows; row p (1..2000) is Person p in Town (p-1) % 7.
    const file = csv("move.csv", 2000);
    const m = await start([file]);
    await height(m, 11, 40);
    // Filter on Town 3: shown Persons 4, 11, 18, 25, ...
    await goto(m, "5, 3");
    await cmd(m, "Filter by selected value");
    await sleep(800);
    const i1 = await invoke(m, "doc_info", { doc: 1 });
    ok(i1.filtered && i1.rows === 287, `filter on: ${i1.rows} rows shown`);
    ok(JSON.stringify(await shownNames(m, 1, 4)) === JSON.stringify(["Person 4", "Person 11", "Person 18", "Person 25"]), "shown: Persons 4, 11, 18, 25");
    // Persons 11 and 18 (hidden rows between) down one shown row.
    await goto(m, "12");
    await m.keyboard.press("Shift+ArrowDown");
    await sleep(300);
    await cmd(m, "Move rows down");
    await sleep(600);
    ok(JSON.stringify(await shownNames(m, 1, 4)) === JSON.stringify(["Person 4", "Person 25", "Person 11", "Person 18"]), `moved below Person 25 (${await shownNames(m, 1, 4)})`);
    const s1 = await msgText(m);
    ok(/Moved 2 rows\. 6 hidden rows between them stayed in place\./.test(s1), `status: ${s1.slice(0, 110)}`);
    await shot(m, "move-filtered.png");
    await cmd(m, "Clear filter");
    await sleep(800);
    const all = await shownNames(m, 1, 30);
    ok(JSON.stringify(all.slice(0, 9)) === JSON.stringify(["Person 1", "Person 2", "Person 3", "Person 4", "Person 5", "Person 6", "Person 7", "Person 8", "Person 9"]), `rows 1-9 unchanged (${all.slice(0, 9)})`);
    const p11 = all.indexOf("Person 11") + 1;
    const all40 = await shownNames(m, 1, 40);
    const q = all40.indexOf("Person 11");
    ok(JSON.stringify(all40.slice(q - 2, q + 3)) === JSON.stringify(["Person 24", "Person 25", "Person 11", "Person 18", "Person 26"]), `Persons 11 and 18 sit straight after Person 25, the row they passed; hidden Persons 26-31 stayed below (${all40.slice(q - 2, q + 3)})`);
    ok(JSON.stringify(all.filter((n) => ["Person 12", "Person 13", "Person 14", "Person 15", "Person 16", "Person 17"].includes(n))) === JSON.stringify(["Person 12", "Person 13", "Person 14", "Person 15", "Person 16", "Person 17"]), "the hidden rows between kept their order");
    ok((await height(m, p11)) === "40", "Person 11's height went with it");
    // Undo (the filter clear, then the move): every row back.
    await key(m, "Control+z");
    await key(m, "Control+z");
    await cmd(m, "Clear filter");
    await sleep(800);
    const back = await shownNames(m, 1, 30);
    ok(back.every((n, i) => n === `Person ${i + 1}`), `Undo puts every row back (${back.slice(9, 20)})`);
    await shot(m, "move-undone.png");
  } catch (e) {
    ok(false, String(e && e.stack ? e.stack : e));
  } finally {
    await killApp();
    restore();
    say(failed ? `${failed} FAILED` : "all passed");
    process.exit(failed ? 1 : 0);
  }
})();
