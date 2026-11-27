// Drives the REAL built app to check the rule card (C10) on a made-up file of names: Ctrl+E fills and the
// status line gives the rule and the odd results with Show the rule; Ctrl+Shift+E shows the rule as a form
// with the preview and the counts; a change is read back by the engine and Ctrl+Enter fills with it (one
// Ctrl+Z); Save as formula adds a computed column that gives what the fill gives; examples that disagree
// open the card on the one, Leave it out keeps that cell as typed; a column read as numbers greys Save as
// formula with the reason. The app's session, recent-file and recovery data are set aside and put back.
// Usage: NODE_PATH=<playwright> node scripts/real-rulecard.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-rulecard.cjs <exe> <evidence folder>");
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
const FILE = path.join(OUT, "people.csv");
fs.writeFileSync(FILE, "first,last,short\r\nAnn,Lee,A.Lee\r\nBo,Kim,B.Kim\r\nCy,Hart,\r\nDi,O'Neil,\r\nEd,,\r\nFay,12,\r\nGus,Abcdefghijklmnopqrstuvwxyz,\r\n");
const facts = (m, doc = 1) => invoke(m, "doc_info", { doc });
const status = async (m) => ((await m.textContent("footer.status").catch(() => "")) || "").replace(/\s+/g, " ").trim();
const cell = async (m, row, col) => (await invoke(m, "cell_text", { doc: 1, row, col })).text;
const column = async (m, col) => {
  const out = [];
  for (let r = 0; r < 8; r++) out.push(await cell(m, r, col));
  return out;
};
const card = (m) => m.locator('[role="dialog"]:has-text("The rule for")');
const cardText = async (m) => ((await card(m).textContent().catch(() => "")) || "").replace(/\s+/g, " ");
const toShort = async (m) => {
  await m.locator("canvas").first().click({ position: { x: 80, y: 120 } });
  await m.keyboard.press("Control+Home");
  await m.keyboard.press("ArrowRight");
  await m.keyboard.press("ArrowRight");
  await m.keyboard.press("ArrowDown");
  await sleep(200);
};
const undo = async (m) => {
  await m.locator("canvas").first().click({ position: { x: 80, y: 120 } });
  await m.keyboard.press("Control+z");
};

const proof = async () => {
  const m = await start([FILE]);
  ok(await waitFor(async () => (await facts(m)).indexed, 20000), "the file opened");
  await sleep(800);
  await dismiss(m);
  // All text, so the app does not guess a title row: say so, as a person would.
  await cmd(m, "First row is header");
  ok(await waitFor(async () => /7 rows/.test(await status(m)), 5000), `the first row holds the titles (${await status(m)})`);

  // Ctrl+E: the fill, then the status line's rule and odd results.
  await toShort(m);
  const t0 = Date.now();
  await m.keyboard.press("Control+e");
  ok(await waitFor(async () => (await cell(m, 3, 2)) === "C.Hart", 8000), `Ctrl+E fills (${Date.now() - t0} ms to the first cell)`);
  ok(await waitFor(async () => /look odd/.test(await status(m)), 8000), `the status line counts the odd results (${await status(m)})`);
  const st = await status(m);
  ok(/first letter of first \+ "\." \+ last/.test(st) && /3 results look odd \(2 shaped unlike the examples, 1 far longer or shorter\)/.test(st), "with the rule in words and why they look odd");
  ok((await column(m, 2)).join("|") === "short|A.Lee|B.Kim|C.Hart|D.O'Neil||F.12|G.Abcdefghijklmnopqrstuvwxyz", `what it wrote (${(await column(m, 2)).join("|")})`);
  await m.click('footer.status button:has-text("Show the rule")');
  ok(await waitFor(() => card(m).isVisible(), 8000), "Show the rule opens the card");
  await m.keyboard.press("Escape");
  await undo(m);
  ok(await waitFor(async () => (await cell(m, 3, 2)) === "", 8000), "one Ctrl+Z takes the fill back");

  // The card: the form, the preview, the counts; a change; Ctrl+Enter.
  await toShort(m);
  await m.keyboard.press("Control+Shift+E");
  ok(await waitFor(async () => /It fills 4 cells/.test(await cardText(m)), 10000), "Ctrl+Shift+E opens the card with its counts");
  let ct = await cardText(m);
  ok(/Showing the 5 found in the first 7 rows\./.test(ct) && /Row 7 “F\.12”: shaped unlike the examples/.test(ct), "the preview and the odd rows");
  await m.screenshot({ path: path.join(evidence, "rule-card-real.png") });
  await m.locator('select[aria-label="Letters"]').nth(1).selectOption("upper");
  ok(await waitFor(async () => /last, in capitals/.test(await cardText(m)) && /C\.HART/.test(await cardText(m)), 8000), "a change is read back and previewed by the engine");
  await m.locator('select[aria-label="Letters"]').nth(1).focus();
  await m.keyboard.press("Control+Enter");
  ok(await waitFor(async () => (await cell(m, 3, 2)) === "C.HART", 8000), "Ctrl+Enter fills with the changed rule");
  ok(!(await card(m).isVisible()), "and the card closes");
  await undo(m);
  ok(await waitFor(async () => (await cell(m, 3, 2)) === "", 8000), "one Ctrl+Z takes it back");

  // Save as formula: the engine's formula gives what the fill gives.
  await toShort(m);
  await m.keyboard.press("Control+Shift+E");
  ok(await waitFor(async () => !(await m.locator('[role="dialog"] button:has-text("Save as formula")').isDisabled()), 10000), "Save as formula can be pressed");
  await m.click('[role="dialog"] button:has-text("Save as formula")');
  ok(await waitFor(async () => (await facts(m)).cols === 4, 8000), "it adds a column");
  const f = await column(m, 3);
  ok(f.join("|") === "short (formula)|A.Lee|B.Kim|C.Hart|D.O'Neil||F.12|G.Abcdefghijklmnopqrstuvwxyz", `to the right, computing what the fill writes (${f.join("|")})`);
  await undo(m);
  ok(await waitFor(async () => (await facts(m)).cols === 3, 8000), "Ctrl+Z takes the column back");

  // Examples that disagree: the card names the one.
  await invoke(m, "set_cells", { doc: 1, cells: [[3, 2, "Hart"]] });
  await toShort(m);
  await m.keyboard.press("Control+e");
  ok(await waitFor(async () => /Row 4 does not fit the rule the other examples share: you typed “Hart”/.test(await cardText(m)), 10000), "Ctrl+E on examples that disagree opens the card on the one");
  await m.screenshot({ path: path.join(evidence, "rule-card-conflict-real.png") });
  await m.click('[role="dialog"] button:has-text("Leave it out")');
  ok(await waitFor(async () => /Row 4 is left out of learning/.test(await cardText(m)) && /It fills/.test(await cardText(m)), 10000), "Leave it out learns again");
  await m.click('[role="dialog"] button.primary');
  ok(await waitFor(async () => (await cell(m, 4, 2)) === "D.O'Neil", 8000), "Apply fills");
  ok((await cell(m, 3, 2)) === "Hart" && /Row 4 was left as you typed it\./.test(await status(m)), `the left-out row stays as typed and the status line says so (${await status(m)})`);
  await undo(m);
  await waitFor(async () => (await cell(m, 4, 2)) === "", 8000);

  // A column read as numbers: no formula form, greyed with the reason.
  await invoke(m, "set_column_type", { doc: 1, col: 1, kind: "number" });
  await toShort(m);
  await m.keyboard.press("Control+Shift+E");
  const fb = m.locator('[role="dialog"] button:has-text("Save as formula")');
  ok(await waitFor(async () => (await fb.isDisabled()) && /read as numbers or dates/.test((await fb.getAttribute("title")) || ""), 10000), `Save as formula is greyed with the reason (${await fb.getAttribute("title")})`);
  await m.keyboard.press("Escape");
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
