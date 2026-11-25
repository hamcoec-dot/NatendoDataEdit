// Drives the REAL built app (not the browser preview) over WebView2 remote debugging to check the data rules
// on a 1,000,000-row file of made-up voters with 10,000+ problems: rules kept beside the file load when it opens
// (and run nothing on their own), "Check now" over the whole table, F7 / Shift+F7 (timed), "Suggest rules from
// this data" (timed), opening a file with a rules file beside it against one without, and scroll frame / draw
// time with the problems drawn as fill or underline on the worst screen (every cell of 100 rows marked).
//
// Usage (after `npx tauri build`):
//   NODE_PATH=<folder with playwright> node scripts/real-lint.cjs <exe> <evidence folder> [scenarios]
// scenarios: a comma list of rules, opentime, frames (default: all). The app's own data (session.json,
// recent.json, private.json, recovery copies) is backed up and put back; the app runs with its own browser
// profile under the evidence folder. Files are made-up and live in <evidence>/work. Nothing is installed.
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence, which = "all"] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-lint.cjs <exe> <evidence folder> [scenarios]");
  process.exit(2);
}
const WORK = path.join(evidence, "work");
fs.mkdirSync(WORK, { recursive: true });
const DATA = path.join(process.env.APPDATA, "com.natendo.dataedit");
const LOCAL = path.join(process.env.LOCALAPPDATA, "com.natendo.dataedit");
const PORT = 9367;
const T0 = Date.now();
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let failed = 0;
const say = (m) => console.log(`[${((Date.now() - T0) / 1000).toFixed(1)}s] ${m}`);
const ok = (cond, msg) => {
  if (!cond) failed++;
  say(`${cond ? "PASS" : "FAIL"} ${msg}`);
};
const info = (m) => say(`INFO ${m}`);

// ---- files and the app's own data ----
const csv = (name, rows) => {
  const p = path.join(WORK, name);
  fs.writeFileSync(p, "id,name,city\r\n" + Array.from({ length: rows }, (_, i) => `${i + 1},Person ${i + 1},Town ${i % 7}`).join("\r\n") + "\r\n");
  return p;
};
const first = csv("first.csv", 50);
const second = csv("second.csv", 50);
const third = csv("third.csv", 50);
const hash = (p) => (fs.existsSync(p) ? crypto.createHash("sha256").update(fs.readFileSync(p)).digest("hex") : null);
const BACKUP = path.join(WORK, "backup");
const backup = () => {
  fs.mkdirSync(BACKUP, { recursive: true });
  for (const f of ["session.json", "recent.json"]) if (fs.existsSync(path.join(DATA, f))) fs.copyFileSync(path.join(DATA, f), path.join(BACKUP, f));
  // The person's own recovery copies are put aside (and put back at the end), so a run starts with none and the app's "recover unsaved work" offer does not cover the test.
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
  // private.json (the private-files list) is put back as it was; a list the test made is moved aside, not deleted.
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
// Only the instance this script started is ever stopped (never another one: a
// person's own running app is left alone).
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
const pageMain = () => ctx.pages().find((p) => !p.url().includes("window=2"));
const pageSecond = () => ctx.pages().find((p) => p.url().includes("window=2"));
const start = async (files, extraEnv = {}, exePath = exe) => {
  await killApp();
  wipeRecovery();
  // A private-files list a test run made (private scenario) must not leak into the next scenario.
  const pj = path.join(DATA, "private.json");
  if (fs.existsSync(pj) && fs.existsSync(path.join(BACKUP, "no-private-json"))) fs.renameSync(pj, path.join(BACKUP, `private.from-run-${Date.now()}.json`));
  if (running()) throw new Error("another Natendo Data Edit is running: close it first (this test never stops an app it did not start)");
  const child = spawn(exePath, files, {
    env: { ...process.env, ...extraEnv, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT}`, WEBVIEW2_USER_DATA_FOLDER: path.join(WORK, "wv2") },
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
  const m = pageMain();
  await m.waitForSelector("canvas, .start, button", { timeout: 30000 });
  await sleep(1500);
  await dismiss(m);
  return m;
};
const dismiss = async (p) => {
  const notNow = p.getByRole("button", { name: "Not now" });
  if (await notNow.isVisible().catch(() => false)) await notNow.click();
  await sleep(300);
  await p.locator(".header-ask button.close").click({ timeout: 800 }).catch(() => {});
};
const secondLaunch = async (file) => {
  spawnSync(exe, [file], { encoding: "utf8", timeout: 20000 });
  await sleep(2500);
};
const invoke = (page, cmd, args) => page.evaluate(([c, a]) => window.__TAURI_INTERNALS__.invoke(c, a), [cmd, args || {}]);
const tabNames = (page) => page.locator("[role=tab], .tab").allTextContents().then((a) => a.map((t) => t.replace(/\s+/g, " ").trim()));
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
/** Click the first data cell and type: an edit through the page, as a person makes it. */
const edit = async (page, text = "X") => {
  await page.locator("canvas").first().click({ position: { x: 140, y: 40 } });
  await page.keyboard.type(text);
  await page.keyboard.press("Enter");
  await sleep(500);
};
const dialog = (page) => page.locator(".dialog[role=alertdialog]");
const dialogText = async (page) => ((await dialog(page).textContent({ timeout: 800 }).catch(() => "")) || "").replace(/\s+/g, " ").trim();
const osClose = (title) => {
  // Top-level windows of the app's process, matched by exact title and visibility (a hidden helper window can share the title).
  const ps =
    `Add-Type 'using System;using System.Text;using System.Collections.Generic;using System.Runtime.InteropServices;public class W{public delegate bool CB(IntPtr h,IntPtr l);[DllImport("user32.dll")]public static extern bool EnumWindows(CB cb,IntPtr l);[DllImport("user32.dll")]public static extern int GetWindowText(IntPtr h,StringBuilder s,int n);[DllImport("user32.dll")]public static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);[DllImport("user32.dll")]public static extern bool IsWindowVisible(IntPtr h);[DllImport("user32.dll")]public static extern bool PostMessage(IntPtr h,uint m,IntPtr w,IntPtr l);public static List<IntPtr> Find(uint pid,string t){var r=new List<IntPtr>();EnumWindows((h,l)=>{uint p;GetWindowThreadProcessId(h,out p);var sb=new StringBuilder(256);GetWindowText(h,sb,256);if(p==pid&&IsWindowVisible(h)&&sb.ToString()==t)r.Add(h);return true;},IntPtr.Zero);return r;}}'; ` +
    `$pids=@(Get-Process -Name natendo-data-edit -ErrorAction SilentlyContinue | ForEach-Object { $_.Id }); $f=$null; foreach ($i in $pids) { $x=[W]::Find([uint32]$i,'${title}'); if ($x.Count -gt 0) { $f=$x[0] } }; ` +
    `if ($f -eq $null) { 'no window' } else { [void][W]::PostMessage($f,0x10,[IntPtr]::Zero,[IntPtr]::Zero); 'sent' }`;
  return spawnSync("powershell", ["-NoProfile", "-Command", ps], { encoding: "utf8" }).stdout.trim();
};
/** Minimise (6) or restore (9) a window of the app by its exact title. */
const osShow = (title, cmd) => {
  const ps =
    `Add-Type 'using System;using System.Text;using System.Collections.Generic;using System.Runtime.InteropServices;public class V{public delegate bool CB(IntPtr h,IntPtr l);[DllImport("user32.dll")]public static extern bool EnumWindows(CB cb,IntPtr l);[DllImport("user32.dll")]public static extern int GetWindowText(IntPtr h,StringBuilder s,int n);[DllImport("user32.dll")]public static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);[DllImport("user32.dll")]public static extern bool ShowWindow(IntPtr h,int c);[DllImport("user32.dll")]public static extern bool IsIconic(IntPtr h);public static string Do(uint pid,string t,int c){string r="no window";EnumWindows((h,l)=>{uint p;GetWindowThreadProcessId(h,out p);var sb=new StringBuilder(256);GetWindowText(h,sb,256);if(p==pid&&sb.ToString()==t){ShowWindow(h,c);r="done iconic="+IsIconic(h);}return true;},IntPtr.Zero);return r;}}'; ` +
    `$o='no window'; foreach ($i in @(Get-Process -Name natendo-data-edit -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })) { $x=[V]::Do([uint32]$i,'${title}',${cmd}); if ($x -ne 'no window') { $o=$x } }; $o`;
  return spawnSync("powershell", ["-NoProfile", "-Command", ps], { encoding: "utf8" }).stdout.trim();
};
const CLOSE_MAIN = () => osClose("Natendo Data Edit");
const CLOSE_W2 = () => osClose("Natendo Data Edit (window 2)");
const mem = () => {
  const r = spawnSync("powershell", ["-NoProfile", "-Command", `$p=Get-CimInstance Win32_Process | Where-Object { $_.CommandLine -match 'wv2' -and $_.Name -match 'msedgewebview2' }; $ids=$p.ProcessId; $m=(Get-Process -Id $ids -ErrorAction SilentlyContinue | Measure-Object -Property PrivateMemorySize64 -Sum).Sum; "$($ids.Count) $([math]::Round($m/1MB))"`], { encoding: "utf8" });
  const [n, mb] = r.stdout.trim().split(/\s+/);
  return { procs: Number(n), mb: Number(mb) };
};
/** Open window 2 with the tab the main window is showing. */
const moveActiveToSecond = async (m) => {
  const wait = ctx.waitForEvent("page", { timeout: 20000 });
  await cmd(m, "Move tab to a new window");
  const w2 = await wait;
  await w2.waitForLoadState("load").catch(() => {});
  await waitFor(async () => (await tabNames(w2)).length > 0, 20000);
  await sleep(500);
  return w2;
};
const listen = (page, names, label, list) =>
  page.evaluate(async ([names, label, list]) => {
    window[list] = window[list] || [];
    const { invoke, transformCallback } = window.__TAURI_INTERNALS__;
    for (const event of names) {
      const handler = transformCallback((e) => window[list].push(e.event));
      await invoke("plugin:event|listen", { event, target: { kind: "WebviewWindow", label }, handler });
    }
  }, [names, label, list]);
const got = (page, list) => page.evaluate((l) => (window[l] || []).slice(), list);

// ---- scenarios: Ctrl+click selection of several blocks ----
const people = path.join(WORK, "people.csv");
fs.writeFileSync(people, "id,name,score\r\n" + Array.from({ length: 80 }, (_, i) => `${i + 1},Person ${i + 1},${(i + 1) * 3}`).join("\r\n") + "\r\n");
const cell = async (page, doc, row, col) => (await invoke(page, "cell_text", { doc, row, col })).text;
const msg = async (page) => ((await page.locator("footer.status .msg").textContent({ timeout: 1000 }).catch(() => "")) || "").replace(/\s+/g, " ").trim();
const foot = async (page) => ((await page.locator("footer.status").textContent({ timeout: 1000 }).catch(() => "")) || "").replace(/\s+/g, " ").trim();
/** Column starts found by clicking along a body row (3 columns). */
const geom = async (page) => {
  const c = await page.locator("canvas").first().boundingBox();
  const HEAD = 25;
  const ROW = 25;
  const starts = [];
  let last = 0;
  for (let x = 30; x < 700 && starts.length < 3; x += 2) {
    await page.mouse.click(c.x + x, c.y + HEAD + ROW + 12);
    const m = /C(\d+)$/.exec(await page.locator("input.goto").inputValue());
    const col = m ? Number(m[1]) : 0;
    if (col > last) {
      starts.push(x);
      last = col;
    }
  }
  starts.push(starts[2] + (starts[2] - starts[1]));
  return { c, starts, HEAD, ROW };
};
const cellXY = (g, row, col) => [g.c.x + (g.starts[col] + g.starts[col + 1]) / 2, g.c.y + g.HEAD + row * g.ROW + g.ROW / 2];
const ctrlClickAt = async (page, x, y) => {
  await page.keyboard.down("Control");
  await page.mouse.click(x, y);
  await page.keyboard.up("Control");
};
const dismissAsk = async (m) => {
  const yes = m.getByRole("button", { name: "Yes, column names" });
  if (await yes.isVisible().catch(() => false)) await m.keyboard.press("Alt+Shift+Y");
  await sleep(500);
};



/** A made-up voter file: the first 100 rows break every rule; then a bad ZIP on every 125th row and a bad date on every 500th. */
const N = 1_000_000;
const BIG = path.join(WORK, "rules-1m.csv");
const PLAIN = path.join(WORK, "plain-1m.csv");
let expectedProblems = 0;
let lastBad = 0;
{
  const lines = ["id,name,zip,joined,city"];
  let probs = 0;
  for (let i = 0; i < N; i++) {
    const early = i < 100;
    const zipBad = early || i % 125 === 7;
    const dateBad = early || i % 500 === 11;
    const nameShort = early;
    probs += (zipBad ? 2 : 0) + (dateBad ? 1 : 0) + (nameShort ? 1 : 0);
    if (zipBad || dateBad || nameShort) lastBad = i + 1; // shown row number (row 0 is the title row)
    const mm = String(1 + (i % 12)).padStart(2, "0");
    const dd = String(1 + (i % 28)).padStart(2, "0");
    lines.push(`${i + 1},${nameShort ? "P" + i : "Person " + (i % 997)},${zipBad ? "1234" : 10000 + (i % 90000)},${dateBad ? "02/03/2024" : `2024-${mm}-${dd}`},Town ${i % 31}`);
  }
  expectedProblems = probs;
  if (!fs.existsSync(BIG)) fs.writeFileSync(BIG, lines.join("\r\n") + "\r\n");
  if (!fs.existsSync(PLAIN)) fs.copyFileSync(BIG, PLAIN);
}
const SIDECAR = BIG + ".ndchecks";
const RULES = {
  version: 1,
  keep_first: true,
  rules: [
    { col: 1, title: "name", kind: { type: "required" } },
    { col: 2, title: "zip", name: "5-digit ZIP", kind: { type: "pattern", pattern: { text: "[0-9]{5}", kind: "regex", match_case: false, whole_cell: true } } },
    { col: 2, title: "zip", kind: { type: "length", min: 5, max: 5 } },
    { col: 3, title: "joined", kind: { type: "date_iso" } },
    { col: 0, title: "id", kind: { type: "unique", match_case: false } },
    { kind: { type: "expression", expr: "LEN([name]) > 3" } },
  ],
};
fs.writeFileSync(SIDECAR, JSON.stringify(RULES, null, 2));
const nameBox = (m) => m.locator("input.goto").inputValue();
const firstDataCell = async (m) => {
  const c = await m.locator("canvas").first().boundingBox();
  await m.mouse.click(c.x + 140, c.y + 25 + 12);
  return c;
};
const loaded = async (m) => waitFor(async () => new RegExp(`${N.toLocaleString("en-US")} rows`).test(await foot(m)), 120000, 100);
const ensureRows = async (m) => {
  await loaded(m);
  await dismissAsk(m);
};

const scenarios = {
  async rules() {
    const m = await start([BIG]);
    await ensureRows(m);
    ok(!!(await waitFor(async () => /6 rules/.test(await foot(m)), 20000, 200)), `the rules beside the file loaded (${(await foot(m)).slice(0, 160)})`);
    await sleep(3000);
    const f0 = await foot(m);
    ok(/6 rules/.test(f0) && /Check now/.test(f0) && !/\d problems/.test(f0), `loading ran nothing on its own: rules shown, Check now offered, no problem count (${f0.slice(0, 160)})`);
    await m.screenshot({ path: path.join(evidence, "rules-loaded-real.png") });
    // Check now over the whole table.
    const t0 = Date.now();
    await m.getByRole("button", { name: "Check now" }).click();
    const done = await waitFor(async () => /6 rules, [\d,]+ problems/.test(await foot(m)), 60000, 20);
    const ms = Date.now() - t0;
    const f1 = await foot(m);
    const got = Number(((/6 rules, ([\d,]+) problems/.exec(f1) || [])[1] || "0").replace(/,/g, ""));
    ok(!!done && got === expectedProblems, `Check now found ${got.toLocaleString()} problems (expected ${expectedProblems.toLocaleString()}) in about ${ms} ms over ${N.toLocaleString()} rows`);
    // F7 from the top.
    await firstDataCell(m);
    await m.keyboard.press("Control+Home");
    await sleep(300);
    const before = await nameBox(m);
    const t1 = Date.now();
    await m.keyboard.press("F7");
    const moved = await waitFor(async () => (await nameBox(m)) !== before, 5000, 10);
    const jms = Date.now() - t1;
    const at = await nameBox(m);
    ok(!!moved && /^R2 C3$/.test(at), `F7 from the top went to the first problem ${at} in about ${jms} ms (expected R2 C3: the ZIP of the first row)`);
    info(`status after F7: ${(await foot(m)).slice(0, 200)}`);
    // Shift+F7 from the top wraps to the very last problem (a miss over the whole file).
    await m.keyboard.press("Control+Home");
    await sleep(300);
    const b2 = await nameBox(m);
    const t2 = Date.now();
    await m.keyboard.press("Shift+F7");
    const wrapped = await waitFor(async () => (await nameBox(m)) !== b2, 10000, 10);
    const wms = Date.now() - t2;
    const at2 = await nameBox(m);
    ok(!!wrapped && new RegExp(`^R${lastBad + 1} `).test(at2), `Shift+F7 went back round to the last problem ${at2} in about ${wms} ms (expected row ${lastBad + 1})`);
    ok(/back at the top/.test(await foot(m)), "and said it wrapped");
    ok(jms < 300 && wms < 1500, `jump times: next ${jms} ms, a miss round 1M rows ${wms} ms`);
    // Suggest rules from this data on the ZIP column.
    await m.locator("canvas").first().click({ position: { x: 300, y: 40 } });
    await cmd(m, "Data checks");
    await sleep(600);
    const t3 = Date.now();
    await m.getByRole("button", { name: "Suggest rules from this data" }).click();
    const sug = await waitFor(async () => (await m.locator("[aria-label='Suggested rules'] label").count()) > 0, 10000, 20);
    const sms = Date.now() - t3;
    const texts = await m.locator("[aria-label='Suggested rules'] label").allTextContents();
    ok(!!sug && sms < 1500, `Suggest rules answered in about ${sms} ms with ${texts.length} suggestions`);
    info(`suggestions: ${texts.map((t) => t.replace(/\s+/g, " ").trim()).join(" | ").slice(0, 300)}`);
    await m.getByRole("button", { name: "Cancel" }).click();
  },

  // Opening a file with a rules file beside it must cost no more than 5% over the same file without one.
  async opentime() {
    const t = { with: [], without: [] };
    for (let i = 0; i < 5; i++) {
      for (const kind of i % 2 ? ["without", "with"] : ["with", "without"]) {
        const t0 = Date.now();
        const m = await start([kind === "with" ? BIG : PLAIN]);
        await loaded(m);
        t[kind].push(Date.now() - t0);
        await killApp();
      }
    }
    const best = (a) => Math.min(...a);
    const pct = ((best(t.with) - best(t.without)) / best(t.without)) * 100;
    info(`open to all rows shown, best of 5: with a rules file ${best(t.with)} ms (${t.with.join(", ")}), without ${best(t.without)} ms (${t.without.join(", ")}): ${pct.toFixed(1)}%`);
    ok(pct <= 5, `a rules file beside the file adds ${pct.toFixed(1)}% to the open time (limit 5%)`);
  },

  // Scroll frame time and draw time with the problems drawn: the worst screen (top 100 rows all marked) and spread over the file.
  async frames() {
    const modes = [
      { name: "no rules (plain file)", file: PLAIN, cmdText: null },
      { name: "fill", file: BIG, cmdText: "Show problems as a red fill" },
      { name: "underline", file: BIG, cmdText: "Show problems as a wavy underline" },
    ];
    for (const mode of modes) {
      const m = await start([mode.file]);
      await ensureRows(m);
      if (mode.file === BIG) {
        await waitFor(async () => /6 rules/.test(await foot(m)), 20000, 200);
        await m.getByRole("button", { name: "Check now" }).click();
        await waitFor(async () => /6 rules, [\d,]+ problems/.test(await foot(m)), 60000, 50);
        await cmd(m, mode.cmdText);
      }
      const c = await m.locator("canvas").first().boundingBox();
      const run = async (worst) => {
        await m.evaluate(() => {
          window.__ndeDrawLog = [];
        });
        await m.mouse.move(c.x + 300, c.y + 200);
        const fr = m.evaluate(async () => {
          const d = [];
          let last = performance.now();
          let n = 0;
          await new Promise((resolve) => {
            const tick = (t) => {
              d.push(t - last);
              last = t;
              if (++n < 150) requestAnimationFrame(tick);
              else resolve();
            };
            requestAnimationFrame(tick);
          });
          d.shift();
          return d;
        });
        for (let i = 0; i < 60; i++) {
          await m.mouse.wheel(0, worst ? (i % 2 ? -60 : 60) : 240);
          await sleep(25);
        }
        const frames = await fr;
        const draws = await m.evaluate(() => window.__ndeDrawLog || []);
        await m.evaluate(() => {
          window.__ndeDrawLog = undefined;
        });
        for (let i = 0; i < 60; i++) await m.mouse.wheel(0, -240);
        await sleep(500);
        const st = (a) => {
          const d = [...a].sort((x, y) => x - y);
          return d.length ? { median: d[Math.floor(d.length / 2)], p95: d[Math.floor(d.length * 0.95)], n: d.length } : { median: 0, p95: 0, n: 0 };
        };
        return { frame: st(frames), draw: st(draws) };
      };
      for (const [label, worst] of [["worst screen (top 100 rows all marked)", true], ["spread over the file", false]]) {
        const runs = [];
        for (let i = 0; i < 3; i++) runs.push(await run(worst));
        runs.sort((a, b) => a.draw.median - b.draw.median);
        const b = runs[0];
        info(`[${path.basename(path.dirname(exe))}] ${mode.name}, ${label}: frame median ${b.frame.median.toFixed(1)} ms, p95 ${b.frame.p95.toFixed(1)} ms; draw median ${b.draw.median.toFixed(2)} ms, p95 ${b.draw.p95.toFixed(2)} ms (${b.draw.n} draws)`);
      }
      await killApp();
    }
    ok(true, "measured");
  },
};

(async () => {
  if (running()) {
    console.log("another Natendo Data Edit is running: close it first (this test never stops an app it did not start)");
    process.exit(2);
  }
  backup();
  const names = which === "all" ? Object.keys(scenarios) : which.split(",").map((s) => s.trim());
  try {
    for (const n of names) {
      if (!scenarios[n]) {
        say(`unknown scenario ${n}`);
        continue;
      }
      say(`=== ${n} ===`);
      try {
        await scenarios[n]();
      } catch (e) {
        failed++;
        say(`FAIL scenario ${n} threw: ${e && e.message ? e.message : e}`);
      }
      await killApp();
    }
  } finally {
    await killApp();
    restore();
  }
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  process.exit(failed ? 1 : 0);
})();
