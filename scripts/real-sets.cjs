// Drives the REAL built app to check Compare lists: three files, the slice counts, a slice and its keys opened as
// tabs, a sort making the result stale, a private list making the slice private, and the files left untouched.
// The app's own session, recent-file and recovery data are set aside and put back.
// Usage: NODE_PATH=<playwright> node scripts/real-sets.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-sets.cjs <exe> <evidence folder>");
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
// Made-up voter numbers.
const make = (name, lines) => {
  const f = path.join(OUT, name);
  fs.writeFileSync(f, [...lines, ""].join(CRLF));
  return f;
};
const FILES = [
  make("registered.csv", ["Voter,Name", "V1,Ada", "V2,Grace", "V3,Alan", "V4,Mary", ",Nobody"]),
  make("mailed.csv", ["Voter", "v2 ", "V3", "V9"]),
  make("returned.csv", ["Voter", "V3", "V4"]),
];
const before = FILES.map((f) => fs.readFileSync(f, "utf8"));
const status = async (m) => ((await m.textContent("footer.status")) || "").replace(/\s+/g, " ");
const body = async (m) => ((await m.textContent("body")) || "").replace(/\s+/g, " ");

const proof = async () => {
  const m = await start(FILES);
  for (const doc of [1, 2, 3]) await waitFor(async () => (await invoke(m, "doc_info", { doc })).indexed, 60000, 300);
  await sleep(1500);
  await dismiss(m);
  await cmd(m, "Compare lists");
  const dlg = m.getByRole("dialog", { name: "Compare lists" });
  await dlg.waitFor({ timeout: 8000 });
  await dlg.getByRole("button", { name: "Add a list" }).click();
  for (const [i, doc] of [1, 2, 3].entries()) await dlg.getByLabel(`Tab of list ${"ABC"[i]}`).selectOption(String(doc));
  // A tab not yet looked at may not have its title row set: the list's own tick.
  for (const i of [0, 1, 2]) await dlg.locator("fieldset").nth(i).getByRole("checkbox", { name: "First row is titles" }).check();
  for (const i of [0, 1, 2]) await dlg.locator("fieldset").nth(i).getByRole("checkbox", { name: "Voter" }).check({ timeout: 4000 });
  await dlg.getByRole("button", { name: "Compare", exact: true }).click();
  ok(await waitFor(async () => /registered: 5 rows, 4 different keys, 1 with a blank key/.test(await body(m))), "totals");
  const rows = await dlg.locator("table[aria-label=Slices] tbody tr").evaluateAll((all) => all.map((tr) => [...tr.querySelectorAll("th, td:not(.open)")].map((x) => x.textContent.trim()).join("|")));
  const want = ["registered only|1|1|0|0", "mailed only|1|0|1|0", "registered and mailed, not returned|1|1|1|0", "registered and returned, not mailed|1|1|0|1", "registered, mailed and returned|1|1|1|1"];
  ok(JSON.stringify(rows) === JSON.stringify(want), `slices: ${JSON.stringify(rows)}`);
  await m.screenshot({ path: path.join(evidence, "compare-lists.png") });
  const row = (name) => dlg.locator("table[aria-label=Slices] tbody tr", { hasText: name });
  await row("registered, mailed and returned").getByRole("button", { name: /^Open / }).click();
  ok(await waitFor(async () => (await invoke(m, "doc_info", { doc: 4 })).rows === 2), "a slice opens: title row and one voter");
  const cell = async (doc, r, c) => (await invoke(m, "cell_text", { doc, row: r, col: c })).text;
  ok((await cell(4, 1, 0)) === "V3" && (await cell(4, 1, 1)) === "Alan", "the registered row, all columns");
  const name = (await invoke(m, "doc_info", { doc: 4 })).name;
  ok(name === "registered, mailed and returned, rows of registered (capitals ignored, spaces trimmed)", `titled: ${name}`);

  // Again, with the second list private: its keys as it spells them, in a private tab.
  await invoke(m, "set_private", { doc: 2, folder: false, on: true });
  await sleep(1500);
  const states = [];
  for (const doc of [1, 2, 3]) {
    const p = await invoke(m, "private_state", { doc });
    states.push([(await invoke(m, "doc_info", { doc })).name, p.file || p.folder]);
  }
  ok(JSON.stringify(states.filter((x) => x[1]).map((x) => x[0])) === "[\"mailed.csv\"]", `only the second list is private: ${JSON.stringify(states)}`);
  await cmd(m, "Compare lists");
  await dlg.waitFor({ timeout: 8000 });
  await dlg.getByLabel("Tab of list A").selectOption("1");
  await dlg.getByLabel("Tab of list B").selectOption("2");
  for (const i of [0, 1]) await dlg.locator("fieldset").nth(i).getByRole("checkbox", { name: "First row is titles" }).check();
  for (const i of [0, 1]) await dlg.locator("fieldset").nth(i).getByRole("checkbox", { name: "Voter" }).check({ timeout: 4000 });
  await dlg.getByRole("button", { name: "Compare", exact: true }).click();
  await waitFor(async () => (await dlg.locator("table[aria-label=Slices] tbody tr").count()) === 3);
  // A sort of one list behind the screen's back: refused.
  await invoke(m, "sort_rows", { doc: 2, spec: { keys: [{ col: 0, descending: true, kind: "auto", match_case: false }], keep_first: true, read: { decimal_comma: false, date_order: "mdy" }, lists: [] } });
  await row("registered and mailed").getByRole("button", { name: /^Open / }).click();
  ok(await waitFor(async () => /run the comparison again/.test(await status(m))), "after a sort the old counts are refused");
  await dlg.getByRole("button", { name: "Compare", exact: true }).click();
  await waitFor(async () => (await dlg.locator("table[aria-label=Slices] tbody tr").count()) === 3);
  await row("registered and mailed").locator("select").selectOption("k");
  await row("registered and mailed").getByRole("button", { name: /^Open / }).click();
  ok(await waitFor(async () => (await invoke(m, "doc_info", { doc: 5 })).rows === 3), "just the keys: V2 and V3");
  const state = await invoke(m, "private_state", { doc: 5 });
  ok(state.file === true, `the slice of a private list is private (${JSON.stringify(state)})`);
  ok((await invoke(m, "private_state", { doc: 4 })).file === false, "the slice made before the list was private is not");
  await m.screenshot({ path: path.join(evidence, "compare-slice.png") });
  ok(FILES.every((f, i) => fs.readFileSync(f, "utf8") === before[i]), "the three files are untouched");
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
