// Drives the REAL built app (not the browser preview) over WebView2 remote
// debugging to check Ctrl+click selection of several blocks: 20 cells picked,
// Delete as one undo step, totals against a hand count, everything that needs
// one block refused with its message, and speed with 1,000 blocks selected on a
// 1,000,000-row file.
//
// Usage (after `npx tauri build`):
//   NODE_PATH=<folder with playwright> node scripts/real-multi-area.cjs <exe> <evidence folder> <big 1M-row csv> [scenarios]
// scenarios: a comma list of basic, refuse, speed (default: all). The app's own
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
  console.log("usage: node scripts/real-multi-area.cjs <exe> <evidence folder> <big csv> [scenarios]");
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

const scenarios = {
  // Twenty cells with Ctrl+click: Delete clears exactly those (one undo step), totals match a hand count.
  async basic() {
    const m = await start([people]);
    await dismissAsk(m);
    const g = await geom(m);
    // Screen rows 1..8 are rows R3..R10 here (the title row is R1, the first data row R2).
    const picked = [];
    await m.mouse.click(...cellXY(g, 1, 2));
    picked.push(cellXY(g, 1, 2));
    for (let r = 2; r <= 20; r++) {
      const [x, y] = cellXY(g, r, r % 2 === 0 ? 2 : 0);
      await ctrlClickAt(m, x, y);
      picked.push([x, y]);
    }
    const f = await foot(m);
    info(`footer: ${f.slice(0, 160)}`);
    ok(/20 cells in 20 blocks selected/.test(f), "20 cells picked with Ctrl+click: the status bar counts 20 cells in 20 blocks");
    // Totals: the score cells picked (column 3) are 3 x their row number.
    const scoreRows = [];
    for (let r = 1; r <= 20; r++) if (r === 1 || r % 2 === 0) scoreRows.push(r);
    // Their values, read back from the file, summed by hand.
    const doc = 1;
    let hand = 0;
    let handCount = 0;
    for (let r = 1; r <= 20; r++) {
      const col = r === 1 ? 2 : r % 2 === 0 ? 2 : 0;
      const v = Number(await cell(m, doc, r + 1, col));
      if (!Number.isNaN(v)) {
        hand += v;
        handCount++;
      }
    }
    await sleep(1200);
    const f2 = await foot(m);
    const sumShown = /Sum:\s*([\d,.]+)/.exec(f2);
    const cntShown = /Count:\s*([\d,]+)/.exec(f2);
    ok(sumShown && Number(sumShown[1].replace(/,/g, "")) === hand, `the status bar's Sum matches a hand count (${sumShown && sumShown[1]} vs ${hand})`);
    ok(cntShown && Number(cntShown[1].replace(/,/g, "")) === handCount, `and its Count (${cntShown && cntShown[1]} vs ${handCount})`);
    await m.screenshot({ path: path.join(evidence, "multiarea-20.png") });
    const before = [];
    for (let r = 0; r < 24; r++) for (let c = 0; c < 3; c++) before.push(await cell(m, doc, r, c));
    const pos0 = (await invoke(m, "doc_info", { doc })).history_pos;
    await m.keyboard.press("Delete");
    await sleep(900);
    const after = [];
    for (let r = 0; r < 24; r++) for (let c = 0; c < 3; c++) after.push(await cell(m, doc, r, c));
    // Exactly the picked cells (display rows r+1 for screen row r) are blank now.
    const pickedIdx = new Set();
    pickedIdx.add(2 * 3 + 2); // screen row 1 -> doc row 2, col 2
    for (let r = 2; r <= 20; r++) pickedIdx.add((r + 1) * 3 + (r % 2 === 0 ? 2 : 0));
    let wrong = 0;
    for (let i = 0; i < after.length; i++) {
      const should = pickedIdx.has(i) ? "" : before[i];
      if (after[i] !== should) wrong++;
    }
    ok(wrong === 0, `Delete cleared exactly the 20 picked cells and nothing else (${wrong} cells differ from that)`);
    ok((await invoke(m, "doc_info", { doc })).history_pos === pos0 + 1, "as one undo step");
    await m.keyboard.press("Control+z");
    await sleep(900);
    const back = [];
    for (let r = 0; r < 24; r++) for (let c = 0; c < 3; c++) back.push(await cell(m, doc, r, c));
    ok(JSON.stringify(back) === JSON.stringify(before), "one Ctrl+Z puts all 20 back");
  },

  // Everything that needs one block says so and changes nothing.
  async refuse() {
    const m = await start([people]);
    await dismissAsk(m);
    const g = await geom(m);
    const doc = 1;
    await m.mouse.click(...cellXY(g, 1, 0));
    await ctrlClickAt(m, ...cellXY(g, 3, 2));
    const before = [];
    for (let r = 0; r < 12; r++) for (let c = 0; c < 3; c++) before.push(await cell(m, doc, r, c));
    const pos0 = (await invoke(m, "doc_info", { doc })).history_pos;
    const tries = [
      ["Fill down (Ctrl+D)", async () => m.keyboard.press("Control+d")],
      ["Sort A to Z", async () => cmd(m, "Sort A to Z")],
      ["Insert row above", async () => cmd(m, "Insert row above")],
      ["Cut (Ctrl+X)", async () => m.keyboard.press("Control+x")],
      ["Remove duplicates", async () => cmd(m, "Remove duplicates")],
    ];
    for (const [name, f] of tries) {
      await m.mouse.click(...cellXY(g, 1, 0));
      await ctrlClickAt(m, ...cellXY(g, 3, 2));
      await f();
      await sleep(500);
      const said = await msg(m);
      const same = (await invoke(m, "doc_info", { doc })).history_pos === pos0;
      ok(/works on one block/i.test(said) && same, `${name}: refused with the one-block message and nothing changed (${said.slice(0, 70)})`);
      await m.keyboard.press("Escape");
      await m.keyboard.press("Escape");
    }
    // The fill handle is not drawn: a drag from where it would be changes nothing.
    await m.mouse.click(...cellXY(g, 1, 0));
    await ctrlClickAt(m, ...cellXY(g, 3, 2));
    const [hx, hy] = cellXY(g, 3, 2);
    await m.mouse.move(hx + (g.starts[3] - g.starts[2]) / 2 - 2, hy + 10);
    await m.mouse.down();
    await m.mouse.move(hx + 5, hy + 120, { steps: 6 });
    await m.mouse.up();
    await sleep(500);
    ok((await invoke(m, "doc_info", { doc })).history_pos === pos0, "a drag from the fill handle's place changes nothing");
    const after = [];
    for (let r = 0; r < 12; r++) for (let c = 0; c < 3; c++) after.push(await cell(m, doc, r, c));
    ok(JSON.stringify(after) === JSON.stringify(before), "the cells are untouched after every refusal");
    // Typing writes the active cell only; the earlier block goes.
    await m.mouse.click(...cellXY(g, 1, 0));
    await ctrlClickAt(m, ...cellXY(g, 3, 2));
    await m.keyboard.type("only");
    await m.keyboard.press("Enter");
    await sleep(600);
    ok((await cell(m, doc, 4, 2)) === "only" && (await cell(m, doc, 2, 0)) === before[2 * 3 + 0], "typing writes the active cell only (Excel's way)");
  },

  // Clearing whole 1,000,000-row columns: two column blocks against one.
  async clear() {
    const big = path.join(WORK, "big-multi-clear.csv");
    fs.copyFileSync(bigCsv, big);
    const m = await start([big]);
    await waitFor(async () => /1,000,000 rows/.test((await m.textContent("footer.status")) || ""), 90000, 500);
    await dismissAsk(m);
    const g = await geom(m);
    const doc = 1;
    const head = (col) => [g.c.x + (g.starts[col] + g.starts[col + 1]) / 2, g.c.y + g.HEAD / 2];
    const hist = async () => (await invoke(m, "doc_info", { doc })).history_pos;
    const timeDelete = async (cols) => {
      await m.mouse.click(...head(cols[0]));
      for (const c of cols.slice(1)) await ctrlClickAt(m, ...head(c));
      const h0 = await hist();
      const t0 = Date.now();
      await m.keyboard.press("Delete");
      const done = await waitFor(async () => (await hist()) > h0, 120000, 20);
      return { done, ms: Date.now() - t0, steps: (await hist()) - h0 };
    };
    const one = await timeDelete([1]);
    info(`Delete of one whole 1M-row column: ${one.ms} ms, ${one.steps} history step(s)`);
    await m.keyboard.press("Control+z");
    await sleep(800);
    const two = await timeDelete([1, 2]);
    info(`Delete of two whole 1M-row column blocks: ${two.ms} ms, ${two.steps} history step(s)`);
    ok(one.done && two.done, "both deletes finished");
    ok(two.ms <= one.ms * 2.2 + 150, `two column blocks take no more than about twice one (${one.ms} ms vs ${two.ms} ms: the same engine work per column)`);
    const h1 = await hist();
    await m.keyboard.press("Control+z");
    await sleep(1000);
    ok((await hist()) <= h1 - 1 && (await cell(m, doc, 5, 1)) !== "" && (await cell(m, doc, 5, 2)) !== "", "one Ctrl+Z puts both columns back");
  },

  // Speed with 1,000 blocks: scrolling, and a Ctrl+click.
  async speed() {
    const big = path.join(WORK, "big-multi.csv");
    fs.copyFileSync(bigCsv, big);
    const m = await start([big]);
    await waitFor(async () => /1,000,000 rows/.test((await m.textContent("footer.status")) || ""), 90000, 500);
    await dismissAsk(m);
    const g = await geom(m);
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
    const scrollRun = async () => {
      const c = await m.locator("canvas").first().boundingBox();
      await m.mouse.move(c.x + 300, c.y + 200);
      const f = frames(m);
      for (let i = 0; i < 60; i++) {
        await m.mouse.wheel(0, 240);
        await sleep(25);
      }
      return f;
    };
    await gotoOne(m, "R1 C1");
    const none = await scrollRun();
    info(`scroll frame time, no selection: median ${none.median.toFixed(1)} ms, p95 ${none.p95.toFixed(1)} ms`);
    // 1,000 single cells: 20 screens of 50 cells.
    await gotoOne(m, "R2 C1");
    await m.mouse.click(...cellXY(g, 1, 0));
    let n = 1;
    const clickMs = [];
    for (let page = 0; page < 40 && n < 1000; page++) {
      for (let r = 1; r <= 24 && n < 1000; r++) {
        for (const c of [0, 1]) {
          if (n >= 1000) break;
          const [x, y] = cellXY(g, r, c);
          const t0 = Date.now();
          await ctrlClickAt(m, x, y);
          clickMs.push(Date.now() - t0);
          n++;
        }
      }
      // One screen down: the wheel moves a few rows a step, so several steps.
      await m.mouse.move(g.c.x + 300, g.c.y + 200);
      for (let w = 0; w < 14; w++) {
        await m.mouse.wheel(0, 240);
        await sleep(15);
      }
      await sleep(80);
    }
    const f1 = await foot(m);
    info(`after ${n} picks: footer says "${f1.slice(0, 100)}"`);
    ok(/1,000 cells in 1,000 blocks|9\d\d cells in 9\d\d blocks/.test(f1), "about 1,000 blocks selected");
    clickMs.sort((a, b) => a - b);
    info(`Ctrl+click round trip (mouse down to up as the test sees it): median ${clickMs[Math.floor(clickMs.length / 2)]} ms, p95 ${clickMs[Math.floor(clickMs.length * 0.95)]} ms`);
    // Back to the top with the wheel (Go To would replace the selection).
    for (let i = 0; i < 700; i++) {
      await m.mouse.wheel(0, -240);
      await sleep(5);
    }
    const many = await scrollRun();
    info(`scroll frame time with ~1,000 blocks selected: median ${many.median.toFixed(1)} ms, p95 ${many.p95.toFixed(1)} ms`);
    ok(many.median <= none.median * 1.1 + 0.5, `scrolling with 1,000 blocks stays within 10% of no selection (median ${none.median.toFixed(1)} -> ${many.median.toFixed(1)} ms)`);
  },
};

async function gotoOne(page, v) {
  // (Go To replaces the selection; used only before blocks are picked or after, to move the view.)
  for (let i = 0; i < 4; i++) {
    await page.locator("input.goto").fill(v);
    await page.keyboard.press("Enter");
    await sleep(500);
    if ((await page.locator("input.goto").inputValue()) === v) return;
  }
}

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
