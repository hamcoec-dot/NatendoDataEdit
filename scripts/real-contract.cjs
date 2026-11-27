// Drives the REAL built app to check file contracts (C9) on two made-up monthly CSV files: Make a contract
// from January through its dialog (the file lands in the app's data folder holding the pattern and the ticked
// values only), then open February: the bar says what drifted, Review lists it, a count shows its rows as a
// filter chip, Fix what I can renames and orders the columns and one Ctrl+Z puts them back. The app's own
// session, recent-file, recovery data and Contracts folder are set aside and put back. Made-up data only.
// Usage: NODE_PATH=<playwright> node scripts/real-contract.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-contract.cjs <exe> <evidence folder>");
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


// ---- the Contracts folder: set aside, put back ----
const CONTRACTS = path.join(DATA, "Contracts");
const CONTRACTS_ASIDE = path.join(WORK, `contracts-aside-${Date.now()}`);
const hadContracts = fs.existsSync(CONTRACTS);
if (hadContracts) fs.renameSync(CONTRACTS, CONTRACTS_ASIDE);
const putBackContracts = () => {
  if (fs.existsSync(CONTRACTS)) fs.renameSync(CONTRACTS, path.join(WORK, `contracts-from-run-${Date.now()}`));
  if (hadContracts) fs.renameSync(CONTRACTS_ASIDE, CONTRACTS);
  say(`the Contracts folder is back as it was: ${hadContracts ? fs.existsSync(CONTRACTS) : !fs.existsSync(CONTRACTS)}`);
};

const OUT = path.join(WORK, `out-${Date.now()}`);
fs.mkdirSync(OUT, { recursive: true });
const JAN = path.join(OUT, "voters_jan.csv");
const FEB = path.join(OUT, "voters_feb.csv");
fs.writeFileSync(JAN, "id,name,party,phone\r\n1,Ann Moss,DEM,555-0100\r\n2,Bo Lane,REP,555-0101\r\n3,Cy Hart,DEM,\r\n4,Di Fox,REP,555-0103\r\n5,Ed Ray,DEM,555-0104\r\n6,Flo Oak,DEM,555-0105\r\n");
fs.writeFileSync(FEB, "id,full_name,phone,party,email\r\n7,Gus Elm,,DEM,g@example.com\r\n8,Hal Ash,,GRN,h@example.com\r\n9,Ivy Fir,555-0108,REP,i@example.com\r\n10,Jo Yew,,GRN,j@example.com\r\n");
const facts = (m, doc = 1) => invoke(m, "doc_info", { doc });
const status = async (m) => ((await m.textContent("footer.status").catch(() => "")) || "").replace(/\s+/g, " ").trim();
const names = (m, doc) => invoke(m, "first_row_names", { doc });
const banner = async (m) => ((await m.textContent('[data-testid="contract-banner"]').catch(() => "")) || "").replace(/\s+/g, " ").trim();

const proof = async () => {
  // January: make the contract through the menu entry's dialog.
  let m = await start([JAN]);
  ok(await waitFor(async () => (await facts(m)).indexed, 20000), "January opened");
  await sleep(800);
  await dismiss(m);
  ok(!(await banner(m)), "no contracts yet: no bar");
  await cmd(m, "Make a contract from this file");
  ok(await waitFor(() => m.getByText("Make a contract from this file").first().isVisible(), 8000), "Make a contract opens");
  await m.fill('input[aria-label="Contract name"]', "Monthly voters");
  await m.fill('input[aria-label="File-name pattern"]', "voters_*.csv");
  await m.click('input[aria-label="Keep the values of party"]');
  await sleep(300);
  await m.screenshot({ path: path.join(evidence, "contract-make-real.png") });
  await m.click('[role="dialog"] button.primary');
  const saved = path.join(CONTRACTS, "Monthly voters.ndcontract");
  ok(await waitFor(() => fs.existsSync(saved), 5000), `the contract file is in the app's data folder (${saved})`);
  const c = JSON.parse(fs.readFileSync(saved, "utf8"));
  ok(c.contract === 1 && c.recognise.pattern === "voters_*.csv" && c.columns[2].allowed.values.join() === "DEM,REP" && !c.columns[1].allowed, "it keeps the pattern and party's values only (no names)");
  ok(!fs.readFileSync(saved, "utf8").includes("Ann Moss"), "no other cell is in it");

  // February: the bar, Review, a count's rows, Fix and one Ctrl+Z.
  m = await start([FEB]);
  ok(await waitFor(async () => (await facts(m)).indexed, 20000), "February opened");
  ok(await waitFor(async () => /Looks like Monthly voters:/.test(await banner(m)), 15000), `the bar says what drifted (${await banner(m)})`);
  await m.screenshot({ path: path.join(evidence, "contract-banner-real.png") });
  await m.click('[data-testid="contract-banner"] button:has-text("Review")');
  ok(await waitFor(() => m.locator('[data-testid="contract-panel"]').isVisible(), 5000), "Review opens");
  const pt = ((await m.textContent('[data-testid="contract-panel"]')) || "").replace(/\s+/g, " ");
  ok(/full_name is probably name/.test(pt) && /New: email/.test(pt) && /2 cells in party: GRN/.test(pt) && /3 blank cells in phone/.test(pt), "Review lists the rename, the new column, the new values and the blanks");
  await m.screenshot({ path: path.join(evidence, "contract-review-real.png") });
  await m.click('[data-testid="contract-panel"] button.link:has-text("2 cells")');
  ok(await waitFor(async () => (await facts(m)).rows === 3, 8000), "the count shows its 2 rows (and the titles) as a filter chip");
  await m.locator("canvas").first().click({ position: { x: 300, y: 300 } });
  await m.keyboard.press("Control+z");
  ok(await waitFor(async () => (await facts(m)).rows === 5, 8000), "Ctrl+Z takes the chip off");
  await m.click('[data-testid="contract-panel"] button:has-text("Fix what I can")');
  await sleep(400);
  await m.getByRole("button", { name: "Fix", exact: true }).click();
  ok(await waitFor(async () => (await names(m, 1)).join() === "id,name,party,phone,email", 8000), `Fix renames and orders the columns (${(await names(m, 1)).join()})`);
  info(`status: ${await status(m)}`);
  await m.screenshot({ path: path.join(evidence, "contract-fixed-real.png") });
  await m.locator("canvas").first().click({ position: { x: 300, y: 300 } });
  await m.keyboard.press("Control+z");
  ok(await waitFor(async () => (await names(m, 1)).join() === "id,full_name,phone,party,email", 8000), `one Ctrl+Z puts both back (${(await names(m, 1)).join()})`);
  ok(!(await facts(m)).modified, "and the tab is unchanged again");
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
    putBackContracts();
    say(failed ? `${failed} FAILED` : "ALL PASSED");
    process.exit(failed ? 1 : 0);
  }
})();
