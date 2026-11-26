// Drives the REAL built app to check the total-row bar (C4b): found at open with its sums checked, Set aside then a
// byte-for-byte save, a wrong total naming its column (no Delete), Keep, and no bar for a last name "Total".
// The app's own session, recent-file and recovery data are set aside and put back. Made-up data only.
// Usage: NODE_PATH=<playwright> node scripts/real-total.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-total.cjs <exe> <evidence folder>");
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
const GOOD = path.join(OUT, "shop-totals.csv");
const BAD = path.join(OUT, "shop-totals-wrong.csv");
const NAME = path.join(OUT, "people.csv");
const ROWS = ["shop,manager,amount,visits", "Harbour St,Ada Lind,1520.40,12", "Mill Lane,Bo Varga,980.10,9", "Old Quay,Cy Okafor,1210.00,7"];
const GOOD_BODY = [...ROWS, "Total,,3710.50,28", ""].join("\r\n");
const BAD_BODY = [...ROWS, "Grand total:,,3700.00,28", ""].join("\r\n");
// A person whose last name is Total, and no number in that row: no bar.
const NAME_BODY = ["first,last,amount", "Ann,Lee,3", ",Total,", ""].join("\r\n");
const body = async (m) => ((await m.textContent("body")) || "").replace(/\s+/g, " ");
const save = async (m) => {
  await m.keyboard.press("Control+s");
  await sleep(1500);
};
const facts = (m, doc = 1) => invoke(m, "doc_info", { doc });
const opened = async (m) => {
  await waitFor(async () => (await facts(m)).indexed, 60000, 300);
  await sleep(1500);
  await dismiss(m);
};

const proof = async () => {
  fs.writeFileSync(GOOD, GOOD_BODY);
  fs.writeFileSync(BAD, BAD_BODY);
  fs.writeFileSync(NAME, NAME_BODY);
  let m = await start([GOOD]);
  await opened(m);
  ok(await waitFor(async () => /Row 5 looks like a total row \(“Total”\), and the rows above add up to it\./.test(await body(m))), "the bar finds the total row and says it adds up");
  ok((await m.getByRole("button", { name: "Delete", exact: true }).count()) === 1, "Delete is offered");
  await m.setViewportSize({ width: 1180, height: 420 }).catch(() => {});
  await sleep(400);
  await m.screenshot({ path: path.join(evidence, "total-bar.png") });
  await m.getByRole("button", { name: "Set aside", exact: true }).click();
  await sleep(900);
  let f = await facts(m);
  ok(f.rows === 4 && f.notes.below_set === 1, `Set aside: the total line is below the table (${f.rows} rows)`);
  await save(m);
  ok(fs.readFileSync(GOOD, "utf8") === GOOD_BODY, "saved byte for byte, the total line last");

  m = await start([BAD]);
  await opened(m);
  ok(await waitFor(async () => /looks like a total row \(“Grand total:”\), but amount \(the rows above make 3710\.50, it says 3700\.00\) does not add up\./.test(await body(m))), `a wrong total names the column and both figures`);
  ok((await m.getByRole("button", { name: "Delete", exact: true }).count()) === 0, "no Delete when it does not add up");
  await m.getByRole("button", { name: "Keep", exact: true }).click();
  await sleep(600);
  ok(!/looks like a total row/.test(await body(m)) && (await facts(m)).rows === 5, "Keep leaves the row");

  m = await start([NAME]);
  await opened(m);
  await sleep(1500);
  ok(!/looks like a total row/.test(await body(m)), "a last name Total with no number: no bar");
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
