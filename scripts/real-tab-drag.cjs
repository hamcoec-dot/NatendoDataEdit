// Drives the REAL built app (not the browser preview) over WebView2 remote
// debugging to check cells dragged onto another tab (copy, Shift: move): real
// mouse drags between two real files, a source with hidden rows, and speed on
// two 1,000,000-row files (a 100,000-cell drop against Copy then Paste).
//
// Usage (after `npx tauri build`):
//   NODE_PATH=<folder with playwright> node scripts/real-tab-drag.cjs <exe> <evidence folder> <big 1M-row csv> [scenarios]
// scenarios: a comma list of copy, hidden, speed (default: all). The app's own
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
  console.log("usage: node scripts/real-tab-drag.cjs <exe> <evidence folder> <big csv> [scenarios]");
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

// ---- scenarios: cells dropped on another tab ----
// Two small made-up files: s.csv (cells S1, S2, ...) and d.csv (D1, D2, ...).
const mk = (name, tag, rows) => {
  const p = path.join(WORK, name);
  fs.writeFileSync(p, "id,name,city\r\n" + Array.from({ length: rows }, (_, i) => `${tag}${i + 1},${tag}name ${i + 1},${tag}town ${i % 7}`).join("\r\n") + "\r\n");
  return p;
};
const sCsv = mk("s.csv", "S", 40);
const dCsv = mk("d.csv", "D", 40);
const cell = async (page, doc, row, col) => (await invoke(page, "cell_text", { doc, row, col })).text;
const tabEl = (page, name) => page.locator("[data-tab-index]").filter({ hasText: name }).first();
const showTab = async (page, name) => {
  const b = await tabEl(page, name).boundingBox();
  await page.mouse.click(b.x + b.width / 2 - 8, b.y + b.height / 2);
  await sleep(500);
};
const gotoBox = async (page, v) => {
  for (let i = 0; i < 5; i++) {
    await page.locator("input.goto").fill(v);
    await page.keyboard.press("Enter");
    await sleep(500);
    if ((await page.locator("input.goto").inputValue()) === v) return;
  }
};
/** Cell geometry from the canvas: the left edge of data column 0 and the row pitch. */
const geometry = async (page) => {
  const c = await page.locator("canvas").first().boundingBox();
  const HEAD = 25;
  const ROW = 25;
  // Column starts, found by clicking along a body row and reading the Name box.
  const starts = [];
  let last = 0;
  for (let x = 30; x < 700 && starts.length < 4; x += 2) {
    await page.mouse.click(c.x + x, c.y + HEAD + ROW + 12);
    const m = /C(\d+)$/.exec(await page.locator("input.goto").inputValue());
    const col = m ? Number(m[1]) : 0;
    if (col > last) {
      starts.push(x);
      last = col;
    }
  }
  // A 3-column file: the last column is taken to be as wide as the one before it.
  if (starts.length === 3) starts.push(starts[2] + (starts[2] - starts[1]));
  return { c, left: starts[0], starts, HEAD, ROW };
};
/** Drag the selection's left outline to a tab; optional Shift at release. */
const dragSelectionToTab = async (page, g, row, name, shift, shot) => {
  const tb = await tabEl(page, name).boundingBox();
  const x0 = g.c.x + g.starts[1] + 1;
  const y0 = g.c.y + g.HEAD + row * g.ROW + g.ROW / 2;
  await page.mouse.move(x0, y0);
  await page.mouse.down();
  await page.mouse.move(x0 + 8, y0, { steps: 2 });
  await page.mouse.move(tb.x + tb.width / 2 - 8, tb.y + tb.height / 2, { steps: 10 });
  const outlined = await page.locator(".tab.drop").count();
  if (shot) await page.screenshot({ path: path.join(evidence, shot) });
  if (shift) await page.keyboard.down("Shift");
  dragSelectionToTab.upAt = Date.now();
  await page.mouse.up();
  if (shift) await page.keyboard.up("Shift");
  await sleep(300);
  return outlined;
};
const msg = async (page) => ((await page.locator("footer.status .msg").textContent({ timeout: 1000 }).catch(() => "")) || "").replace(/\s+/g, " ").trim();

const scenarios = {
  // Copy and Shift-move between two real files, and the undo of each half.
  async copy() {
    const m = await start([sCsv, dCsv]);
    await waitFor(async () => (await tabNames(m)).length === 2, 20000);
    for (const n of ["s.csv", "d.csv"]) {
      await showTab(m, n);
      const yes = m.getByRole("button", { name: "Yes, column names" });
      if (await yes.isVisible().catch(() => false)) await yes.click();
      await sleep(300);
    }
    await showTab(m, "d.csv");
    await m.locator("canvas").first().click({ position: { x: 300, y: 130 } });
    const dAt = await m.locator("input.goto").inputValue();
    await showTab(m, "s.csv");
    const g = await geometry(m);
    await m.mouse.click(g.c.x + (g.starts[1] + g.starts[2]) / 2, g.c.y + g.HEAD + 1 * g.ROW + 12);
    await m.keyboard.down("Shift");
    await m.mouse.click(g.c.x + (g.starts[2] + g.starts[3]) / 2, g.c.y + g.HEAD + 2 * g.ROW + 12);
    await m.keyboard.up("Shift");
    const sel = await m.locator("input.goto").inputValue();
    info(`source selection ${sel}, destination cell ${dAt}`);
    const ids = { s: 1, d: 2 };
    const rc = /R(\d+) C(\d+)/.exec(dAt);
    const dRow = Number(rc[1]) - 1;
    const dCol = Number(rc[2]) - 1;
    const sRowM = /R(\d+) C(\d+)/.exec(sel);
    const sRow = Number(sRowM[1]) - 1;
    const sCol = Number(sRowM[2]) - 1;
    const want = [await cell(m, ids.s, sRow, sCol), await cell(m, ids.s, sRow, sCol + 1), await cell(m, ids.s, sRow + 1, sCol), await cell(m, ids.s, sRow + 1, sCol + 1)];
    const hist = (await invoke(m, "doc_info", { doc: ids.d })).history_pos;
    const over = await dragSelectionToTab(m, g, 1, "d.csv", false, "tabdrag-over.png");
    ok(over === 1, "the tab under the pointer is outlined while dragging");
    const got = [await cell(m, ids.d, dRow, dCol), await cell(m, ids.d, dRow, dCol + 1), await cell(m, ids.d, dRow + 1, dCol), await cell(m, ids.d, dRow + 1, dCol + 1)];
    ok(JSON.stringify(got) === JSON.stringify(want), `the 2x2 block landed at the other file's cell (${got.join(" | ")})`);
    ok(JSON.stringify([await cell(m, ids.s, sRow, sCol), await cell(m, ids.s, sRow + 1, sCol + 1)]) === JSON.stringify([want[0], want[3]]), "and the source still has it (a copy)");
    ok(/Copied 4 cells to d\.csv/.test(await msg(m)), `toast says so (${await msg(m)})`);
    ok((await invoke(m, "doc_info", { doc: ids.d })).history_pos === hist + 1, "one undo step in the other file");
    ok(/s\.csv/.test((await m.locator("[data-tab-index].active").first().textContent()) || ""), "the person stays on the source tab");
    await m.screenshot({ path: path.join(evidence, "tabdrag-copy.png") });
    // Shift: move.
    await m.mouse.click(g.c.x + (g.starts[1] + g.starts[2]) / 2, g.c.y + g.HEAD + 1 * g.ROW + 12);
    await m.keyboard.down("Shift");
    await m.mouse.click(g.c.x + (g.starts[2] + g.starts[3]) / 2, g.c.y + g.HEAD + 2 * g.ROW + 12);
    await m.keyboard.up("Shift");
    await showTab(m, "d.csv");
    await m.locator("canvas").first().click({ position: { x: 300, y: 210 } });
    const dAt2 = await m.locator("input.goto").inputValue();
    const rc2 = /R(\d+) C(\d+)/.exec(dAt2);
    const r2 = Number(rc2[1]) - 1;
    const c2 = Number(rc2[2]) - 1;
    await showTab(m, "s.csv");
    const h0 = (await invoke(m, "doc_info", { doc: ids.s })).history_pos;
    await dragSelectionToTab(m, g, 1, "d.csv", true);
    ok(/Moved 4 cells to d\.csv/.test(await msg(m)), `Shift-drop moves (${await msg(m)})`);
    ok((await cell(m, ids.d, r2, c2)) === want[0] && (await cell(m, ids.d, r2 + 1, c2 + 1)) === want[3], "the other file has the cells");
    ok((await cell(m, ids.s, sRow, sCol)) === "" && (await cell(m, ids.s, sRow + 1, sCol + 1)) === "", "and the source cells are cleared");
    ok((await invoke(m, "doc_info", { doc: ids.s })).history_pos === h0 + 1, "one undo step in the source");
    await m.keyboard.press("Control+z");
    await sleep(600);
    ok((await cell(m, ids.s, sRow, sCol)) === want[0], "Ctrl+Z in the source puts its cells back");
    await showTab(m, "d.csv");
    await m.keyboard.press("Control+z");
    await sleep(600);
    ok((await cell(m, ids.d, r2, c2)) !== want[0], "Ctrl+Z in the other file takes them out");
  },

  // A source with hidden rows: the copy and the clear skip the same rows.
  async hidden() {
    const m = await start([sCsv, dCsv]);
    await waitFor(async () => (await tabNames(m)).length === 2, 20000);
    await showTab(m, "s.csv");
    const g = await geometry(m);
    // Hide the rows at screen rows 3 and 4 (select their numbers, Ctrl+9).
    await m.mouse.click(g.c.x + 20, g.c.y + g.HEAD + 3 * g.ROW + 12);
    await m.keyboard.down("Shift");
    await m.mouse.click(g.c.x + 20, g.c.y + g.HEAD + 4 * g.ROW + 12);
    await m.keyboard.up("Shift");
    await m.keyboard.press("Control+9");
    await sleep(600);
    const info1 = await invoke(m, "doc_info", { doc: 1 });
    ok(info1.rows < info1.total_rows, `two rows are hidden (${info1.rows} of ${info1.total_rows} shown)`);
    // Select shown rows 1..6 of column B..C (it spans the hidden ones).
    await m.mouse.click(g.c.x + (g.starts[1] + g.starts[2]) / 2, g.c.y + g.HEAD + 1 * g.ROW + 12);
    await m.keyboard.down("Shift");
    await m.mouse.click(g.c.x + (g.starts[2] + g.starts[3]) / 2, g.c.y + g.HEAD + 6 * g.ROW + 12);
    await m.keyboard.up("Shift");
    const sel = await m.locator("input.goto").inputValue();
    const r0 = Number(/R(\d+)/.exec(sel)[1]);
    info(`selection ${sel}`);
    const before = [];
    for (let r = 0; r < 14; r++) before.push(await cell(m, 1, r, 1));
    await showTab(m, "d.csv");
    await m.locator("canvas").first().click({ position: { x: 300, y: 130 } });
    const dAt = /R(\d+) C(\d+)/.exec(await m.locator("input.goto").inputValue());
    const dRow = Number(dAt[1]) - 1;
    const dCol = Number(dAt[2]) - 1;
    await showTab(m, "s.csv");
    await m.mouse.click(g.c.x + (g.starts[1] + g.starts[2]) / 2, g.c.y + g.HEAD + 1 * g.ROW + 12);
    await m.keyboard.down("Shift");
    await m.mouse.click(g.c.x + (g.starts[2] + g.starts[3]) / 2, g.c.y + g.HEAD + 6 * g.ROW + 12);
    await m.keyboard.up("Shift");
    await dragSelectionToTab(m, g, 1, "d.csv", true);
    ok(/Moved \d+ cells to d\.csv/.test(await msg(m)), `Shift-move from a source with hidden rows (${await msg(m)})`);
    // What the other file got: the shown rows only (the 2 hidden rows inside the span are not copied).
    const got = [];
    for (let r = 0; r < 6; r++) got.push(await cell(m, 2, dRow + r, dCol));
    info(`destination column: ${got.join(" | ")}`);
    ok(JSON.stringify(got) === JSON.stringify(["Sname 2", "Sname 3", "Sname 6", "Sname 7", "Sname 8", "Sname 9"]), "the other file got exactly the 6 shown rows (not the hidden ones)");
    // The source: show the hidden rows again; they must still hold their values and exactly the 6 shown ones are cleared.
    await showTab(m, "s.csv");
    await m.locator("canvas").first().click({ position: { x: 300, y: 300 } });
    await m.keyboard.press("Control+a");
    await cmd(m, "Unhide rows in the selection");
    await sleep(900);
    info("rows now: " + JSON.stringify((await invoke(m, "doc_info", { doc: 1 })).rows));
    const after = [];
    for (let r = 0; r < 13; r++) after.push(await cell(m, 1, r, 1));
    info(`source after, hidden rows shown again: ${after.join("|")}`);
    ok(JSON.stringify(after) === JSON.stringify(["name", "Sname 1", "", "", "Sname 4", "Sname 5", "", "", "", "", "Sname 10", "Sname 11", "Sname 12"]), "the 2 hidden rows kept their values and exactly the 6 shown rows were cleared");
  },

  // Speed: a big block (about 80,000 cells, under the 100,000 cap) dropped between two 1M-row files, against the engine calls Copy then Paste make.
  async speed() {
    const a = path.join(WORK, "big-a.csv");
    const b = path.join(WORK, "big-b.csv");
    fs.copyFileSync(bigCsv, a);
    fs.copyFileSync(bigCsv, b);
    const m = await start([a, b]);
    await waitFor(async () => (await tabNames(m)).length === 2, 60000, 500);
    await showTab(m, "big-b.csv");
    await waitFor(async () => /1,000,000 rows/.test((await m.textContent("footer.status")) || ""), 120000, 500);
    await showTab(m, "big-a.csv");
    await waitFor(async () => /1,000,000 rows/.test((await m.textContent("footer.status")) || ""), 120000, 500);
    // B2 down by pages with Shift+PageDown until at least 40,000 rows (x 2 columns) are selected.
    const selectBlock = async () => {
      await gotoBox(m, "R2 C2");
      await m.keyboard.down("Shift");
      await m.keyboard.press("ArrowRight");
      await m.keyboard.up("Shift");
      let rows = 0;
      for (let i = 0; i < 400 && rows < 40000; i++) {
        for (let k = 0; k < 40; k++) await m.keyboard.press("Shift+PageDown");
        await sleep(150);
        const mm = /([\d,]+) × 2 selected/.exec((await m.textContent("footer.status")) || "");
        rows = mm ? Number(mm[1].replace(/,/g, "")) : rows;
      }
      return rows;
    };
    const rows = await selectBlock();
    const cells = rows * 2;
    info(`selected ${rows.toLocaleString()} rows x 2 = ${cells.toLocaleString()} cells`);
    ok(cells >= 80000 && cells <= 100000, "the block is between 80,000 and 100,000 cells");
    const g = await geometry(m);
    await selectBlock();
    // A: drop on the other tab (drag the left outline of the block).
    await dragSelectionToTab(m, g, 1, "big-b.csv", false);
    const done = await waitFor(async () => /Copied [\d,]+ cells to big-b/.test(await msg(m)), 180000, 50);
    const dropMs = Date.now() - dragSelectionToTab.upAt;
    info(`drop on a tab: ${dropMs} ms (message: ${await msg(m)})`);
    ok(done, "the big drop finished");
    // B: the same block copied and pasted by the engine calls Copy then Paste make.
    await showTab(m, "big-b.csv");
    // The same two backend calls with no clipboard in between (what Copy then Paste costs the engine).
    const rowsSel = rows;
    const b0 = Date.now();
    const text = await invoke(m, "copy_cells", { doc: 1, row: 1, rows: rowsSel, col: 1, cols: 2, skip: null });
    await invoke(m, "data_op", { doc: 2, op: { op: "paste", spec: { row: 1, col: 1, op: "none", skip_blanks: false, transpose: false, stop_at_end: false }, text } });
    const cpMs = Date.now() - b0;
    info(`copy_cells then data_op paste of ${rowsSel.toLocaleString()} rows x 2 (no clipboard): ${cpMs} ms`);
    // The UI's own Ctrl+V / Paste cannot be driven over remote debugging (the clipboard read is not answered), so the
    // comparison is with the two engine calls Copy then Paste make, plus a sanity bound for the screen work the drop adds
    // (reading the pasted text again, one undo step, the toast). The drop reuses the paste code, it does not replace it.
    ok(dropMs <= cpMs * 2 + 250, `the drop (${dropMs} ms from letting go) stays within twice the engine's own copy + paste (${cpMs} ms) plus 250 ms for screen work`);
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
