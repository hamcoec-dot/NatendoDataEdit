// Drives the REAL built app over WebView2 remote debugging to check that no command asked about a document
// freezes the window while a change holds it: doc_info is polled every 20 ms from the page during a
// 10,000,000-row save (Save holds the write lock for the whole write) and the gaps between animation frames are measured (poll scenario); and private
// mode is toggled in tab B while tab A saves (private scenario). Run it on a main exe (it shows the freeze)
// and on this branch's exe.
//
// Usage: NODE_PATH=<folder with playwright> node scripts/real-lock.cjs <exe> <evidence folder> [poll,private]
// The app's own data is backed up and put back; files are made-up and live in <evidence>/work.
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence, which = "all"] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-lock.cjs <exe> <evidence folder> [scenarios]");
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




/** Made-up files: ten million rows (a random-looking text key to sort on) and a small one for the second tab. */
const BIG0 = path.join(WORK, "ten-m.csv");
const BIG = path.join(WORK, "ten-m-save.csv");
const SMALL = path.join(WORK, "small.csv");
const ROWS = 10_000_000;
if (!fs.existsSync(SMALL)) fs.writeFileSync(SMALL, "id,name\r\n" + Array.from({ length: 50 }, (_, i) => `${i + 1},Person ${i + 1}`).join("\r\n") + "\r\n");
const makeBig = () => {
  if (fs.existsSync(BIG0)) return;
  const fd = fs.openSync(BIG0, "w");
  fs.writeSync(fd, "id,key,name\r\n");
  let x = 123456789;
  const CH = 100_000;
  for (let base = 0; base < ROWS; base += CH) {
    const lines = [];
    for (let i = 0; i < CH; i++) {
      x = (Math.imul(x, 1103515245) + 12345) >>> 0;
      lines.push(`${base + i + 1},k${x.toString(36)},Person ${x % 997}`);
    }
    fs.writeSync(fd, lines.join("\r\n") + "\r\n");
  }
  fs.closeSync(fd);
};
makeBig();
// The saved-over copy is made afresh for each run (a save rewrites it).
const freshBig = async () => {
  for (let i = 0; i < 40; i++) {
    try {
      fs.copyFileSync(BIG0, BIG);
      return;
    } catch {
      await sleep(500);
    }
  }
  fs.copyFileSync(BIG0, BIG);
};
/** The long change: an edit, then Save over the file. A save holds the document's write lock for the whole write
 *  of ten million rows (a sort or replace holds it only to apply), so it is what shows the freeze. */
const heavy = (m) =>
  m.evaluate(async () => {
    const call = window.__TAURI_INTERNALS__.invoke;
    try {
      await call("set_cells", { doc: 1, cells: [[1, 0, "X" + Date.now()]] });
      return await call("save", { doc: 1, saveAs: false, overwriteChanged: true, allowLossy: true, options: null });
    } catch (e) {
      return { error: typeof e === "string" ? e : JSON.stringify(e) };
    }
  });

/** Page-side monitor: the gaps between animation frames, and how long each doc_info call took (polled every 20 ms). */
const monitorOn = (m, pollDoc) =>
  m.evaluate((doc) => {
    window.__lock = { gaps: [], maxGap: 0, lat: [], polls: 0, stop: false };
    let last = performance.now();
    const tick = (t) => {
      const g = t - last;
      last = t;
      window.__lock.gaps.push(g);
      if (g > window.__lock.maxGap) window.__lock.maxGap = g;
      if (!window.__lock.stop) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
    if (doc) {
      const loop = async () => {
        while (!window.__lock.stop) {
          const t0 = performance.now();
          try {
            await window.__TAURI_INTERNALS__.invoke("doc_info", { doc });
          } catch {}
          window.__lock.lat.push(performance.now() - t0);
          window.__lock.polls++;
          await new Promise((r) => setTimeout(r, 20));
        }
      };
      void loop();
    }
  }, pollDoc);
const monitorOff = (m) =>
  m.evaluate(() => {
    window.__lock.stop = true;
    const L = window.__lock;
    const lat = [...L.lat].sort((a, b) => a - b);
    return { maxGap: L.maxGap, frames: L.gaps.length, polls: L.polls, maxLat: lat.length ? lat[lat.length - 1] : 0, p95Lat: lat.length ? lat[Math.floor(lat.length * 0.95)] : 0 };
  });

const scenarios = {
  // doc_info polled every 20 ms from the page during a 10,000,000-row save: the window keeps redrawing.
  async poll() {
    await freshBig();
    const m = await start([BIG]);
    await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed === true, 600000, 500);
    const di = await invoke(m, "doc_info", { doc: 1 });
    info(`document 1 is ready: ${di.rows.toLocaleString()} rows`);
    await dismissAsk(m);
    await sleep(1000);
    for (const polling of [false, true]) {
      await monitorOn(m, polling ? 1 : 0);
      await sleep(300);
      const t0 = Date.now();
      const sorted = await heavy(m).catch((e) => ({ error: String(e && e.message ? e.message : e) }));
      const ms = Date.now() - t0;
      if (sorted && sorted.error) info(`save failed: ${sorted.error}`);
      const r = await monitorOff(m);
      info(`${polling ? "polling doc_info every 20 ms" : "no polling"}: save took ${ms} ms; longest gap between frames ${r.maxGap.toFixed(0)} ms over ${r.frames} frames; ${r.polls} polls, longest ${r.maxLat.toFixed(0)} ms, p95 ${r.p95Lat.toFixed(0)} ms`);
      if (polling) {
        ok(r.maxLat < 100, `every doc_info answered within 100 ms during the save (longest ${r.maxLat.toFixed(0)} ms)`);
        ok(r.maxGap < 100, `the page kept redrawing during the save while doc_info was polled (longest gap ${r.maxGap.toFixed(0)} ms, limit 100)`);
      }
      await sleep(500);
    }
  },

  // Toggling private mode in tab B while tab A saves: the window does not freeze.
  async private() {
    await freshBig();
    const m = await start([BIG, SMALL]);
    await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed === true, 600000, 500);
    await waitFor(async () => (await invoke(m, "doc_info", { doc: 2 })).indexed === true, 60000, 200);
    await dismissAsk(m);
    await sleep(1000);
    await monitorOn(m, 0);
    const t0 = Date.now();
    const sort = heavy(m).then((r) => r && r.error && info(`save failed: ${r.error}`));
    let finished = false;
    sort.then(() => (finished = true));
    await sleep(300);
    const times = [];
    let during = false;
    for (const on of [true, false, true, false]) {
      const t1 = Date.now();
      await invoke(m, "set_private", { doc: 2, folder: false, on }).catch(() => {});
      times.push(Date.now() - t1);
      // The save must still be running when the first call has come back (else it proved nothing).
      if (times.length === 1) during = !finished;
      await sleep(100);
    }
    await sort;
    const total = Date.now() - t0;
    const r = await monitorOff(m);
    info(`save ${total} ms; set_private calls took ${times.join(", ")} ms (save still running while they ran: ${during}); longest gap between frames ${r.maxGap.toFixed(0)} ms`);
    ok(during, "the save was still running while private mode was toggled in the other tab");
    ok(Math.max(...times) < 150, `toggling private mode took at most ${Math.max(...times)} ms during the save`);
    ok(r.maxGap < 100, `the window kept redrawing (longest gap ${r.maxGap.toFixed(0)} ms, limit 100)`);
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
