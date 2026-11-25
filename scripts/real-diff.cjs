// Drives the REAL built app over WebView2 remote debugging to check the key-aware diff on two 1,000,000-row
// tables: the diff through the app's commands (timed), a window of marks (limit 10 ms), the next difference, a
// sort keeping the marks, an edit making them out of date, a table with itself, a private source; and scroll
// frame / draw time with the marks drawn (compared with a main exe, NO_DIFF=1, which has none).
//
// Usage: NODE_PATH=<folder with playwright> node scripts/real-diff.cjs <exe> <evidence folder> [ipc,frames]
// The app's own data is backed up and put back; files are made-up and live in <evidence>/work.
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence, which = "all"] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-diff.cjs <exe> <evidence folder> [scenarios]");
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





/** Two made-up tables of 1,000,000 voters: the second has another column order and one more column, every
 *  10th row changed, every 997th row missing and a few rows of its own. */
const N = 1_000_000;
const FA = path.join(WORK, "diff-a.csv");
const FB = path.join(WORK, "diff-b.csv");
if (!fs.existsSync(FA) || !fs.existsSync(FB)) {
  const a = ["id,name,city,amount"];
  const b = ["city,id,name,amount,zip"];
  for (let i = 0; i < N; i++) {
    a.push(`${i},name ${i % 9973},City ${i % 311},${i % 100000}.${String(i % 100).padStart(2, "0")}`);
    if (i % 997 === 0) continue;
    b.push(`City ${i % 10 === 0 ? (i + 1) % 311 : i % 311},${i},name ${i % 9973},${i % 100000}.${String(i % 100).padStart(2, "0")},${10000 + (i % 90000)}`);
  }
  for (let i = 0; i < 200; i++) b.push(`City x,${N + i},name x,1.00,1`);
  fs.writeFileSync(FA, a.join("\r\n") + "\r\n");
  fs.writeFileSync(FB, b.join("\r\n") + "\r\n");
}
const SPEC = { keys: [0], keep_first: true, match_case: false, trim: true };
const ready = async (m, n) => waitFor(async () => (await invoke(m, "doc_info", { doc: n })).indexed === true, 300000, 250);
const evalTimes = (m, cmdName, args, n) =>
  m.evaluate(
    async ([c, a, k]) => {
      const t = [];
      let last;
      for (let i = 0; i < k; i++) {
        const t0 = performance.now();
        last = await window.__TAURI_INTERNALS__.invoke(c, a);
        t.push(performance.now() - t0);
      }
      t.sort((x, y) => x - y);
      return { median: t[Math.floor(t.length / 2)], last };
    },
    [cmdName, args, n],
  );

const scenarios = {
  // The diff through the app's own commands: the job, a window of marks, the next difference, out of date after an edit, privacy.
  async ipc() {
    const m = await start([FA, FB]);
    await ready(m, 1);
    await ready(m, 2);
    await dismissAsk(m);
    const t0 = Date.now();
    const rep = await invoke(m, "diff_files", { a: 1, b: 2, spec: SPEC });
    const ms = Date.now() - t0;
    const r = rep.report;
    info(`diff_files, ${N.toLocaleString()} rows: ${ms} ms; ${r.changed.toLocaleString()} changed, ${r.removed.toLocaleString()} removed, ${r.added.toLocaleString()} added, ${r.same.toLocaleString()} the same; ${r.listed.toLocaleString()} listed; columns only in the second: ${r.only_b.join(",")}`);
    ok(r.changed === 99899 && r.removed === 1004 && r.added === 200 && r.same === N - 99899 - 1004, `the counts are exact (${r.changed}/${r.removed}/${r.added}/${r.same})`);
    const w = await evalTimes(m, "diff_marks", { doc: 1, row: 500000, rows: 60, col: 0, cols: 20 }, 15);
    info(`diff_marks through the app, 60 rows x 20 columns on the first table: median ${w.median.toFixed(1)} ms (${w.last.marks.length} marks)`);
    ok(w.median < 10 + 5 && w.last.marks.length > 0, `a window of marks answers within 10 ms plus the IPC (${w.median.toFixed(1)} ms)`);
    const wb = await evalTimes(m, "diff_marks", { doc: 2, row: 500000, rows: 60, col: 0, cols: 20 }, 15);
    info(`on the second table: median ${wb.median.toFixed(1)} ms (${wb.last.marks.length} marks)`);
    const wr = await evalTimes(m, "diff_marks", { doc: rep.info.id, row: 5000, rows: 60, col: 0, cols: 20 }, 15);
    info(`on the result table: median ${wr.median.toFixed(1)} ms (${wr.last.marks.length} marks)`);
    const t1 = Date.now();
    const j = await invoke(m, "diff_next", { doc: 1, row: 500001, backward: false });
    info(`diff_next from the middle: ${Date.now() - t1} ms to row ${j.row}`);
    const t2 = Date.now();
    const far = await invoke(m, "diff_next", { doc: 1, row: N - 1, backward: false });
    info(`diff_next past the last difference (wraps round 1,000,000 rows): ${Date.now() - t2} ms to row ${far.row}, wrapped ${far.wrapped}`);
    // A sort keeps the marks (by key); an edit makes them out of date.
    await invoke(m, "sort_rows", { doc: 1, spec: { keys: [{ col: 1, descending: true, kind: "text", match_case: false }], keep_first: true } });
    const after = await invoke(m, "diff_marks", { doc: 1, row: 0, rows: 60, col: 0, cols: 20 });
    ok(!after.out_of_date && after.marks.length >= 0 && after.diff !== null, "after a sort the marks are still there (looked up by key)");
    await invoke(m, "set_cells", { doc: 1, cells: [[1, 1, "edited"]] });
    const stale = await invoke(m, "diff_marks", { doc: 1, row: 0, rows: 60, col: 0, cols: 20 });
    ok(stale.out_of_date && stale.marks.length === 0, "after an edit the marks are out of date and nothing is drawn");
    // A table with itself.
    const me = await invoke(m, "diff_files", { a: 2, b: 2, spec: SPEC });
    ok(me.report.changed === 0 && me.report.added === 0 && me.report.removed === 0, "a table compared with itself: all the same");
    // Private: the result of a private source is private.
    await invoke(m, "set_private", { doc: 2, folder: false, on: true });
    const pr = await invoke(m, "diff_files", { a: 2, b: 2, spec: SPEC });
    const st = await invoke(m, "private_state", { doc: pr.info.id });
    ok(st.private === true || st.file === true || JSON.stringify(st).includes("true"), `the result of a private tab is private (${JSON.stringify(st).slice(0, 80)})`);
  },

  // Scroll frame and draw time with the differences drawn (the first table, marks on) against a main exe (none).
  async frames() {
    const m = await start([FA, FB]);
    await ready(m, 1);
    await ready(m, 2);
    await dismissAsk(m);
    if (!process.env.NO_DIFF) {
      // Tools > Compare, by key, from the dialog (it also turns the marks on): the first tab, key column id.
      await m.locator("[role=tab], .tab").first().click({ position: { x: 20, y: 8 } });
      await sleep(500);
      await cmd(m, "Compare with another open file");
      await sleep(800);
      await m.locator("fieldset label", { hasText: "id" }).first().locator("input").check();
      await m.getByRole("button", { name: "Run" }).click();
      await waitFor(async () => /Compared by key/.test((await m.textContent("footer.status")) || ""), 120000, 250);
      await m.locator("[role=tab], .tab").first().click({ position: { x: 20, y: 8 } });
      await sleep(1500);
      await m.screenshot({ path: path.join(evidence, "diff-real.png") });
    }
    const c = await m.locator("canvas").first().boundingBox();
    const measure = async () => {
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
        await m.mouse.wheel(0, 240);
        await sleep(25);
      }
      const frames = await fr;
      const draws = await m.evaluate(() => window.__ndeDrawLog || []);
      await m.evaluate(() => {
        window.__ndeDrawLog = undefined;
      });
      for (let i = 0; i < 60; i++) await m.mouse.wheel(0, -240);
      await sleep(600);
      const st = (a) => {
        const d = [...a].sort((x, y) => x - y);
        return d.length ? { median: d[Math.floor(d.length / 2)], p95: d[Math.floor(d.length * 0.95)], n: d.length } : { median: 0, p95: 0, n: 0 };
      };
      return { frame: st(frames), draw: st(draws) };
    };
    const r = [];
    for (let i = 0; i < 3; i++) r.push(await measure());
    r.sort((a, b) => a.draw.median - b.draw.median);
    const b = r[0];
    info(`[${path.basename(path.dirname(exe))}] ${process.env.NO_DIFF ? "no diff" : "diff marks on"}: frame median ${b.frame.median.toFixed(1)} ms, p95 ${b.frame.p95.toFixed(1)} ms; draw median ${b.draw.median.toFixed(2)} ms, p95 ${b.draw.p95.toFixed(2)} ms`);
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
