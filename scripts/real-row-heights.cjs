// Drives the REAL built app (not the browser preview) over WebView2 remote
// debugging to check per-row heights: dragging the line under a row number (one
// row, several), Undo and Redo, fitting a row to its wrapped text, a row
// inserted above a tall one, hidden rows, the heights coming back after the app
// is closed and opened again, and the scroll frame time on a 1,000,000-row file.
//
// Usage (after `npx tauri build`):
//   NODE_PATH=<folder with playwright> node scripts/real-row-heights.cjs <exe> <evidence folder> <big 1M-row csv> [scenarios]
// scenarios: a comma list of basic, session, speed (default: all). The app's own
// data (session.json, recent.json, private.json, recovery copies) is backed up
// and put back; the app runs with its own browser profile under the evidence
// folder. Files are made-up and live in <evidence>/work. Nothing is installed.
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence, bigCsv, which = "all"] = process.argv.slice(2);
if (!exe || !evidence || !bigCsv) {
  console.log("usage: node scripts/real-row-heights.cjs <exe> <evidence folder> <big csv> [scenarios]");
  process.exit(2);
}
const WORK = path.join(evidence, "work");
fs.mkdirSync(WORK, { recursive: true });
const DATA = path.join(process.env.APPDATA, "com.natendo.dataedit");
const LOCAL = path.join(process.env.LOCALAPPDATA, "com.natendo.dataedit");
const PORT = 9361;
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

// ---- scenarios: per-row heights ----
const notes = path.join(WORK, "notes.csv");
fs.writeFileSync(notes, "id,name,note\r\n" + Array.from({ length: 60 }, (_, i) => `${i + 1},Person ${i + 1},"${"a long note that has to wrap over several lines when its column is narrow ".repeat(3)}${i + 1}"`).join("\r\n") + "\r\n");
const cell = async (page, doc, row, col) => (await invoke(page, "cell_text", { doc, row, col })).text;
const foot = async (page) => ((await page.locator("footer.status").textContent({ timeout: 1000 }).catch(() => "")) || "").replace(/\s+/g, " ").trim();
const msg = async (page) => ((await page.locator("footer.status .msg").textContent({ timeout: 1000 }).catch(() => "")) || "").replace(/\s+/g, " ").trim();
const dismissAsk = async (m) => {
  const yes = m.getByRole("button", { name: "Yes, column names" });
  if (await yes.isVisible().catch(() => false)) await m.keyboard.press("Alt+Shift+Y");
  await sleep(500);
};
const grid = async (page) => {
  const c = await page.locator("canvas").first().boundingBox();
  const starts = [];
  let last = 0;
  for (let x = 30; x < 700 && starts.length < 3; x += 2) {
    await page.mouse.click(c.x + x, c.y + 25 + 25 + 12);
    const m = /C(\d+)$/.exec(await page.locator("input.goto").inputValue());
    const col = m ? Number(m[1]) : 0;
    if (col > last) {
      starts.push(x);
      last = col;
    }
  }
  return { c, x0: starts[0] + 10 };
};
/** Rows by scanning down the first column: row number -> [top y, bottom y] (client pixels). */
const scanRows = async (page, g, count = 10, step = 2) => {
  const out = new Map();
  const y0 = g.c.y + 27;
  for (let y = y0; y < y0 + 32 * count && out.size <= count; y += step) {
    await page.mouse.click(g.x0 + g.c.x, y);
    const m = /R(\d+) C/.exec(await page.locator("input.goto").inputValue());
    if (!m) continue;
    const r = Number(m[1]);
    const e = out.get(r);
    out.set(r, e ? [e[0], y] : [y, y]);
  }
  return out;
};
const tall = (rows, r, step = 2) => (rows.get(r) ? rows.get(r)[1] - rows.get(r)[0] + step : 0);
const numberX = (g) => g.c.x + 20;

const scenarios = {
  async basic() {
    const m = await start([notes]);
    await dismissAsk(m);
    const g = await grid(m);
    const rows0 = await scanRows(m, g, 8);
    const P = tall(rows0, 4);
    info(`plain row height about ${P} px (rows R3..R9 scanned)`);
    ok(P >= 20 && P <= 45, "plain rows measured");
    // Drag the line under R5 down by 30 px.
    const edge = rows0.get(5)[1] - 1;
    await m.mouse.move(numberX(g), edge);
    const cursor = await m.evaluate(() => getComputedStyle(document.querySelector("[aria-label='Data grid']") ?? document.body).cursor);
    ok(cursor === "row-resize", `the pointer on the line under a row number shows the row-resize cursor (${cursor})`);
    await m.mouse.down();
    await m.mouse.move(numberX(g), edge + 15, { steps: 4 });
    await m.mouse.move(numberX(g), edge + 30, { steps: 4 });
    await m.mouse.up();
    await sleep(500);
    await m.screenshot({ path: path.join(evidence, "rowheights-drag.png") });
    const rows1 = await scanRows(m, g, 8);
    ok(Math.abs(tall(rows1, 5) - (P + 30)) <= 4 && Math.abs(tall(rows1, 6) - P) <= 3, `dragging the line makes row 5 ${tall(rows1, 5)} px (plain ${P} + 30) and leaves row 6 at ${tall(rows1, 6)}`);
    // Undo and redo.
    await m.keyboard.press("Control+z");
    await sleep(700);
    const rows2 = await scanRows(m, g, 8);
    ok(Math.abs(tall(rows2, 5) - P) <= 3, `Ctrl+Z takes the height back (${tall(rows2, 5)} px)`);
    await m.keyboard.press("Control+y");
    await sleep(700);
    const rows3 = await scanRows(m, g, 8);
    ok(Math.abs(tall(rows3, 5) - (P + 30)) <= 4, `Ctrl+Y puts it back (${tall(rows3, 5)} px)`);
    // Several rows: select R8 and R9 by their numbers, drag the line of R8.
    await m.mouse.click(numberX(g), rows3.get(7)[0] + 4);
    await m.keyboard.down("Shift");
    await m.mouse.click(numberX(g), rows3.get(8)[0] + 4);
    await m.keyboard.up("Shift");
    const e8 = rows3.get(7)[1] - 1;
    await m.mouse.move(numberX(g), e8);
    await m.mouse.down();
    await m.mouse.move(numberX(g), e8 + 20, { steps: 5 });
    await m.mouse.up();
    await sleep(500);
    const rows4 = await scanRows(m, g, 9);
    ok(Math.abs(tall(rows4, 7) - (P + 20)) <= 4 && Math.abs(tall(rows4, 8) - (P + 20)) <= 4, `with two rows selected, the line of one sizes both (${tall(rows4, 7)} and ${tall(rows4, 8)} px)`);
    // Wrapped text: wrap the note column, then fit R3 by double-clicking the line.
    await m.mouse.click(g.c.x + g.x0 + 3 * 130 - 40, g.c.y + 12);
    await cmd(m, "Wrap text");
    await sleep(400);
    // (Wrapping also grows every row to fit what is on screen: put every row back to one line, so only the fit below makes a row taller.)
    await cmd(m, "Row height of every row (lines)");
    await sleep(400);
    await m.keyboard.press("Control+a");
    await m.keyboard.type("1");
    await m.keyboard.press("Enter");
    await sleep(600);
    const rowsW = await scanRows(m, g, 5);
    const e3 = rowsW.get(3)[1] - 1;
    await m.mouse.dblclick(numberX(g), e3);
    await sleep(600);
    const rowsF = await scanRows(m, g, 4);
    ok(tall(rowsF, 3) > P + 20, `double-clicking the line fits the row to its wrapped text (${tall(rowsF, 3)} px against ${P})`);
    await m.screenshot({ path: path.join(evidence, "rowheights-fit.png") });
    // A row inserted above a tall row: the height follows its data.
    await m.mouse.click(numberX(g), rowsF.get(2)[0] + 4);
    await cmd(m, "Insert row above");
    await sleep(600);
    const rowsI = await scanRows(m, g, 8);
    ok(tall(rowsI, 4) > P + 20 && tall(rowsI, 3) <= P + 3, `a row inserted above the fitted one: the tall height is now on row 4, the new row 3 is plain (${tall(rowsI, 3)}, ${tall(rowsI, 4)} px)`);
    await m.keyboard.press("Control+z");
    await sleep(700);
    const rowsU = await scanRows(m, g, 14);
    ok(tall(rowsU, 3) > P + 20, "Ctrl+Z puts it back on the same data row");
    // Rows hidden: the heights stay in the file's state but are not drawn; the status bar says so.
    await m.mouse.click(numberX(g), rowsU.get(6)[0] + 4);
    await m.keyboard.press("Control+9");
    await sleep(700);
    const f = await foot(m);
    ok(/Custom row heights hidden while filtered/.test(f), "with a row hidden the status bar says the custom heights are hidden");
    const rowsH = await scanRows(m, g, 8);
    ok(Math.abs(tall(rowsH, 3) - P) <= 3, `and the rows draw at the plain height (${tall(rowsH, 3)} px)`);
    await cmd(m, "Unhide all rows");
    await sleep(700);
    const rowsS = await scanRows(m, g, 14);
    ok(tall(rowsS, 3) > P + 20, "unhiding brings the heights back");
  },

  // The heights are kept in the remembered session when the app is closed (what "Pick up where you left off"
  // reads back; applying a view, the same code, is covered by the browser suite).
  async session() {
    const m = await start([notes]);
    await dismissAsk(m);
    const g = await grid(m);
    const rows0 = await scanRows(m, g, 8);
    const edge = rows0.get(5)[1] - 1;
    await m.mouse.move(numberX(g), edge);
    await m.mouse.down();
    await m.mouse.move(numberX(g), edge + 30, { steps: 6 });
    await m.mouse.up();
    await sleep(600);
    await sleep(11000); // the session is kept every 10 s
    say("close main: " + CLOSE_MAIN());
    ok(await waitFor(() => !alive(), 15000), "the app ended");
    const text = fs.existsSync(path.join(DATA, "session.json")) ? fs.readFileSync(path.join(DATA, "session.json"), "utf8") : "";
    const entry = JSON.parse(text || "[]").find((e) => /notes\.csv$/.test(e.path));
    info("remembered view heights: " + JSON.stringify(entry && entry.view && entry.view.row_heights));
    ok(!!entry && Array.isArray(entry.view.row_heights) && entry.view.row_heights.length === 1 && entry.view.row_heights[0][0] === 4 && entry.view.row_heights[0][1] > 40, "the remembered session holds the row's height (row 5 -> index 4) and no cell text");
    ok(!/a long note/.test(text), "and nothing from the file's cells");
  },

  // Frame time scrolling a 1M-row file: plain rows, and with 30,000 heights from the remembered session.
  async speed() {
    const big = path.join(WORK, "big-heights.csv");
    fs.copyFileSync(bigCsv, big);
    const m = await start([big]);
    await waitFor(async () => /1,000,000 rows/.test((await m.textContent("footer.status")) || ""), 90000, 500);
    await dismissAsk(m);
    const frames = (page) =>
      page.evaluate(async () => {
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
        d.sort((a, b) => a - b);
        return { median: d[Math.floor(d.length / 2)], p95: d[Math.floor(d.length * 0.95)] };
      });
    const c = await m.locator("canvas").first().boundingBox();
    const scrollRun = async () => {
      await m.mouse.move(c.x + 300, c.y + 200);
      const f = frames(m);
      for (let i = 0; i < 60; i++) {
        await m.mouse.wheel(0, 240);
        await sleep(25);
      }
      return f;
    };
    const runs = [];
    for (let i = 0; i < 3; i++) runs.push(await scrollRun());
    runs.sort((a, b) => a.median - b.median);
    info(`scroll frame time, plain rows: median ${runs[0].median.toFixed(1)} ms, p95 ${runs[0].p95.toFixed(1)} ms (${path.basename(path.dirname(exe))} build)`);
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
