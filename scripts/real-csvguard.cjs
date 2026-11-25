// Drives the REAL built app to check the CSV export safety option: the share-safe copy and Split dialogs, both
// reports and a save of the file itself, answering the native dialogs with UI Automation and reading the files.
// The app's own session, recent-file and recovery data are set aside and put back.
// Usage: NODE_PATH=<playwright> node scripts/real-csvguard.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-csvguard.cjs <exe> <evidence folder>");
  process.exit(2);
}
const WORK = path.join(evidence, "work");
fs.mkdirSync(WORK, { recursive: true });
const DATA = path.join(process.env.APPDATA, "com.natendo.dataedit");
const LOCAL = path.join(process.env.LOCALAPPDATA, "com.natendo.dataedit");
const PORT = 9387;
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
for (const d of ["split-off", "split-on"]) fs.mkdirSync(path.join(OUT, d), { recursive: true });
const FILE = path.join(OUT, "guard.csv");
const BODY = ["name,note", "ann,=1+1", "@bob,-5", "cat,+1 555 0100", ""].join("\r\n");
const GUARDED = ["name,note", "ann,'=1+1", "'@bob,-5", "cat,'+1 555 0100", ""].join("\r\n");
fs.writeFileSync(FILE, BODY);
const read = (f) => (fs.existsSync(f) ? fs.readFileSync(f, "utf8").replace(/^﻿/, "") : "");
const status = async (m) => ((await m.textContent("footer.status")) || "").replace(/\s+/g, " ");

/** Answer the native dialog `title` with `target` while `press` opens it; true when it was pressed. */
const answer = async (title, target, press, editId = "1001") => {
  const stamp = path.join(OUT, `stamp-${Date.now()}.txt`);
  const w = spawn("pwsh", ["-NoProfile", "-File", path.join(__dirname, "uia-save-dialog.ps1"), target, stamp, "45", title, editId], { stdio: ["ignore", "pipe", "pipe"] });
  let wout = "";
  w.stdout.on("data", (d) => (wout += d));
  const done = new Promise((r) => w.on("exit", r));
  const pressed = press();
  await done;
  await Promise.race([pressed, sleep(10000)]);
  await sleep(1500);
  if (!fs.existsSync(stamp)) info(`dialog "${title}": ${wout.trim().slice(0, 300)}`);
  return fs.existsSync(stamp);
};

const proof = async () => {
  const m = await start([FILE]);
  await sleep(1500);
  await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed, 60000, 300);

  // Share-safe copy, as the dialog opens (the setting is on to start with) and unticked.
  for (const on of [true, false]) {
    const target = path.join(OUT, `safe-${on ? "on" : "off"}.csv`);
    await cmd(m, "Save a share-safe copy");
    const tick = m.getByTestId("safe-guard");
    await tick.waitFor({ timeout: 8000 });
    await sleep(800);
    // Keep every column as it is, whatever the finder suggested.
    for (const s of await m.getByRole("dialog").locator("select").all()) await s.selectOption("keep").catch(() => {});
    if (on) ok(await tick.isChecked(), "share-safe copy: the tick is on to start with");
    else await tick.uncheck();
    ok(await answer("Save a share-safe copy", target, () => m.getByRole("button", { name: "Save a share-safe copy…" }).click()), `share-safe copy (guard ${on}): the Save dialog was answered`);
    const text = read(target);
    ok(text === (on ? GUARDED : BODY), `share-safe copy (guard ${on}) reads ${JSON.stringify(text)}`);
    const s = await status(m);
    ok(/got an apostrophe/.test(s) === on, `status: ${s.slice(0, 220)}`);
  }

  // Split: unticked to start with, then ticked.
  for (const on of [false, true]) {
    const folder = path.join(OUT, `split-${on ? "on" : "off"}`);
    await cmd(m, "Split into files");
    const tick = m.getByTestId("split-guard");
    await tick.waitFor({ timeout: 8000 });
    if (on) await tick.check();
    else ok(!(await tick.isChecked()), "split: the tick is off to start with");
    await m.getByRole("dialog").getByRole("radio", { name: "Every" }).check();
    ok(await answer("Choose the folder for the new files", folder, () => m.getByRole("dialog").getByRole("button", { name: "Choose…" }).click(), "1152"), `split (guard ${on}): the folder dialog was answered`);
    await waitFor(() => fs.readdirSync(folder).length > 0, 15000);
    const files = fs.readdirSync(folder);
    const text = files.length ? read(path.join(folder, files[0])) : "";
    ok(files.length === 1 && text === (on ? GUARDED : BODY), `split (guard ${on}) wrote ${files.join()} ${JSON.stringify(text)}`);
    await sleep(800);
  }

  // The two reports, guard on and off.
  await invoke(m, "checkpoint_add", { doc: 1, name: "begin" });
  const begin = (await invoke(m, "checkpoint_list", { doc: 1 })).find((c) => c.name === "begin");
  await invoke(m, "set_cells", { doc: 1, cells: [[1, 0, "=cmd"]] });
  const reports = [
    ["list of changes", "Save the list of changes", "save_change_report", { doc: 1 }],
    ["since the checkpoint", "Save what changed since the checkpoint", "checkpoint_since_report", { doc: 1, checkpoint: begin.id, header: true }],
  ];
  for (const [what, title, command, args] of reports) {
    for (const on of [true, false]) {
      const target = path.join(OUT, `${command}-${on ? "on" : "off"}.csv`);
      let rep = null;
      ok(await answer(title, target, () => invoke(m, command, { ...args, guard: on }).then((r) => (rep = r))), `${what} (guard ${on}): the Save dialog was answered`);
      await waitFor(() => rep, 10000);
      const text = read(target);
      ok(text.includes(on ? ",ann,'=cmd" : ",ann,=cmd") && rep && rep.guarded === (on ? 1 : 0), `${what} (guard ${on}): ${JSON.stringify(rep)} ${JSON.stringify(text).slice(0, 200)}`);
    }
  }

  // Saving the file itself adds nothing.
  await invoke(m, "save", { doc: 1, saveAs: false, overwriteChanged: true, allowLossy: false }).catch((e) => info(String(e).slice(0, 200)));
  await sleep(1000);
  ok(read(FILE) === BODY.replace("ann,", "=cmd,"), `the saved file: ${JSON.stringify(read(FILE))}`);
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
