// Drives the REAL built app over the totals row: switched on from command search over a made-up voter file
// (IDs and ZIP codes counted, votes added), a filter changes the totals, a click on a total picks another one,
// copying gives the numbers; then scroll and re-filter frame times on a 1,000,000 x 40 file with the row off
// and on.
//
// Usage: NODE_PATH=<folder with playwright> node scripts/real-totals.cjs <exe> <evidence folder> [big.csv]
// The app's own data is backed up and put back.
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
const PORT = 9402;
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
const start = async (files, extraEnv = {}, exePath = exe, keepQuestion = false) => {
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
  if (!keepQuestion) await dismiss(m);
  return m;
};
const startKeeping = (files) => start(files, {}, exe, true);
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

const cell = async (page, doc, row, col) => (await invoke(page, "cell_text", { doc, row, col })).text;

const msg = async (page) => ((await page.locator("footer.status .msg").textContent({ timeout: 1000 }).catch(() => "")) || "").replace(/\s+/g, " ").trim();


const bigFile = process.argv[4];
const people = path.join(WORK, "made up voters.csv");
// 60 made-up rows: every third is "North".
fs.writeFileSync(people, "Voter ID,ZIP,Area,Votes\r\n" + Array.from({ length: 60 }, (_, i) => `${100200 + i},${37400 + (i % 4)},${i % 3 === 0 ? "North" : "South"},${i + 1}`).join("\r\n") + "\r\n");
const shot = (m, n) => m.screenshot({ path: path.join(evidence, n) });
const clean = () => {
  for (const f of ["session.json", "recent.json"]) {
    const mine = path.join(DATA, f);
    if (fs.existsSync(mine) && hash(path.join(BACKUP, f)) !== null) fs.rmSync(mine);
  }
};
const clip = (m) => m.evaluate(() => navigator.clipboard.readText());
/** Copy all totals until the clipboard holds `want`. */
const copied = async (m, want, label) => {
  let got = "";
  const hit = await waitFor(async () => {
    await cmd(m, "Copy all totals as a row");
    got = await clip(m);
    return got === want;
  }, 8000);
  ok(!!hit, `${label}: ${JSON.stringify(got)} (want ${JSON.stringify(want)})`);
};
const median = (a) => {
  const s = [...a].sort((x, y) => x - y);
  return s[Math.floor(s.length / 2)];
};
/** Frame intervals and draw times while `act` runs. */
const frames = async (m, act) => {
  await m.evaluate(() => {
    window.__ndeDrawLog = [];
    window.__frames = [];
    window.__stop = false;
    let last = performance.now();
    const tick = (t) => {
      window.__frames.push(t - last);
      last = t;
      if (!window.__stop) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
  await act();
  return m.evaluate(() => {
    window.__stop = true;
    const f = window.__frames.slice(1);
    const d = window.__ndeDrawLog;
    window.__ndeDrawLog = undefined;
    return { f, d };
  });
};
const stats = (r) => {
  const f = [...r.f].sort((a, b) => a - b);
  return { mean: r.f.reduce((a, b) => a + b, 0) / r.f.length, p95: f[Math.floor(f.length * 0.95)], draw: median(r.d), n: r.f.length };
};

(async () => {
  // The app's own data is touched only while no other run (or person) has the app open.
  for (let i = 0; i < 120 && running(); i++) await sleep(5000);
  if (running()) {
    say("another Natendo Data Edit stayed open: nothing was touched");
    process.exit(3);
  }
  backup();
  clean();
  try {
    let m = await start([people]);
    await m.context().grantPermissions(["clipboard-read", "clipboard-write"]).catch(() => {});
    await cmd(m, "Totals row (under the shown rows)");
    await sleep(900);
    await shot(m, "totals-on.png");
    // Voter ID, ZIP, Area counted; Votes added (1 + ... + 60).
    await copied(m, "60\t60\t60\t1830", "defaults");
    // Right-click an Area cell reading North (row 2) and filter by it.
    await m.locator("input.goto").first().fill("R2C3");
    await m.keyboard.press("Enter");
    await sleep(300);
    await cmd(m, "Filter by selected value");
    await sleep(900);
    // North: rows 1, 4, 7, ... 58 (20 rows), votes 590.
    await copied(m, "20\t20\t20\t590", "filtered");
    await shot(m, "totals-filtered.png");
    // A click on the total under Votes (the fourth column) opens its menu; the heading says where it is.
    const box = await m.locator("canvas").first().boundingBox();
    const click = () => m.mouse.click(505, box.y + box.height - 12);
    await click();
    const item = m.getByRole("menuitem", { name: /Totals row: Maximum/ }).or(m.locator(".context").getByText("Totals row: Maximum"));
    const opened = await waitFor(() => item.first().isVisible(), 4000);
    ok(!!opened, "a click on the Votes total opens its menu");
    await shot(m, "totals-menu.png");
    await m.locator(".context").getByText("Copy this column's total").click();
    await sleep(500);
    const one = await clip(m);
    ok(one === "590", `the menu acts on the column clicked: copied ${JSON.stringify(one)}`);
    await click();
    await sleep(300);
    await m.locator(".context").getByText("Totals row: Maximum").click();
    await sleep(500);
    // A right-click opens the same menu.
    await m.mouse.click(505, box.y + box.height - 12, { button: "right" });
    ok(!!(await waitFor(() => m.locator(".context").getByText("Copy all totals as a row").isVisible(), 3000)), "a right-click opens the menu too");
    await m.keyboard.press("Escape");
    await copied(m, "20\t20\t20\t58", "maximum chosen");
    await sleep(500);
    await shot(m, "totals-maximum.png");
    // An edit changes the total.
    await cmd(m, "Undo");
    await sleep(600);
    await cmd(m, "Totals row (under the shown rows)");
    await sleep(300);
    await shot(m, "totals-off.png");
    await killApp();
    // The totals row and the title-row question together, whole and split: the last row stays reachable.
    clean();
    const names = path.join(WORK, "made up names.csv");
    fs.writeFileSync(names, Array.from({ length: 90 }, (_, i) => `sample ${i + 1},town ${i % 7},note ${i}`).join("\r\n") + "\r\n");
    m = await startKeeping([names]);
    await cmd(m, "Totals row (under the shown rows)");
    const ask = m.locator(".header-ask");
    const asked = await waitFor(() => ask.isVisible(), 5000);
    info(`title-row question shown: ${!!asked}`);
    await m.locator("canvas").first().click({ position: { x: 140, y: 40 } });
    await m.keyboard.press("Control+End");
    await sleep(900);
    const goto = await m.locator("input.goto").first().inputValue();
    ok(goto.startsWith("R90"), `Ctrl+End still lands on the last data row: ${goto}`);
    if (asked && (await ask.isVisible())) {
      const q = await ask.boundingBox();
      const c = await m.locator("canvas").first().boundingBox();
      ok(q.y + q.height <= c.y + c.height - 40, "the question floats above the totals row");
    }
    await shot(m, "totals-with-question.png");
    await cmd(m, "Split top and bottom");
    await sleep(900);
    await shot(m, "totals-split.png");
    await killApp();
    if (bigFile) {
      clean();
      m = await start([bigFile]);
      ok(await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed, 120000, 300), "the 1M-row file is open");
      await sleep(1500);
      const canvas = await m.locator("canvas").first().boundingBox();
      const scroll = async () => {
        await m.mouse.move(canvas.x + 400, canvas.y + 200);
        for (let i = 0; i < 80; i++) {
          await m.mouse.wheel(i % 20 < 10 ? 0 : 200, 240);
          await sleep(25);
        }
      };
      // A filter on and off again and again, about as often as typing.
      await m.locator("input.goto").first().fill("R2C2");
      await m.keyboard.press("Enter");
      await sleep(300);
      await cmd(m, "Filter by selected value");
      await sleep(2500);
      const refilter = async () => {
        for (let i = 0; i < 12; i++) {
          await m.keyboard.press("Control+z");
          await sleep(180);
          await m.keyboard.press("Control+y");
          await sleep(180);
        }
      };
      const res = { off: { scroll: [], filter: [] }, on: { scroll: [], filter: [] } };
      for (let round = 0; round < 3; round++) {
        for (const state of round % 2 ? ["on", "off"] : ["off", "on"]) {
          const is = await m.evaluate(() => !!window.__totalsOn);
          if ((state === "on") !== is) {
            await cmd(m, "Totals row (under the shown rows)");
            await m.evaluate((v) => (window.__totalsOn = v), state === "on");
          }
          await sleep(2000);
          res[state].scroll.push(stats(await frames(m, scroll)));
          res[state].filter.push(stats(await frames(m, refilter)));
        }
      }
      await shot(m, "totals-big.png");
      for (const kind of ["scroll", "filter"]) {
        const of = (s, k) => median(res[s][kind].map((r) => r[k]));
        const ratio = of("on", "mean") / of("off", "mean");
        info(`${kind}: mean frame off ${of("off", "mean").toFixed(2)} ms, on ${of("on", "mean").toFixed(2)} ms (ratio ${ratio.toFixed(3)}); p95 off ${of("off", "p95").toFixed(1)}, on ${of("on", "p95").toFixed(1)}; draw off ${of("off", "draw").toFixed(2)}, on ${of("on", "draw").toFixed(2)} ms`);
        ok(ratio <= 1.02, `${kind}: frame time with the row on within 2% of off`);
      }
    }
  } catch (e) {
    ok(false, String(e && e.stack ? e.stack : e));
  } finally {
    await killApp();
    restore();
    say(failed ? `${failed} FAILED` : "all passed");
    process.exit(failed ? 1 : 0);
  }
})();
