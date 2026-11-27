// Drives the REAL built app to check three Excel/Sheets reflexes (made-up files):
// - Ctrl+Shift+V pastes what the real Windows clipboard holds into the grid, as one undo step;
// - Alt+= turns on the totals row with Sum (the status bar says so) and letting go of Alt opens no menu;
// - right-click a tab › Make a copy of this tab: an unsaved copy beside it; with the copy open, the original
//   (a file over 32 MB, so the app maps it) saves over itself, and the bytes on disk are checked;
// - Tab colour: green on a tab, and it comes back when the session is reopened after a restart.
// The app's session, recent-file and recovery data are put back.
// Usage: NODE_PATH=<playwright> node scripts/real-reflexes.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-reflexes.cjs <exe> <evidence folder>");
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
const V = path.join(OUT, "v.csv");
const BIG = path.join(OUT, "big.csv");
fs.writeFileSync(V, "id,city,amount\r\n1,Ames,10.5\r\n2,Bly,4\r\n3,Cody,7\r\n");
{
  const parts = ["id,name\r\n"];
  let n = 0;
  let len = parts[0].length;
  while (len < 34 * 1024 * 1024) {
    const line = `${n},name ${n}\r\n`;
    parts.push(line);
    len += line.length;
    n++;
  }
  fs.writeFileSync(BIG, parts.join(""));
}
const facts = (m, doc) => invoke(m, "doc_info", { doc });
const msg = async (m) => ((await m.locator("footer.status .msg").textContent({ timeout: 1000 }).catch(() => "")) || "").replace(/\s+/g, " ").trim();
const tabEl = (m, name) => m.locator("[data-tab-index]").filter({ hasText: name }).first();
const tabNames = (m) => m.$$eval("[data-tab-index] > span:first-child", (e) => e.map((x) => x.textContent.replace(/^● /, "").trim()));
const showTab = async (m, name) => {
  const b = await tabEl(m, name).boundingBox();
  await m.mouse.click(b.x + b.width / 2 - 8, b.y + b.height / 2);
  await sleep(500);
};
const clipboard = (text) => spawnSync("powershell", ["-NoProfile", "-Command", `Set-Clipboard -Value '${text}'`], { encoding: "utf8" });
const HEAD = 25;
const ROW = 25;
const clickCell = async (m, row, x = 110) => {
  const c = await m.locator("canvas").first().boundingBox();
  await m.mouse.click(c.x + x, c.y + HEAD + (row - 1) * ROW + 12);
  await sleep(250);
};
const menuItem = (m, label) => m.click(`.context [role=menuitem]:has-text("${label}")`);

const proof = async () => {
  let m = await start([V, BIG]);
  ok(await waitFor(async () => (await facts(m, 2)).indexed, 30000), "two files opened (one of 34 MB)");
  await showTab(m, "v.csv");
  const yes = m.getByRole("button", { name: "Yes, column names" });
  if (await yes.isVisible().catch(() => false)) await yes.click();
  await sleep(300);
  await dismiss(m);

  // Ctrl+Shift+V from the real clipboard.
  clipboard("Zed");
  await clickCell(m, 1, 60);
  const h = (await facts(m, 1)).history_pos;
  await m.keyboard.press("Control+Shift+v");
  ok(await waitFor(async () => (await invoke(m, "cell_text", { doc: 1, row: 1, col: 0 })).text === "Zed", 6000), "Ctrl+Shift+V pastes the Windows clipboard into the cell");
  ok((await facts(m, 1)).history_pos === h + 1, "as one undo step");
  await m.keyboard.press("Control+z");
  ok(await waitFor(async () => (await invoke(m, "cell_text", { doc: 1, row: 1, col: 0 })).text === "1", 6000), "Ctrl+Z puts it back");

  // Alt+= on the amount column.
  // The amount cell of the first data row, by the Go to box.
  await m.locator("input.goto").fill("R2 C3");
  await m.locator("input.goto").press("Enter");
  await sleep(400);
  await m.keyboard.down("Alt");
  await m.keyboard.press("Equal");
  await m.keyboard.up("Alt");
  await sleep(1500);
  const words = await msg(m);
  ok(/Totals row on: Sum of amount/.test(words), `Alt+= turns on the totals row with Sum (${words})`);
  ok((await m.$$eval("[role=menu]", (x) => x.length)) === 0, "letting go of Alt opens no menu");
  await m.screenshot({ path: path.join(evidence, "autosum-real.png") });
  await m.keyboard.press("Alt+Equal");
  await sleep(500);
  ok(/Totals row hidden/.test(await msg(m)), "Alt+= again hides it");

  // Make a copy of the big file, then save the original over itself with the copy open.
  await showTab(m, "big.csv");
  await dismiss(m);
  await tabEl(m, "big.csv").click({ button: "right" });
  await sleep(300);
  await menuItem(m, "Make a copy of this tab");
  ok(await waitFor(async () => (await tabNames(m)).includes("big.csv (copy)"), 20000), `the copy opens (${(await tabNames(m)).join(" | ")})`);
  const names = await tabNames(m);
  ok(names.indexOf("big.csv (copy)") === names.indexOf("big.csv") + 1, "beside the original");
  ok(/not saved yet/.test(await msg(m)), `the status bar says it is not saved (${await msg(m)})`);
  const copy = await facts(m, 3);
  ok(copy.name === "big.csv (copy)" && copy.modified === true && !copy.path, "doc 3 is the copy: unsaved, no file");
  await invoke(m, "set_cells", { doc: 2, cells: [[1, 1, "changed"]] });
  await sleep(300);
  ok((await invoke(m, "cell_text", { doc: 2, row: 1, col: 1 })).text === "changed", "the original is edited");
  await invoke(m, "save", { doc: 2, saveAs: false, overwriteChanged: false, allowLossy: false });
  const head = fs.readFileSync(BIG).subarray(0, 40).toString("utf8");
  ok(head.startsWith("id,name\r\n0,changed\r\n1,name 1\r\n"), `the original saved over itself with the copy open (${JSON.stringify(head)})`);
  ok((await invoke(m, "cell_text", { doc: 3, row: 1, col: 1 })).text === "name 0", "the copy still has its own rows");

  // Tab colour: green on v.csv; it comes back with the session.
  await showTab(m, "v.csv");
  await tabEl(m, "v.csv").click({ button: "right" });
  await sleep(300);
  await menuItem(m, "Tab colour…");
  await sleep(400);
  await menuItem(m, "Tab colour: green");
  await sleep(300);
  ok((await tabEl(m, "v.csv").getAttribute("aria-label")) === "v.csv, tab colour green", "the tab says its colour");
  await m.screenshot({ path: path.join(evidence, "tab-colour-real.png") });
  // Let the session be written (every 10 s); the unsaved copy has no file, so it is not in it.
  await sleep(12000);
  await killApp();
  m = await (async () => {
    wipeRecovery();
    const child = spawn(exe, [], {
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
    const pg = ctx.pages().find((p) => !p.url().includes("window=2"));
    await pg.waitForSelector("canvas, .start, button", { timeout: 30000 });
    return pg;
  })();
  await sleep(1500);
  const again = m.getByRole("button", { name: "Reopen" });
  if (await again.isVisible().catch(() => false)) await again.click();
  ok(await waitFor(async () => (await tabNames(m)).includes("v.csv"), 30000), "the session reopens v.csv");
  ok(await waitFor(async () => (await tabEl(m, "v.csv").getAttribute("aria-label")) === "v.csv, tab colour green", 15000), "with its tab colour");
  ok(!(await tabNames(m)).some((x) => /copy/.test(x)), "the unsaved copy is not in the session");
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
