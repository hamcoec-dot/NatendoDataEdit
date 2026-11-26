// Drives the REAL built app through "changes from a newer file" with rows hidden in both tabs: a hidden row here is
// not offered as new, a row hidden in the newer file is not offered for deletion, a change to a hidden row is marked,
// and a hidden row is not deleted unless ticked. Made-up data only; the app's own session, recent-file and recovery
// data are set aside and put back.
// Usage: NODE_PATH=<playwright> node scripts/real-updates-hidden.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-updates-hidden.cjs <exe> <evidence folder>");
  process.exit(2);
}
const WORK = path.join(evidence, "work");
fs.mkdirSync(WORK, { recursive: true });
const DATA = path.join(process.env.APPDATA, "com.natendo.dataedit");
const LOCAL = path.join(process.env.LOCALAPPDATA, "com.natendo.dataedit");
const PORT = 9393;
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
  // The person's recent files and session are moved aside before the app starts, so no screenshot can show a real
  // file name (they were copied above and are copied back afterwards).
  for (const f of ["session.json", "recent.json"]) if (fs.existsSync(path.join(DATA, f))) fs.renameSync(path.join(DATA, f), path.join(BACKUP, `${f}.aside-${Date.now()}`));
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
const HERE = path.join(OUT, "here.csv");
const NEWER = path.join(OUT, "newer.csv");
const csv = (rows) => rows.concat([""]).join("\r\n");
fs.writeFileSync(HERE, csv(["id,name,city", "1,Ann,Oslo", "2,Bo,Rome", "3,Cy,Lyon", "4,Dee,Kiel"]));
fs.writeFileSync(NEWER, csv(["id,name,city", "1,Ann,Oslo", "2,Bob,Rome", "3,Cy,Lyon", "5,Eve,Bern"]));

const proof = async () => {
  const m = await start([HERE, NEWER]);
  await waitFor(async () => (await invoke(m, "doc_info", { doc: 2 })).indexed, 60000, 300);
  await sleep(1000);
  await dismiss(m);
  const tabOf = (name) => m.locator("[role=tab]").filter({ hasText: name }).first();
  const yes = async () => {
    const y = m.getByRole("button", { name: "Yes, column names" });
    if (await y.isVisible().catch(() => false)) await y.click();
    await sleep(400);
  };
  const grid = m.locator("canvas").first();
  const hideRow = async (down) => {
    const box = await grid.boundingBox();
    await m.mouse.click(box.x + 150, box.y + 36);
    await sleep(200);
    await m.keyboard.press("Control+Home");
    for (let i = 0; i < down; i++) await m.keyboard.press("ArrowDown");
    await sleep(200);
    await m.keyboard.press("Control+9");
    await sleep(600);
  };
  // Newer tab: hide Cy (two rows below the first).
  await tabOf("newer.csv").click();
  await sleep(500);
  await yes();
  await hideRow(2);
  ok((await invoke(m, "doc_info", { doc: 2 })).hidden_rows === 1, "a row is hidden in the newer tab");
  // This tab: hide Dee (row 4), then Bo (row 2).
  await tabOf("here.csv").click();
  await sleep(500);
  await yes();
  await hideRow(3);
  await hideRow(1);
  ok((await invoke(m, "doc_info", { doc: 1 })).hidden_rows === 2, "two rows are hidden here");
  const shownNames = async (doc) => {
    const info = await invoke(m, "doc_info", { doc });
    const out = [];
    for (let r = 1; r < info.rows; r++) out.push((await invoke(m, "cell_text", { doc, row: r, col: 1 })).text);
    return JSON.stringify(out);
  };
  const [here, newer] = [await shownNames(1), await shownNames(2)];
  ok(here === JSON.stringify(["Ann", "Cy"]) && newer === JSON.stringify(["Ann", "Bob", "Eve"]), `shown: here ${here}, newer ${newer}`);
  await cmd(m, "Changes from a newer file");
  const dialog = m.getByRole("dialog");
  ok(/filters and hidden rows, here and in the newer file, are ignored/.test((await dialog.textContent()) || ""), "the setup says hidden rows are ignored");
  await dialog.locator("select").first().selectOption({ label: "newer.csv" });
  await dialog.getByRole("checkbox", { name: "id", exact: true }).check();
  await dialog.getByRole("button", { name: "Compare" }).click();
  await sleep(1500);
  const text = async () => ((await dialog.textContent()) || "").replace(/\s+/g, " ");
  ok(/1 changed cell, 1 new row \(in the newer file only\), 1 missing row/.test(await text()), `counts: ${(await text()).slice(0, 160)}`);
  ok(/In rows hidden here: 1 changed cell and 1 missing row\. ?Hidden rows are not ticked for deletion/.test(await text()), "the hidden ones are counted");
  ok(/Row 3 \(hidden\)/.test(await text()), "the changed cell is marked hidden");
  await m.screenshot({ path: path.join(evidence, "updates-hidden.png") });
  await dialog.getByRole("tab", { name: /Missing rows/ }).click();
  await sleep(300);
  await dialog.getByRole("button", { name: "Take all shown (delete them)" }).click();
  await sleep(300);
  ok(/Missing rows \(0 of 1\)/.test(await text()), "taking all shown leaves the hidden row unticked");
  await dialog.getByRole("button", { name: "Apply the ticked changes" }).click();
  await sleep(1500);
  await cmd(m, "Unhide all rows");
  await sleep(600);
  const info = await invoke(m, "doc_info", { doc: 1 });
  const rows = [];
  for (let r = 1; r < info.rows; r++) rows.push((await invoke(m, "cell_text", { doc: 1, row: r, col: 1 })).text);
  ok(JSON.stringify(rows) === JSON.stringify(["Ann", "Bob", "Cy", "Dee", "Eve"]), `applied: ${JSON.stringify(rows)}`);
  ok(fs.readFileSync(HERE, "utf8").includes("2,Bo,"), "the file is not written without Save");
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
