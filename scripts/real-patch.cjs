// Drives the REAL built app to check patch files: compare by key, the save screen, a protected patch written and
// applied (dry run, one undo step, a stale run refused, private mode on), the command line, the log without values.
// The app's own session, recent-file and recovery data are set aside and put back.
// Usage: NODE_PATH=<playwright> node scripts/real-patch.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-patch.cjs <exe> <evidence folder>");
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
// Made-up people; their words must never reach the log or a protected patch.
const make = (name, lines) => {
  const f = path.join(OUT, name);
  fs.writeFileSync(f, [...lines, ""].join(CRLF));
  return f;
};
const OLD = ["Id,Name,Town", "101,Zebedee Quill,Yarrowby", "102,Ottilie Vantree,Mossford", "103,Perrin Ashgrove,Hollin"];
const FILES = [
  make("old.csv", OLD),
  make("new.csv", ["Id,Name,Town", "101,Zebedee Quill,Brackwen", "103,Perrin Ashgrove,Hollin", "104,Isolde Fennick,Wrayholt"]),
  make("copy.csv", ["Town,Id,Name,Seen", "Hollin,103,Perrin Ashgrove,yes", "Yarrowby,101,Zebedee Quill,no", "Mossford,102,Ottilie Vantree,yes"]),
];
const before = FILES.map((f) => fs.readFileSync(f, "utf8"));
const PHRASE = "a made-up phrase 42";
const VALUES = /zebedee|quill|ottilie|vantree|ashgrove|isolde|fennick|yarrowby|mossford|brackwen|wrayholt/i;
const status = async (m) => ((await m.textContent("footer.status")) || "").replace(/\s+/g, " ");
const body = async (m) => ((await m.textContent("body")) || "").replace(/\s+/g, " ");
const LOG = path.join(LOCAL, "logs", "natendo.log");
const CLI = path.join(path.dirname(exe), "natendo-cli.exe");

const proof = async () => {
  const logStart = fs.existsSync(LOG) ? fs.statSync(LOG).size : 0;
  const m = await start(FILES);
  for (const doc of [1, 2, 3]) await waitFor(async () => (await invoke(m, "doc_info", { doc })).indexed, 60000, 300);
  await sleep(1500);
  await dismiss(m);
  const cell = async (doc, r, c) => (await invoke(m, "cell_text", { doc, row: r, col: c })).text;
  const grid = async (doc) => {
    const d = await invoke(m, "doc_info", { doc });
    const out = [];
    for (let r = 0; r < d.rows; r++) {
      const line = [];
      for (let c = 0; c < d.cols; c++) line.push(await cell(doc, r, c));
      out.push(line.join(","));
    }
    return out;
  };

  // Compare by key from the first tab, then the save screen.
  await m.locator("[role=tab], .tab").first().click({ position: { x: 20, y: 8 } });
  await sleep(1200);
  await dismiss(m);
  await cmd(m, "Compare with another open file");
  const dlg = m.getByRole("dialog");
  await dlg.getByLabel("Compare this file with").selectOption("2");
  await dlg.locator("fieldset label", { hasText: "Id" }).first().locator("input").check();
  await dlg.getByRole("button", { name: "Run" }).click();
  ok(await waitFor(async () => /Compared by key: 1 changed row/.test(await status(m))), `compared: ${(await status(m)).slice(0, 90)}`);
  await cmd(m, "Save changes as a patch");
  await dlg.waitFor({ timeout: 8000 });
  ok(/A patch contains your data/.test(await body(m)) && /finding rows by Id/.test(await body(m)), "the save screen");
  await dlg.getByLabel("Protect with a phrase").check();
  await dlg.getByLabel("Phrase", { exact: true }).fill(PHRASE);
  await dlg.getByLabel("Type it again").fill(PHRASE);
  await m.screenshot({ path: path.join(evidence, "patch-save.png") });
  await m.keyboard.press("Escape");
  await sleep(500);

  // The Save dialog is Windows' own: the command it runs is called with the path.
  const patch = path.join(OUT, "fix.ndpatch");
  const spec = { keys: [0], match_case: false, trim: true, leave_out: false };
  await invoke(m, "set_private", { doc: 2, folder: false, on: true });
  await sleep(1200);
  let said = "";
  await invoke(m, "patch_make", { a: 1, b: 2, spec, path: patch, phrase: null, plain: false }).catch((e) => (said = String(e)));
  ok(/private/.test(said) && !fs.existsSync(patch), "a private source is not saved unprotected without saying so");
  const saved = await invoke(m, "patch_make", { a: 1, b: 2, spec, path: patch, phrase: PHRASE, plain: false });
  ok(saved.changes === 1 && saved.adds === 1 && saved.removes === 1 && saved.protected, `written: ${JSON.stringify(saved)}`);
  const bytes = fs.readFileSync(patch);
  ok(!VALUES.test(bytes.toString("latin1")), "the protected file shows no value");
  ok((await invoke(m, "private_state", { doc: 1 })) && (await invoke(m, "patch_protected", { path: patch })) === true, "it is recognised as protected");

  // Dry run on the copy (another column order, an extra column).
  said = "";
  await invoke(m, "patch_preview", { doc: 3, path: patch, phrase: "the wrong phrase altogether" }).catch((e) => (said = String(e)));
  ok(/password|phrase/i.test(said), `a wrong phrase is refused: ${said.slice(0, 120)}`);
  const preview = () => invoke(m, "patch_preview", { doc: 3, path: patch, phrase: PHRASE });
  let p = await preview();
  ok(p.private && p.report.will_apply === 3 && p.report.extra.join() === "Seen", `dry run: ${p.report.will_apply} would apply, extra ${p.report.extra}`);
  ok((await grid(3)).join("|") === before[2].trim().split(CRLF).join("|") && (await invoke(m, "private_state", { doc: 3 })).file === false, "a dry run changes nothing");
  await invoke(m, "change", { doc: 3, change: { op: "undo" } }).catch(() => {});
  await invoke(m, "sort_rows", { doc: 3, spec: { keys: [{ col: 1, descending: false, kind: "auto", match_case: false }], keep_first: true, read: { decimal_comma: false, date_order: "mdy" }, lists: [] } });
  said = "";
  await invoke(m, "patch_apply", { doc: 3 }).catch((e) => (said = String(e)));
  ok(/changed since the dry run/.test(said), "after a sort the dry run is refused");
  const sorted = await grid(3);
  p = await preview();
  const done = await invoke(m, "patch_apply", { doc: 3 });
  ok(done.cells === 1 && done.added === 1 && done.removed === 1, `applied: ${JSON.stringify([done.cells, done.added, done.removed])}`);
  const want = ["Town,Id,Name,Seen", "Brackwen,101,Zebedee Quill,no", "Hollin,103,Perrin Ashgrove,yes", "Wrayholt,104,Isolde Fennick,"];
  ok((await grid(3)).join("|") === want.join("|"), `the table: ${JSON.stringify(await grid(3))}`);
  const state = await invoke(m, "private_state", { doc: 3 });
  ok(state.file === true, `private mode is on for the tab (${JSON.stringify(state)})`);
  p = await preview();
  ok(p.report.will_apply === 0 && p.report.already === 3, "a second time there is nothing to apply");
  await invoke(m, "change", { doc: 3, change: { op: "undo" } });
  ok((await grid(3)).join("|") === sorted.join("|"), "one undo takes it all back");
  ok(FILES.every((f, i) => fs.readFileSync(f, "utf8") === before[i]), "the three files are untouched");
  await sleep(800);
  const added = fs.existsSync(LOG) ? fs.readFileSync(LOG).subarray(logStart).toString("utf8") : "";
  ok(!VALUES.test(added) && !added.includes(PHRASE), `the log has no values and no phrase (${added.length} characters added)`);

  // The command line, on the same files.
  const cli = (args, input) => spawnSync(CLI, args, { input, encoding: "utf8" });
  const out = path.join(OUT, "applied.csv");
  let r = cli(["patch", "apply", patch, FILES[0], "-o", out, "--phrase-stdin"], PHRASE + CRLF);
  ok(r.status === 0 && fs.readFileSync(out, "utf8") === fs.readFileSync(FILES[1], "utf8"), `the app's patch applied by natendo-cli gives the new file (exit ${r.status}) ${r.stderr}`);
  const two = path.join(OUT, "cli.ndpatch");
  r = cli(["patch", "create", FILES[0], FILES[1], "--key", "Id", "-o", two, "--phrase-stdin"], PHRASE + CRLF);
  ok(r.status === 0, `natendo-cli patch create (exit ${r.status}) ${r.stderr}`);
  const q = await invoke(m, "patch_preview", { doc: 1, path: two, phrase: PHRASE });
  ok(q.report.will_apply === 3, "the command line's patch reads in the app");
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
