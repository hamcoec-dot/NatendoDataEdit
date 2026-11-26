// Drives the REAL built app to check golden records: match one list with itself, accept, merge with an exact
// sum and a highlighted doubt, the new tabs, the source untouched, private results, a changed table refused, the
// log without values.
// The app's own session, recent-file and recovery data are set aside and put back.
// Usage: NODE_PATH=<playwright> node scripts/real-golden.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-golden.cjs <exe> <evidence folder>");
  process.exit(2);
}
const WORK = path.join(evidence, "work");
fs.mkdirSync(WORK, { recursive: true });
const DATA = path.join(process.env.APPDATA, "com.natendo.dataedit");
const LOCAL = path.join(process.env.LOCALAPPDATA, "com.natendo.dataedit");
const PORT = 9420;
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
  // Copied, then moved aside for the run: a screenshot must never show the
  // person's own recent files or tabs.
  for (const f of ["session.json", "recent.json"]) {
    fs.rmSync(path.join(BACKUP, f), { force: true });
    if (fs.existsSync(path.join(DATA, f))) {
      fs.copyFileSync(path.join(DATA, f), path.join(BACKUP, f));
      fs.rmSync(path.join(DATA, f));
    }
  }
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
    else fs.rmSync(path.join(DATA, f), { force: true });
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
const CRLF = String.fromCharCode(13, 10);
// Made-up people; their words must never reach the log.
const FILE = path.join(OUT, "members.csv");
const LINES = [
  "Member Id,Name,Born,Town,Paid",
  "0101,Zebedee Quillfeather,1980-05-13,Yarrowby,0.10",
  "0102,Ottilie Vantree,1975-01-02,Mossford,$5",
  "0103,Perrin Ashgrove,1990-11-30,Hollin,2",
  "0104,Zebedee Quillfeather,1980-05-13,,0.20",
  "0105,Ottilie Vantree,1975-01-02,Brackwen,1",
  "0106,Corwin Dellamar,1968-07-07,Wrayholt,3",
  "",
];
const BODY = LINES.join(CRLF);
fs.writeFileSync(FILE, BODY);
const VALUES = /zebedee|quillfeather|ottilie|vantree|ashgrove|dellamar|yarrowby|mossford|brackwen|wrayholt/i;
const status = async (m) => ((await m.textContent("footer.status")) || "").replace(/\s+/g, " ");
const body = async (m) => ((await m.textContent("body")) || "").replace(/\s+/g, " ");
const LOG = path.join(LOCAL, "logs", "natendo.log");

const proof = async () => {
  const logStart = fs.existsSync(LOG) ? fs.statSync(LOG).size : 0;
  const m = await start([FILE]);
  await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed, 60000, 300);
  await sleep(1500);
  await dismiss(m);
  const grid = async (doc) => {
    const d = await invoke(m, "doc_info", { doc });
    const out = [];
    for (let r = 0; r < d.rows; r++) {
      const line = [];
      for (let c = 0; c < d.cols; c++) line.push((await invoke(m, "cell_text", { doc, row: r, col: c })).text);
      out.push(line.join(","));
    }
    return out;
  };
  await cmd(m, "Match people");
  const dlg = m.getByRole("dialog");
  await dlg.waitFor({ timeout: 8000 });
  await dlg.getByLabel("Match with").selectOption("-1");
  await sleep(500);
  const find = dlg.getByRole("button", { name: "Find matches" });
  if (await find.isDisabled()) await dlg.getByRole("button", { name: "Add a column to compare" }).click();
  await dlg.getByLabel("Column in this table").first().selectOption("1");
  await find.click();
  ok(await waitFor(async () => /2 pairs found/.test(await body(m))), `matched: ${(await body(m)).match(/\d+ pairs? found/)}`);
  await dlg.getByLabel(/Accept every pair scoring/).fill("80");
  await dlg.getByRole("button", { name: "Accept", exact: true }).click();
  await dlg.getByRole("button", { name: "Merge accepted into golden records…" }).click();
  ok(await waitFor(async () => /Merge into golden records/.test(await body(m))), "the golden screen");
  await dlg.getByLabel("Rule for Paid").selectOption("sum");
  await dlg.getByRole("button", { name: "Preview", exact: true }).click();
  ok(await waitFor(async () => /2 clusters would merge: 2 rows folded away, 4 rows in the result/.test(await body(m))), "the preview counts");
  ok((await dlg.locator("tr.doubt").count()) === 1 && /not an exact number/.test(await body(m)), "the money value is highlighted");
  ok(/Cluster 1: 0101; 0104/.test(await body(m)), "members named by id, leading zeros kept");
  await m.screenshot({ path: path.join(evidence, "golden-preview.png") });
  await dlg.getByRole("button", { name: "Create the new table" }).click();
  ok(await waitFor(async () => /Golden records made as a new tab: 2 clusters merged, 4 rows/.test(await status(m))), `created: ${(await status(m)).slice(0, 120)}`);
  const tabs = [];
  for (const doc of [2, 3]) tabs.push([(await invoke(m, "doc_info", { doc })).name, await grid(doc)]);
  const result = tabs.find((t) => /^Golden records/.test(t[0]));
  const away = tabs.find((t) => /^Merged away/.test(t[0]));
  const want = ["Member Id,Name,Born,Town,Paid,Merged_IDs,Merged_Count", "0101,Zebedee Quillfeather,1980-05-13,Yarrowby,0.30,0101; 0104,2", "0102,Ottilie Vantree,1975-01-02,Mossford,$5,0102; 0105,2", "0103,Perrin Ashgrove,1990-11-30,Hollin,2,,", "0106,Corwin Dellamar,1968-07-07,Wrayholt,3,,"];
  ok(result && result[1].join("|") === want.join("|"), `the result: ${JSON.stringify(result && result[1])}`);
  ok(away && away[1].length === 3 && away[1][1] === "1,5,0104,Zebedee Quillfeather,1980-05-13,,0.20", `merged away: ${JSON.stringify(away && away[1])}`);
  ok((await grid(1)).join("|") === LINES.slice(0, -1).join("|") && fs.readFileSync(FILE, "utf8") === BODY, "the source table and file are unchanged");
  ok(!(await invoke(m, "doc_info", { doc: 1 })).modified, "and not marked changed");

  // A private source gives private tabs; a table changed since the match is refused.
  await invoke(m, "set_private", { doc: 1, folder: false, on: true });
  await sleep(1200);
  const spec = { fields: [{ col: 1, other_col: 1, kind: "name", weight: 1 }], keep_first: true, other_header: true, min_score: 80, read: { decimal_comma: false, date_order: "mdy" } };
  const rep = await invoke(m, "match_find", { doc: 1, other: null, spec, bring: [] });
  const accepted = rep.listed.map((c) => [c.id, c.other_row, c.score]);
  const golden = { rules: [], id_col: 0, date_col: null, max_cluster: 10, require_all: false, skip: [], keep_first: true, read: spec.read };
  const sum = await invoke(m, "golden_plan", { doc: 1, accepted, spec: golden });
  const made = await invoke(m, "golden_create", { doc: 1, away: false });
  const state = await invoke(m, "private_state", { doc: made.info.id });
  ok(sum.counts.clusters === 2 && state.file === true, `private source, private result (${JSON.stringify(state)})`);
  await invoke(m, "sort_rows", { doc: 1, spec: { keys: [{ col: 1, descending: true, kind: "auto", match_case: false }], keep_first: true, read: { decimal_comma: false, date_order: "mdy" }, lists: [] } });
  let said = "";
  await invoke(m, "golden_plan", { doc: 1, accepted, spec: golden }).catch((e) => (said = String(e)));
  ok(/run Match people again/.test(said), "after a sort the match is refused");
  said = "";
  await invoke(m, "golden_create", { doc: 1, away: false }).catch((e) => (said = String(e)));
  ok(/run Match people again/.test(said), "and so is the planned result");
  await invoke(m, "change", { doc: 1, change: { op: "undo" } });
  await sleep(800);
  const added = fs.existsSync(LOG) ? fs.readFileSync(LOG).subarray(logStart).toString("utf8") : "";
  ok(!VALUES.test(added), `the log has no values (${added.length} characters added)`);
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
