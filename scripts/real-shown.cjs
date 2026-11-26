// Drives the REAL built app through "only the shown rows": with rows hidden, the quick sort keeps hidden rows in their
// places, a new computed column leaves them empty, the dialogs say which rows change, and one undo takes a sort back.
// Made-up data only; the app's own session, recent-file and recovery data are set aside and put back.
// Usage: NODE_PATH=<playwright> node scripts/real-shown.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-shown.cjs <exe> <evidence folder>");
  process.exit(2);
}
const WORK = path.join(evidence, "work");
fs.mkdirSync(WORK, { recursive: true });
const DATA = path.join(process.env.APPDATA, "com.natendo.dataedit");
const LOCAL = path.join(process.env.LOCALAPPDATA, "com.natendo.dataedit");
const PORT = 9394;
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
const FILE = path.join(OUT, "shown.csv");
const BODY = ["name,n", "Eve,5", "Dee,4", "Cy,3", "Bo,2", "Ann,1", ""].join("\r\n");
fs.writeFileSync(FILE, BODY);

const proof = async () => {
  const m = await start([FILE]);
  await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed, 60000, 300);
  await sleep(1000);
  await dismiss(m);
  const yes = m.getByRole("button", { name: "Yes, column names" });
  if (await yes.isVisible().catch(() => false)) await yes.click();
  await sleep(500);
  const column = async (col) => {
    const info = await invoke(m, "doc_info", { doc: 1 });
    const out = [];
    for (let r = 1; r < info.rows; r++) out.push((await invoke(m, "cell_text", { doc: 1, row: r, col })).text);
    return out.join(" ");
  };
  const box = await m.locator("canvas").first().boundingBox();
  await m.mouse.click(box.x + 150, box.y + 36);
  await sleep(200);
  await m.keyboard.press("Control+Home");
  await m.keyboard.press("ArrowDown");
  await sleep(200);
  await m.keyboard.press("Control+9");
  await sleep(700);
  ok((await column(0)) === "Eve Cy Bo Ann", `Dee is hidden (${await column(0)})`);
  await m.keyboard.press("Control+Home");
  await cmd(m, "Sort A to Z");
  await sleep(800);
  ok((await column(0)) === "Ann Bo Cy Eve", `the shown rows are sorted (${await column(0)})`);
  const status = ((await m.textContent("footer.status")) || "").replace(/\s+/g, " ");
  ok(/1 hidden row stayed in place/.test(status), `said: ${status.slice(0, 120)}`);
  await cmd(m, "Unhide all rows");
  ok((await column(0)) === "Ann Dee Bo Cy Eve", `the hidden row kept its place (${await column(0)})`);
  await invoke(m, "change", { doc: 1, change: { op: "undo" } });
  await invoke(m, "change", { doc: 1, change: { op: "undo" } });
  await invoke(m, "change", { doc: 1, change: { op: "undo" } });
  await cmd(m, "Unhide all rows");
  ok((await column(0)) === "Eve Dee Cy Bo Ann", `undo restores the order (${await column(0)})`);
  await m.keyboard.press("Control+Home");
  await m.keyboard.press("ArrowDown");
  await m.keyboard.press("Control+9");
  await sleep(700);
  await cmd(m, "Computed column");
  const dialog = m.getByRole("dialog").last();
  await dialog.locator("textarea").first().fill('"x"');
  await sleep(900);
  ok(/1 hidden row left empty/.test((await dialog.textContent()) || ""), "the dialog says the hidden row is left empty");
  await m.screenshot({ path: path.join(evidence, "shown-compute.png") });
  await dialog.getByRole("button", { name: "Compute" }).click();
  await sleep(1500);
  await cmd(m, "Unhide all rows");
  const made = [];
  for (let r = 1; r <= 5; r++) made.push((await invoke(m, "cell_text", { doc: 1, row: r, col: 2 })).text || "-");
  ok(made.join(" ") === "x - x x x", `hidden row left empty (${made.join(" ")})`);
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
