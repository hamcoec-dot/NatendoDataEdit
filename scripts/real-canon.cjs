// Drives the REAL built app to check the value canonicalizer (C6): a dictionary saved into the app's Dictionaries
// folder (and refused for a private file until confirmed), Edit values listing it, Load putting its merges in the
// list with the values not in it, a drag of one value onto another, Change and a byte-for-byte save. The app's own
// Dictionaries folder, session, recent and private lists are set aside and put back. Made-up data only.
// Usage: NODE_PATH=<playwright> node scripts/real-canon.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-canon.cjs <exe> <evidence folder>");
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
const DICTS = path.join(DATA, "Dictionaries");
const backup = () => {
  fs.mkdirSync(BACKUP, { recursive: true });
  if (fs.existsSync(DICTS) && !fs.existsSync(path.join(BACKUP, "Dictionaries"))) fs.renameSync(DICTS, path.join(BACKUP, "Dictionaries"));
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
  // The test's own dictionaries go to the evidence folder; the person's come back.
  if (fs.existsSync(DICTS)) fs.renameSync(DICTS, path.join(WORK, `dictionaries-run-${Date.now()}`));
  if (fs.existsSync(path.join(BACKUP, "Dictionaries"))) fs.renameSync(path.join(BACKUP, "Dictionaries"), DICTS);
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
const FILE = path.join(OUT, "counties.csv");
const SRC = ["name,county", "Ann,Hamilton", "Bo,Hamilton Co.", "Cy,Knox Co", "Dee,Knox", "Eli,Ham. County", "Fay,Blount", "Gus,Hamilton Co.", ""].join("\r\n");
const WANT = ["name,county", "Ann,Hamilton", "Bo,Hamilton", "Cy,Knox", "Dee,Knox", "Eli,Hamilton", "Fay,Blount", "Gus,Hamilton", ""].join("\r\n");
const body = async (m) => ((await m.textContent("body")) || "").replace(/\s+/g, " ");
const facts = (m, doc = 1) => invoke(m, "doc_info", { doc });
const opened = async (m) => {
  await waitFor(async () => (await facts(m)).indexed, 60000, 300);
  await sleep(1500);
  await dismiss(m);
};
const dlg = "[role=dialog]:has(h2:has-text('Edit values'))";
const row = (v) => `${dlg} tr[data-value="${v}"]`;

const proof = async () => {
  fs.writeFileSync(FILE, SRC);
  const m = await start([FILE]);
  await opened(m);
  const [dir] = await invoke(m, "dictionary_list");
  ok(/Dictionaries$/.test(dir), `the Dictionaries folder is in the app's data folder (${dir})`);
  const dict = { name: "Counties", match_case: false, trim: true, pairs: [["Hamilton Co.", "Hamilton"], ["Knox Co", "Knox"]] };
  const target = path.join(dir, "counties.ndmap");
  // A private file's values are not written without a yes.
  await invoke(m, "set_private", { doc: 1, folder: false, on: true });
  let refused = "";
  await invoke(m, "dictionary_write", { doc: 1, path: target, dict, confirmed: false }).catch((e) => (refused = String(e)));
  ok(/private/.test(refused) && !fs.existsSync(target), "a dictionary from a private file is refused until confirmed");
  await invoke(m, "set_private", { doc: 1, folder: false, on: false });
  await invoke(m, "dictionary_write", { doc: 1, path: target, dict, confirmed: false });
  const onDisk = JSON.parse(fs.readFileSync(target, "utf8"));
  ok(onDisk.kind === "natendo-dictionary" && onDisk.pairs.length === 2, "the dictionary file is written (.ndmap, 2 pairs)");
  let outside = "";
  await invoke(m, "dictionary_read", { path: FILE }).catch((e) => (outside = String(e)));
  ok(/choose the dictionary file/.test(outside), "a file outside the Dictionaries folder is only read when picked in the file window");

  await cmd(m, "Edit values");
  await m.waitForSelector(dlg, { timeout: 10000 });
  await m.locator(`${dlg} label:has-text('Column') select`).selectOption({ label: "county" });
  await m.waitForSelector(row("Blount"), { timeout: 20000 });
  ok(/Counties · 2 pairs/.test(await m.textContent(`${dlg} [data-testid=dictionary]`)), "Edit values lists the saved dictionary");
  await m.locator(`${dlg} [data-testid=dictionary] button:has-text('Load')`).click();
  await m.waitForSelector(`${dlg} [data-testid=dict-note]`);
  ok(/Loaded “Counties”: 2 pairs put in the list/.test(await m.textContent(`${dlg} [data-testid=dict-note]`)), "Load puts its 2 pairs in the list");
  await m.waitForSelector(`${dlg} [data-testid=dict-new]`);
  ok(/\(2 values, 2 rows\)/.test(await m.textContent(`${dlg} [data-testid=dict-new]`)), "values not in the dictionary: Ham. County and Blount");
  // Drag Ham. County onto Hamilton.
  const a = await m.locator(`${row("Ham. County")} .grip`).boundingBox();
  const z = await m.locator(`${row("Hamilton")} td:nth-child(2)`).boundingBox();
  await m.mouse.move(a.x + a.width / 2, a.y + a.height / 2);
  await m.mouse.down();
  await m.mouse.move(a.x + 20, a.y + 20, { steps: 3 });
  await m.mouse.move(z.x + z.width / 2, z.y + z.height / 2, { steps: 8 });
  await sleep(200);
  ok(/1 value \(1 cell\) → Hamilton/.test((await m.textContent("[data-testid=drag-chip]").catch(() => "")) || ""), "the drag chip in the real app");
  await m.screenshot({ path: path.join(evidence, "canon-real-drag.png") });
  await m.mouse.up();
  await sleep(300);
  ok((await m.inputValue(`${row("Ham. County")} input[type=text]`)) === "Hamilton", "dropped: Ham. County becomes Hamilton");
  await m.click(`${dlg} button.primary:has-text('Change')`);
  await sleep(1500);
  await m.keyboard.press("Control+s");
  await sleep(1500);
  ok(fs.readFileSync(FILE, "utf8") === WANT, "saved: the merges applied (ignoring case and spaces as the dictionary says); other rows byte for byte");
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
