// Drives the REAL built app over the outside-change banner on made-up files: the app's own save never warns;
// another program's write shows the banner; Compare with disk opens only what the other program changed;
// Reload asks when there are unsaved edits and keeps the view; Ignore holds until the next change; a deleted
// file offers Save As. Then, with 64 files open, the time of a look round and of switching tabs, beside a
// base build.
//
// Usage: NODE_PATH=<folder with playwright> node scripts/real-disk.cjs <exe> <evidence folder> [base exe]
// The app's own data is backed up and put back, and only while no other app instance runs.
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
const PORT = 9403;
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

const cell = async (page, doc, row, col) => (await invoke(page, "cell_text", { doc, row, col })).text;

const msg = async (page) => ((await page.locator("footer.status .msg").textContent({ timeout: 1000 }).catch(() => "")) || "").replace(/\s+/g, " ").trim();


const baseExe = process.argv[4];
const RUN = path.join(WORK, `disk-${Date.now()}`);
fs.mkdirSync(RUN, { recursive: true });
const f = (n) => path.join(RUN, n);
const rows = (extra) => "id,name,town\r\n" + Array.from({ length: 40 }, (_, i) => `${i + 1},sample ${i + 1},${extra && i === 4 ? extra : `town ${i % 5}`}`).join("\r\n") + "\r\n";
const people = f("made up list.csv");
fs.writeFileSync(people, rows());
const shot = (m, n) => m.screenshot({ path: path.join(evidence, n) });
const clean = () => {
  for (const n of ["session.json", "recent.json"]) {
    const mine = path.join(DATA, n);
    if (fs.existsSync(mine) && hash(path.join(BACKUP, n)) !== null) fs.rmSync(mine);
  }
};
const banner = (m) => m.getByTestId("disk-banner");
const bannerText = async (m) => ((await banner(m).textContent({ timeout: 800 }).catch(() => "")) || "").replace(/\s+/g, " ").trim();
const look = (m) => cmd(m, "Check open files for outside changes");
const median = (a) => {
  const s = [...a].sort((x, y) => x - y);
  return s[Math.floor(s.length / 2)];
};

(async () => {
  for (let i = 0; i < 120 && running(); i++) await sleep(5000);
  if (running()) {
    say("another Natendo Data Edit stayed open: nothing was touched");
    process.exit(3);
  }
  backup();
  clean();
  try {
    let m = await start([people]);
    ok(await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed, 20000), "opened");
    // The app's own save.
    await m.locator("input.goto").first().fill("R2C3");
    await m.keyboard.press("Enter");
    await sleep(200);
    await m.keyboard.type("saved by the app");
    await m.keyboard.press("Enter");
    await m.mouse.click(87, 48); // the toolbar Save button
    await sleep(1500);
    const saved = await waitFor(() => fs.readFileSync(people, "utf8").includes("saved by the app"), 10000);
    info(`after Ctrl+S: ${await msg(m)} | cell ${JSON.stringify(await cell(m, 1, 1, 2))} | modified ${(await invoke(m, "doc_info", { doc: 1 })).modified} | ${await dialogText(m)}`);
    if (!saved) await shot(m, "disk-save-failed.png");
    ok(!!saved, "the app saved");
    await look(m);
    await sleep(500);
    ok((await bannerText(m)) === "", "the app's own save shows no banner");
    // An unsaved edit, then another program writes the file; no command: the 20 s timer or focus finds it.
    await m.locator("input.goto").first().fill("R3C2");
    await m.keyboard.press("Enter");
    await sleep(200);
    await m.keyboard.type("not saved yet");
    await m.keyboard.press("Enter");
    await sleep(300);
    spawnSync("powershell", ["-NoProfile", "-Command", `(Get-Content -Raw -LiteralPath '${people}').Replace('town 4','written outside') | Set-Content -NoNewline -LiteralPath '${people}'`]);
    const t0 = Date.now();
    const seen = await waitFor(async () => (await bannerText(m)).includes("changed on disk"), 30000, 500);
    ok(!!seen, `the banner came by itself after ${((Date.now() - t0) / 1000).toFixed(1)} s: ${await bannerText(m)}`);
    await shot(m, "disk-changed.png");
    await banner(m).getByRole("button", { name: "Compare with disk" }).click();
    ok(await waitFor(async () => (await tabNames(m)).length === 3, 20000), `compare opened two tabs: ${JSON.stringify(await tabNames(m))}`);
    await sleep(800);
    const diffDoc = 3;
    const cells = [];
    const diffInfo = await invoke(m, "doc_info", { doc: diffDoc });
    for (let r = 0; r < diffInfo.rows; r++) for (let c = 0; c < diffInfo.cols; c++) cells.push(await cell(m, diffDoc, r, c));
    const all = cells.join("|");
    ok(all.includes("written outside") && !all.includes("not saved yet"), `the differences hold the outside change and not the unsaved edit (${diffInfo.rows} rows)`);
    await shot(m, "disk-compare.png");
    // Back to the first tab: Ignore, then another change.
    await m.locator("[role=tab]").first().click();
    await sleep(500);
    await banner(m).getByRole("button", { name: "Ignore" }).click();
    await sleep(300);
    await cmd(m, "Check open files for outside changes");
    ok(await waitFor(async () => (await bannerText(m)).includes("changed on disk"), 5000), "asking again shows it once more");
    // Reload asks because of the unsaved edit.
    await banner(m).getByRole("button", { name: "Reload" }).click();
    ok(await waitFor(async () => (await dialogText(m)).includes("Reload and lose your changes?"), 5000), "reload asks about unsaved changes");
    await shot(m, "disk-reload-ask.png");
    await m.getByRole("button", { name: "Reload and lose them" }).click();
    ok(await waitFor(async () => (await msg(m)).startsWith("Reloaded"), 20000), `reloaded: ${await msg(m)}`);
    const docs = await tabNames(m);
    ok(docs.length === 3, "the tab stayed in place");
    ok((await bannerText(m)) === "", "no banner after the reload");
    // A column removed outside.
    fs.writeFileSync(people, "id,town\r\n1,a\r\n2,b\r\n");
    await look(m);
    await waitFor(async () => (await bannerText(m)).includes("changed on disk"), 5000);
    await banner(m).getByRole("button", { name: "Reload" }).click();
    ok(await waitFor(async () => (await msg(m)).includes("1 column from before is not there any more"), 20000), `a missing column is said: ${await msg(m)}`);
    // Deleted.
    fs.renameSync(people, f("moved away.csv"));
    await look(m);
    ok(await waitFor(async () => (await bannerText(m)).includes("deleted or moved"), 5000), `gone: ${await bannerText(m)}`);
    await shot(m, "disk-gone.png");
    await banner(m).getByRole("button", { name: "Don't warn for this file" }).click();
    ok(await waitFor(async () => !(await banner(m).isVisible()), 3000), "muted");
    await killApp();
    // 64 files open.
    const many = Array.from({ length: 64 }, (_, i) => {
      const p = f(`made up ${i + 1}.csv`);
      fs.writeFileSync(p, rows());
      return p;
    });
    const measure = async (exePath, hasLook) => {
      clean();
      const page = await start(many, {}, exePath);
      await waitFor(async () => (await tabNames(page)).length >= 64, 60000, 500);
      await sleep(4000);
      const rounds = [];
      if (hasLook)
        for (let i = 0; i < 15; i++) {
          rounds.push(await page.evaluate(async () => {
            // How long the window's thread is busy while a round runs: the longest gap between timer ticks.
            let worst = 0;
            let last = performance.now();
            const t = setInterval(() => { const n = performance.now(); worst = Math.max(worst, n - last); last = n; }, 1);
            const t0 = performance.now();
            const r = await window.__TAURI_INTERNALS__.invoke("disk_check", {});
            const took = performance.now() - t0;
            clearInterval(t);
            return { took, worst, n: r.length };
          }));
          await sleep(100);
        }
      const switches = [];
      for (let i = 0; i < 40; i++) {
        switches.push(await page.evaluate(async () => {
          const t0 = performance.now();
          window.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", code: "Tab", ctrlKey: true, bubbles: true }));
          await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
          return performance.now() - t0;
        }));
        await sleep(60);
      }
      const name = await page.locator("[role=tab][aria-selected=true]").first().textContent().catch(() => "");
      await killApp();
      return { rounds, switches, name };
    };
    const mine = await measure(exe, true);
    const took = median(mine.rounds.map((r) => r.took));
    say(`INFO a round over 64 files: ${took.toFixed(1)} ms off the window's thread (answers ${mine.rounds[0].n}); longest timer gap during a round, median ${median(mine.rounds.map((r) => r.worst)).toFixed(1)} ms`);
    ok(mine.rounds[0].n === 64, "all 64 files answered");
    ok(took < 500, "a round is quick");
    if (baseExe) {
      const results = { base: [], branch: [] };
      for (let i = 0; i < 3; i++) {
        results.base.push(median((await measure(baseExe, false)).switches));
        results.branch.push(median((await measure(exe, false)).switches));
      }
      const [b, n] = [median(results.base), median(results.branch)];
      say(`INFO switching tabs with 64 files, to the second frame: base ${b.toFixed(1)} ms, branch ${n.toFixed(1)} ms (ratio ${(n / b).toFixed(3)}); runs base ${results.base.map((x) => x.toFixed(1))}, branch ${results.branch.map((x) => x.toFixed(1))}`);
      ok(n / b <= 1.03, "switching tabs is not slower");
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
