// Drives the REAL built app to check Explode cells (C7) on a made-up CRM export: the detect line, the keys with
// counts, the preview, the cell that did not parse with Show those rows (a filter), JSON to columns as one undo step
// and a byte-for-byte save (1.50 and a 20-digit number kept as written), then unnest a tag list to rows with the
// whole original kept in an added column, again one step and saved byte for byte.
// The app's own session, recent-file and recovery data are set aside and put back. Made-up data only.
// Usage: NODE_PATH=<playwright> node scripts/real-explode.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-explode.cjs <exe> <evidence folder>");
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
const FILE = path.join(OUT, "crm-export.csv");
const CRLF = "\r\n";
const J1 ='"{""plan"":""gold"",""seats"":3,""price"":1.50}"';
const J2 = '"{""plan"":""free"",""seats"":null}"';
const J4 = '"{""plan"":""pro"",""price"":12345678901234567890}"';
const SRC = ["id,details,tags", `1,${J1},vip|beta`, `2,${J2},beta`, "3,{not json,vip", `4,${J4},`, ""].join(CRLF);
const WANT1 = ["id,details,details.plan,details.seats,details.price,tags", `1,${J1},gold,3,1.50,vip|beta`, `2,${J2},free,,,beta`, "3,{not json,,,,vip", `4,${J4},pro,,12345678901234567890,`, ""].join(CRLF);
const WANT2 = [
  "id,details,details.plan,details.seats,details.price,tags,tags (original)",
  `1,${J1},gold,3,1.50,vip,vip|beta`,
  `1,${J1},gold,3,1.50,beta,vip|beta`,
  `2,${J2},free,,,beta,beta`,
  "3,{not json,,,,vip,vip",
  // Not touched by the unnest: the row keeps its bytes (as Split column leaves such rows).
  `4,${J4},pro,,12345678901234567890,`,
  "",
].join(CRLF);
const body = async (m) => ((await m.textContent("body")) || "").replace(/\s+/g, " ");
const facts = (m, doc = 1) => invoke(m, "doc_info", { doc });
const opened = async (m) => {
  await waitFor(async () => (await facts(m)).indexed, 60000, 300);
  await sleep(1500);
  await dismiss(m);
};
const dlg = "[role=dialog]:has(h2:has-text('Explode cells'))";
const tid = (id) => `${dlg} [data-testid=${id}]`;
const openDialog = async (m, column) => {
  await cmd(m, "Explode cells");
  await m.waitForSelector(dlg, { timeout: 10000 });
  await m.selectOption(tid("explode-col"), { label: column });
  await m.waitForSelector(`${tid("explode-detect")}`, { timeout: 10000 });
  await waitFor(async () => (await m.textContent(tid("explode-caps")).catch(() => "")) || false);
};

const proof = async () => {
  fs.writeFileSync(FILE, SRC);
  const m = await start([FILE]);
  await opened(m);
  await openDialog(m, "details");
  const det = await waitFor(async () => {
    const t = await m.textContent(tid("explode-detect"));
    return /JSON objects/.test(t) && t;
  });
  ok(/mixed: 3 of 4 look like JSON objects/.test(det || ""), `the detect line: ${det}`);
  const keys = ((await m.textContent(tid("explode-keys"))) || "").replace(/\s+/g, " ");
  ok(/plan \(3\)/.test(keys) && /seats \(2\)/.test(keys) && /price \(2\)/.test(keys), `keys with counts: ${keys.trim()}`);
  ok(/Adds 3 columns/.test(await m.textContent(tid("explode-caps"))), "the caps line says 3 columns");
  ok(/1 cell was not valid JSON: it stays exactly as it is/.test(await m.textContent(tid("explode-bad"))), "the cell that did not parse is counted");
  const prev = await m.$$eval(`${tid("explode-preview")} tbody tr`, (t) => t.map((r) => [...r.querySelectorAll("td")].map((c) => c.textContent.trim()).join("|")));
  ok(prev[0] && prev[0].endsWith("gold|3|1.50") && prev[1].endsWith("free|(null)|(missing)"), `the preview keeps 1.50 and tells null from missing (${prev.slice(0, 2).join(" ; ")})`);
  await m.screenshot({ path: path.join(evidence, "explode-real-dialog.png") });
  await m.locator(dlg).screenshot({ path: path.join(evidence, "explode-cells.png") });
  await m.click(tid("explode-showbad"));
  await sleep(1200);
  let f = await facts(m);
  ok(f.filtered && f.rows === 2, `Show those rows: a filter of the one row and the titles (${f.rows} rows shown)`);
  await cmd(m, "Clear filter");
  await sleep(800);
  f = await facts(m);
  ok(!f.filtered && f.rows === 5, "the filter cleared");
  await openDialog(m, "details");
  await waitFor(async () => /plan/.test((await m.textContent(tid("explode-keys")).catch(() => "")) || ""));
  await m.click(tid("explode-run"));
  await sleep(1500);
  ok(/Exploded “details”: added 3 columns\. 1 cell did not parse and stayed as it was\./.test(await body(m)), "the status says what happened");
  f = await facts(m);
  ok(f.cols === 6, `3 columns added (${f.cols})`);
  await m.keyboard.press("Escape");
  await m.keyboard.press("Control+z");
  await sleep(1000);
  f = await facts(m);
  ok(f.cols === 3 && f.can_redo, "one Ctrl+Z takes the whole explode back (the filter steps before it stay)");
  await m.keyboard.press("Control+z");
  await sleep(800);
  ok((await facts(m)).cols === 3, "a second Ctrl+Z only undoes the filter, not part of the explode");
  await m.keyboard.press("Control+y");
  await sleep(800);
  await m.keyboard.press("Control+y");
  await sleep(1000);
  ok((await facts(m)).cols === 6, "Ctrl+Y redoes it");
  await m.keyboard.press("Control+s");
  await sleep(2000);
  const got1 = fs.readFileSync(FILE, "utf8");
  ok(got1 === WANT1, "saved byte for byte: 1.50 and the 20-digit number kept, the bad cell left");
  if (got1 !== WANT1) info(JSON.stringify(got1));
  await waitFor(async () => (await facts(m)).indexed, 30000, 300);
  await sleep(800);

  // Unnest the tags with the whole original kept.
  await openDialog(m, "tags");
  await m.click(`${dlg} input[type=radio][value=rows]`);
  await m.click(tid("explode-origcol"));
  const caps = await waitFor(async () => {
    const t = await m.textContent(tid("explode-caps")).catch(() => "");
    return /and 1 column/.test(t || "") && t;
  });
  ok(/Adds 1 row \(at most 10,000,000\) and 1 column/.test(caps || ""), `caps for unnest: ${caps}`);
  await m.click(tid("explode-run"));
  await sleep(1500);
  f = await facts(m);
  ok(f.rows === 6 && f.cols === 7, `a row and a column added (${f.rows} rows, ${f.cols} columns)`);
  await m.keyboard.press("Escape");
  await m.keyboard.press("Control+z");
  await sleep(1000);
  f = await facts(m);
  ok(!f.can_undo && f.rows === 5 && f.cols === 6, "one Ctrl+Z takes the rows and the column back together");
  await m.keyboard.press("Control+y");
  await sleep(1000);
  await m.keyboard.press("Control+s");
  await sleep(2000);
  const got2 = fs.readFileSync(FILE, "utf8");
  ok(got2 === WANT2, "unnested and saved byte for byte");
  if (got2 !== WANT2) info(JSON.stringify(got2));
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
