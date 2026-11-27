// Drives the REAL built app to check saving JSON back (C11) on a made-up JSON Lines file: it opens through the
// import screen into a tab that saves back to it; Ctrl+S with no edits writes the same bytes; one edit changes
// only that value; a number made text is written as text and counted, and after undo is a number again; a newly
// blank cell asks once (leave the key out preselected) and the answer is remembered for the file; another
// program's change stops the save (Cancel leaves their file, Replace writes ours). The app's own session,
// recent-file and recovery data are set aside and put back. Made-up data only.
// Usage: NODE_PATH=<playwright> node scripts/real-jsonl.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-jsonl.cjs <exe> <evidence folder>");
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
const FILE = path.join(OUT, "people.jsonl");
// Made-up records, compact JSON Lines with LF, keys in column order: exact numbers (1.50, a 20-digit id),
// true/false, null, a missing key, nested values with numbers inside.
const RECS = [
  '{"id":12345678901234567890,"name":"Ada Stone","amount":1.50,"active":true,"tags":["a",2.50],"note":null}',
  '{"id":2,"name":"Bo Rivera","amount":3,"active":false,"tags":{"x":1e3}}',
  '{"id":3,"name":"Cy Okafor","amount":-0.0,"active":true,"tags":[],"note":"call"}',
];
const SRC = RECS.join("\n") + "\n";
const facts = (m, doc = 1) => invoke(m, "doc_info", { doc });
const status = async (m) => ((await m.textContent(".msg").catch(() => "")) || "").replace(/\s+/g, " ").trim();
const file = () => fs.readFileSync(FILE, "utf8");
const goTo = async (m, row, col) => {
  await m.locator("canvas").first().click({ position: { x: 300, y: 300 } });
  await m.keyboard.press("Control+Home");
  for (let i = 0; i < row; i++) await m.keyboard.press("ArrowDown");
  for (let i = 0; i < col; i++) await m.keyboard.press("ArrowRight");
  await sleep(200);
};
const type = async (m, row, col, text) => {
  await goTo(m, row, col);
  if (text === "") await m.keyboard.press("Delete");
  else {
    await m.keyboard.type(text);
    await m.keyboard.press("Enter");
  }
  await sleep(500);
};
const saveNow = async (m) => {
  await m.keyboard.press("Control+s");
  await sleep(1200);
};

const proof = async () => {
  fs.writeFileSync(FILE, SRC);
  const m = await start([FILE]);
  ok(await waitFor(() => m.getByRole("button", { name: "Import", exact: true }).isVisible(), 20000), "the .jsonl opens the import screen");
  await m.getByRole("button", { name: "Import", exact: true }).click();
  ok(await waitFor(async () => (await facts(m)).indexed, 20000), "imported");
  await sleep(1200);
  await dismiss(m);
  let f = await facts(m);
  ok(f.json_format === "jsonl" && f.json_refuse === null && f.path === null, `the tab saves back to its JSON Lines file (${f.json_format})`);
  const cells = await invoke(m, "get_rows", { doc: 1, start: 0, count: 2 }).catch(() => null);
  if (cells) info(`first rows: ${JSON.stringify(cells)}`);

  // 1. Save with no edits: the same bytes.
  await goTo(m, 0, 0);
  await saveNow(m);
  ok(file() === SRC, "Ctrl+S with no edits writes the same bytes");
  let st = await status(m);
  await waitFor(async () => /Saved 3 records/.test((st = await status(m))), 5000);
  ok(/Saved 3 records as JSON Lines\./.test(st), `status: ${st}`);

  // 2. One edit: only that value changes.
  await type(m, 1, 1, "Bo Rivers");
  await saveNow(m);
  let lines = file().split("\n");
  ok(lines[0] === RECS[0] && lines[2] === RECS[2] && lines[1] === RECS[1].replace("Bo Rivera", "Bo Rivers"), `only the edited value changed: ${lines[1]}`);
  f = await facts(m);
  ok(!f.modified && f.can_undo, "the tab is saved and keeps its undo history");
  ok(f.name === "people" && f.json_format === "jsonl", `an edit keeps the tab's name and JSON file (${f.name}, ${f.json_format})`);

  // 3. A number column given text: written as text, counted.
  await type(m, 1, 2, "abc");
  await saveNow(m);
  st = await status(m);
  ok(file().split("\n")[1].includes('"amount":"abc"') && /1 cell in number, true\/false or nested columns was not one any more and was written as text\./.test(st), `a number that is not one any more is text and counted: ${st}`);
  // Undo, save: a number again.
  await m.keyboard.press("Control+z");
  await sleep(600);
  await saveNow(m);
  ok(file().split("\n")[1] === RECS[1].replace("Bo Rivera", "Bo Rivers"), "after undo the value is a number again");

  // 4. A newly blank cell: asked once, leave out preselected.
  await type(m, 2, 1, "");
  await saveNow(m);
  const dlg = m.getByRole("dialog", { name: "Blank cells in JSON" });
  ok(await waitFor(() => dlg.isVisible(), 5000), "a newly blank cell asks how blanks are written");
  const pre = await m.evaluate(() => document.querySelector('input[name="json-blanks"]:checked')?.value);
  ok(pre === "leave_out", `leave the key out is preselected (${pre})`);
  await m.screenshot({ path: path.join(evidence, "json-blanks-real.png") });
  await m.check('input[name="json-blanks"][value="null"]');
  await dlg.getByRole("button", { name: "Save", exact: true }).click();
  await sleep(1200);
  ok(file().split("\n")[2] === RECS[2].replace('"Cy Okafor"', "null"), `the blank is written as null: ${file().split("\n")[2]}`);
  // The next save does not ask again (remembered for the file).
  await type(m, 1, 1, "");
  await saveNow(m);
  ok(!(await dlg.isVisible().catch(() => false)), "the next save does not ask again");
  st = await status(m);
  ok(/as null, as remembered for this file/.test(st) && file().split("\n")[1].includes('"name":null'), `remembered: ${st}`);

  // 5. Another program changes the file: the save stops until agreed.
  const theirs = SRC + '{"id":4,"name":"Dee Lind"}\n';
  await sleep(50);
  fs.writeFileSync(FILE, theirs);
  await type(m, 0, 1, "Ada Stone-Lee");
  await saveNow(m);
  ok(await waitFor(() => m.getByText("The file changed on disk").isVisible(), 5000), "an outside change stops the save");
  await m.getByRole("button", { name: "Cancel" }).click();
  await sleep(500);
  ok(file() === theirs, "cancelled: their file is left as it is");
  await saveNow(m);
  await waitFor(() => m.getByText("The file changed on disk").isVisible(), 5000);
  await m.getByRole("button", { name: "Replace their changes" }).click();
  await sleep(1200);
  lines = file().split("\n");
  ok(lines.length === 4 && lines[0].includes('"name":"Ada Stone-Lee"') && lines[0].startsWith('{"id":12345678901234567890,'), `replaced: ${lines[0]}`);
  await m.screenshot({ path: path.join(evidence, "jsonl-real.png") });
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
