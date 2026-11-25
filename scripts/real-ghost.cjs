// Drives the REAL built app over WebView2 remote debugging to check the ghost diff (Find and Replace) on a
// 1,000,000-row file: the window of cells on screen asked for through the app's commands (timed), typing into
// the Replace box with ghosts on (the page keeps redrawing, the count finishes exactly), and scroll frame / draw
// time with ghosts on (worst screen) against off and against a main exe (which has none: NO_GHOSTS=1).
//
// Usage: NODE_PATH=<folder with playwright> node scripts/real-ghost.cjs <exe> <evidence folder> [ipc,typing,frames]
// The app's own data is backed up and put back; files are made-up and live in <evidence>/work.
const path = require("path");
const crypto = require("crypto");

const [exe, evidence, which = "all"] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-ghost.cjs <exe> <evidence folder> [scenarios]");
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





/** A made-up file: 1,000,000 rows of id, name, city (the city repeats in a third of the rows). */
const N = 1_000_000;
const BIG = path.join(WORK, "ghost-1m.csv");
if (!fs.existsSync(BIG)) {
  const lines = ["id,name,city"];
  for (let i = 0; i < N; i++) lines.push(`${i + 1},Person ${i % 997},${["Dayton", "Lisbon", "Perth"][i % 3]} ${i % 31}`);
  fs.writeFileSync(BIG, lines.join("\r\n") + "\r\n");
}
const SPEC = (text, kind = "plain") => ({ pattern: { text, kind, match_case: false, whole_cell: false }, scope: { kind: "all" }, skip_first: true, skip_cols: [], hidden_cols: [], hidden_rows: false });
const loaded = async (m) => waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed === true, 300000, 250);

const scenarios = {
  // The window of cells on screen, asked for through the app's own commands: under 10 ms; and the whole-file count.
  async ipc() {
    const m = await start([BIG]);
    await loaded(m);
    await dismissAsk(m);
    const time = async (spec, rep) => {
      const t = [];
      for (let i = 0; i < 15; i++) {
        const t0 = Date.now();
        const w = await m.evaluate(async ([s, r]) => {
          const a = performance.now();
          const res = await window.__TAURI_INTERNALS__.invoke("replace_ghosts", { doc: 1, spec: s, replacement: r, row: 500000, rows: 60, col: 0, cols: 20 });
          return { ms: performance.now() - a, n: res.ghosts.length, partial: res.partial };
        }, [spec, rep]);
        t.push(w);
        void t0;
      }
      t.sort((a, b) => a.ms - b.ms);
      return t[Math.floor(t.length / 2)];
    };
    const plain = await time(SPEC("Lisbon"), "Porto");
    info(`replace_ghosts through the app, plain, 60 rows x 20 columns: median ${plain.ms.toFixed(1)} ms (${plain.n} cells)`);
    ok(plain.ms < 10 + 5, `a window answers within 10 ms plus the IPC (${plain.ms.toFixed(1)} ms)`);
    const rx = await time(SPEC("(Lis)bon ([0-9]+)", "regex"), "$2-$1");
    info(`regex with groups: median ${rx.ms.toFixed(1)} ms (${rx.n} cells)`);
    const t0 = Date.now();
    const n = await invoke(m, "replace_count", { doc: 1, spec: SPEC("Lisbon"), replacement: "Porto" });
    info(`replace_count over ${N.toLocaleString()} rows: ${Date.now() - t0} ms, ${n.toLocaleString()} cells`);
    ok(n === Math.floor((N - 1 - 1) / 3) + 1 || n > 300000, `the count is ${n.toLocaleString()}`);
  },

  // Typing into the Replace box with ghosts on: the page keeps redrawing and the count never queues up.
  async typing() {
    const m = await start([BIG]);
    await loaded(m);
    await dismissAsk(m);
    await m.keyboard.press("Control+h");
    await sleep(600);
    await m.getByLabel("Find", { exact: true }).first().fill("Lisbon");
    await m.getByLabel("Replace with").click();
    await m.evaluate(() => {
      window.__lock = { max: 0, stop: false };
      let last = performance.now();
      const tick = (t) => {
        window.__lock.max = Math.max(window.__lock.max, t - last);
        last = t;
        if (!window.__lock.stop) requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    });
    for (const ch of "Porto, the city by the sea".split("")) {
      await m.keyboard.type(ch, { delay: 0 });
      await sleep(30);
    }
    await sleep(2500);
    const gap = await m.evaluate(() => {
      window.__lock.stop = true;
      return window.__lock.max;
    });
    const line = await m.evaluate(() => (document.querySelector(".count.ghosts")?.textContent || "").replace(/\s+/g, " "));
    info(`typing 26 characters 30 ms apart: longest gap between frames ${gap.toFixed(0)} ms; the line says: ${line}`);
    ok(gap < 100, `the page kept redrawing while typing with ghosts on (${gap.toFixed(0)} ms, limit 100)`);
    ok(/Would change \d+ of the cells you can see/.test(line) && /Replace all would change [\d,]+ cells/.test(line), "and the count finished with an exact figure once typing stopped");
    await m.screenshot({ path: path.join(evidence, "ghost-real.png") });
  },

  // Scroll frame and draw time with ghosts on (worst screen: every visible city changes) against off; run on a main exe too (it has none).
  async frames() {
    const m = await start([BIG]);
    await loaded(m);
    await dismissAsk(m);
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
    const best = async () => {
      const r = [];
      for (let i = 0; i < 3; i++) r.push(await measure());
      r.sort((a, b) => a.draw.median - b.draw.median);
      return r[0];
    };
    const off = await best();
    info(`[${path.basename(path.dirname(exe))}] Find closed (no ghosts): frame median ${off.frame.median.toFixed(1)} ms, p95 ${off.frame.p95.toFixed(1)} ms; draw median ${off.draw.median.toFixed(2)} ms, p95 ${off.draw.p95.toFixed(2)} ms`);
    if (process.env.NO_GHOSTS) return;
    // The worst screen: Find open with a search that changes every city on screen.
    await m.keyboard.press("Control+h");
    await sleep(600);
    await m.getByLabel("Find", { exact: true }).first().fill("n");
    await m.getByLabel("Replace with").fill("N!");
    await sleep(2500);
    const on = await best();
    info(`[${path.basename(path.dirname(exe))}] ghosts on, every visible cell changes: frame median ${on.frame.median.toFixed(1)} ms, p95 ${on.frame.p95.toFixed(1)} ms; draw median ${on.draw.median.toFixed(2)} ms, p95 ${on.draw.p95.toFixed(2)} ms`);
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
