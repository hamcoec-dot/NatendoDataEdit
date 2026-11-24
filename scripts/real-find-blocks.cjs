// Drives the REAL built app (not the browser preview) over WebView2 remote
// debugging to check Find in several Ctrl+click blocks: it searches all of them in
// reading order, keeps them selected, Replace all is one undo step that touches
// only their cells, Enter and Tab walk the blocks in the order picked, the blocks
// are let go when the rows change, and the speed with 1,000 blocks on a
// 1,000,000-row file.
//
// Usage (after `npx tauri build`):
//   NODE_PATH=<folder with playwright> node scripts/real-find-blocks.cjs <exe> <evidence folder> <big 1M-row csv> [scenarios]
// scenarios: a comma list of basic, speed (default: all). The app's own
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
  console.log("usage: node scripts/real-find-blocks.cjs <exe> <evidence folder> <big csv> [scenarios]");
  process.exit(2);
}
const WORK = path.join(evidence, "work");
fs.mkdirSync(WORK, { recursive: true });
const DATA = path.join(process.env.APPDATA, "com.natendo.dataedit");
const LOCAL = path.join(process.env.LOCALAPPDATA, "com.natendo.dataedit");
const PORT = 9363;
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

const ctrlDragAt = async (page, a, b) => {
  await page.keyboard.down("Control");
  await page.mouse.move(...a);
  await page.mouse.down();
  await page.mouse.move(...b, { steps: 6 });
  await page.mouse.up();
  await page.keyboard.up("Control");
};
const gotoText = (page) => page.locator("input.goto").inputValue();
/** Where the active cell is, from the Name box ("R5 C2" -> row 4, col 1). */
const activeCell = async (page) => {
  const m = /R(\d+) C(\d+)/.exec(await gotoText(page));
  return m ? [Number(m[1]) - 1, Number(m[2]) - 1] : null;
};
const typeInFind = async (page, text) => {
  await page.locator('input[aria-label="Find"]').fill(text);
  await sleep(500);
};

const scenarios = {
  // Real mouse and keyboard: three blocks, Find walks them in reading order, Replace all and one Undo, Tab/Enter in pick order, blocks let go when rows change.
  async basic() {
    const m = await start([people]);
    await dismissAsk(m);
    let g = await geom(m);
    const doc = 1;
    // Block 1: a cell, block 2: 2 x 2, block 3: a column of 3 cells (picked last, active).
    // The anchor of each (the active cell right after picking it) gives its place in the file.
    const picked = [];
    const addBlock = (a, rows, cols) => {
      for (let r = 0; r < rows; r++) for (let c = 0; c < cols; c++) picked.push([a[0] + r, a[1] + c]);
    };
    await m.mouse.click(...cellXY(g, 2, 1));
    addBlock(await activeCell(m), 1, 1);
    await ctrlDragAt(m, cellXY(g, 6, 1), cellXY(g, 7, 2));
    addBlock(await activeCell(m), 2, 2);
    await ctrlDragAt(m, cellXY(g, 11, 1), cellXY(g, 13, 1));
    addBlock(await activeCell(m), 3, 1);
    const f0 = await foot(m);
    ok(/\d+ cells in 3 blocks selected/.test(f0), `three blocks selected (${f0.slice(0, 80)})`);
    const text = {};
    for (let r = 0; r < 40; r++) for (let c = 0; c < 3; c++) text[`${r},${c}`] = await cell(m, doc, r, c);
    const re = /er/;
    const matches = picked.filter(([r, c]) => re.test(text[`${r},${c}`]));
    info(`${picked.length} cells in the union, ${matches.length} match "er"`);
    ok(matches.length >= 4, "the blocks hold matches");
    await m.keyboard.press("Control+f");
    await sleep(500);
    const scope = await m.evaluate(() => {
      const s = document.querySelector("select[aria-label='Search in']");
      return s.options[s.selectedIndex].textContent;
    });
    ok(/3 blocks, \d+ cells/.test(scope), `Find opens on Selection and names the blocks (${scope})`);
    await typeInFind(m, "er");
    // F3 walks the matches in reading order (row by row across the blocks) and wraps.
    const start0 = await activeCell(m);
    const order = [...matches].sort((a, b) => a[0] - b[0] || a[1] - b[1]);
    const key = (r, c) => r * 10 + c;
    const expected = [];
    let cur = key(start0[0], start0[1]);
    for (let i = 0; i < order.length + 2; i++) {
      const nxt = order.find(([r, c]) => key(r, c) > cur) || order[0];
      expected.push(nxt);
      cur = key(nxt[0], nxt[1]);
    }
    const gotSeq = [];
    let kept = true;
    for (let i = 0; i < expected.length; i++) {
      await m.keyboard.press("F3");
      await sleep(300);
      gotSeq.push(await activeCell(m));
      if (!/3 blocks selected/.test(await foot(m))) kept = false;
    }
    ok(JSON.stringify(gotSeq) === JSON.stringify(expected), `F3 visits the matches of all the blocks in reading order and wraps (${JSON.stringify(gotSeq)} vs ${JSON.stringify(expected)})`);
    ok(kept, "all three blocks stay selected while Find next moves the active cell");
    await m.screenshot({ path: path.join(evidence, "find-blocks-real.png") });
    // Replace all: only the cells of the blocks change; one Ctrl+Z undoes it.
    const before = {};
    for (let r = 0; r < 40; r++) for (let c = 0; c < 3; c++) before[`${r},${c}`] = await cell(m, doc, r, c);
    const pos0 = (await invoke(m, "doc_info", { doc })).history_pos;
    await m.keyboard.press("Control+h");
    await sleep(400);
    await m.locator('input[aria-label="Replace with"]').fill("XY");
    await m.getByRole("button", { name: "Replace all…" }).click();
    await sleep(900);
    await m.getByRole("button", { name: "Replace all", exact: true }).click();
    await sleep(1500);
    let wrong = 0;
    const inPicked = new Set(picked.map(([r, c]) => `${r},${c}`));
    for (let r = 0; r < 40; r++)
      for (let c = 0; c < 3; c++) {
        const k = `${r},${c}`;
        const want = inPicked.has(k) ? before[k].replace(/er/g, "XY") : before[k];
        if ((await cell(m, doc, r, c)) !== want) wrong++;
      }
    ok(wrong === 0, `Replace all changed exactly the matching cells inside the blocks (${wrong} wrong of 120)`);
    ok((await invoke(m, "doc_info", { doc })).history_pos === pos0 + 1, "as one undo step");
    await m.getByLabel("Close find").click();
    await sleep(300);
    await m.locator("canvas").first().click({ position: { x: 600, y: 300 } });
    await m.keyboard.press("Control+z");
    await sleep(900);
    let back = 0;
    for (let r = 0; r < 40; r++) for (let c = 0; c < 3; c++) if ((await cell(m, doc, r, c)) !== before[`${r},${c}`]) back++;
    ok(back === 0, "one Ctrl+Z puts every cell back");
    // Enter/Tab in pick order: three single cells picked, Tab goes 1 -> 2 -> 3 -> 1, Shift+Tab back.
    g = await geom(m); // (the Find bar is closed: the grid is back at the top)
    await m.mouse.click(...cellXY(g, 3, 0));
    const p1 = await activeCell(m);
    await ctrlClickAt(m, ...cellXY(g, 9, 2));
    const p2 = await activeCell(m);
    await ctrlClickAt(m, ...cellXY(g, 5, 1));
    const p3 = await activeCell(m);
    info("before Tab: " + (await foot(m)).slice(0, 120) + " | active " + JSON.stringify(await activeCell(m)));
    await m.screenshot({ path: path.join(evidence, "find-blocks-tab.png") });
    const tabs = [];
    for (let i = 0; i < 4; i++) {
      await m.keyboard.press("Tab");
      await sleep(150);
      tabs.push(await activeCell(m));
    }
    const exp = [p1, p2, p3, p1];
    ok(JSON.stringify(tabs) === JSON.stringify(exp), `Tab goes block to block in the order picked, not top to bottom (${JSON.stringify(tabs)} vs ${JSON.stringify(exp)})`);
    await m.keyboard.press("Shift+Tab");
    await sleep(150);
    ok(JSON.stringify(await activeCell(m)) === JSON.stringify(p3), "Shift+Tab goes back to the previous pick");
    await m.keyboard.press("Enter");
    await sleep(150);
    ok(JSON.stringify(await activeCell(m)) === JSON.stringify(p1), "Enter goes on to the next block too");
    ok(/3 blocks selected/.test(await foot(m)), "the blocks stay selected");
    // The blocks are let go when the rows change while Find is open.
    await m.keyboard.press("Control+f");
    await sleep(400);
    const open2 = await m.evaluate(() => {
      const s = document.querySelector("select[aria-label='Search in']");
      return s.options[s.selectedIndex].textContent;
    });
    ok(/3 blocks/.test(open2), `Find opened on the three picks (${open2})`);
    await m.locator("canvas").first().click({ position: { x: 700, y: 400 } });
    await sleep(200);
    await cmd(m, "Insert row above");
    await sleep(900);
    const rel = await m.evaluate(() => ({ scope: document.querySelector("select[aria-label='Search in']")?.value, text: document.querySelector(".find")?.textContent || "" }));
    ok(rel.scope === "all" && /selected blocks were let go/.test(rel.text), `rows changed while Find was open: the blocks were let go with a message (${rel.scope})`);
  },

  // 1,000 blocks on a 1,000,000-row file: how long Find next and the count take, against the whole file.
  async speed() {
    const big = path.join(WORK, "big-find-blocks.csv");
    fs.copyFileSync(bigCsv, big);
    const m = await start([big]);
    await waitFor(async () => /1,000,000 rows/.test((await m.textContent("footer.status")) || ""), 90000, 500);
    await dismissAsk(m);
    const g = await geom(m);
    const timeKey = async (key, want) => {
      await m.evaluate(() => {
        window.__t0 = 0;
        window.addEventListener("keydown", () => (window.__t0 = performance.now()), { capture: true, once: true });
      });
      await m.keyboard.press(key);
      return m.evaluate(
        (w) =>
          new Promise((resolve) => {
            const t0 = window.__t0 || performance.now();
            const poll = () => {
              const v = document.querySelector("input.goto")?.value || "";
              if (new RegExp(w).test(v)) requestAnimationFrame(() => requestAnimationFrame(() => resolve(performance.now() - t0)));
              else if (performance.now() - t0 > 30000) resolve(-1);
              else requestAnimationFrame(poll);
            };
            poll();
          }),
        want,
      );
    };
    // The whole file first: Find next for a word, count shown.
    await m.keyboard.press("Control+f");
    await sleep(500);
    await typeInFind(m, "an");
    await sleep(1500);
    const t1 = await timeKey("F3", "R\\d+ C\\d+");
    info(`whole file: first Find next ${t1.toFixed(0)} ms, count: ${await m.evaluate(() => document.querySelector(".count")?.textContent || "")}`);
    await m.keyboard.press("Escape");
    // 1,000 single-cell blocks down the first columns.
    await m.mouse.click(...cellXY(g, 1, 0));
    let n = 1;
    for (let page = 0; page < 40 && n < 1000; page++) {
      for (let r = 1; r <= 24 && n < 1000; r++)
        for (const c of [0, 1, 2]) {
          if (n >= 1000) break;
          await ctrlClickAt(m, ...cellXY(g, r, c));
          n++;
        }
      await m.mouse.move(g.c.x + 300, g.c.y + 200);
      for (let w = 0; w < 14; w++) {
        await m.mouse.wheel(0, 240);
        await sleep(15);
      }
      await sleep(80);
    }
    const f1 = await foot(m);
    ok(/9\d\d cells in 9\d\d blocks|1,000 cells in 1,000 blocks/.test(f1), `about 1,000 blocks selected (${f1.slice(0, 70)})`);
    await m.keyboard.press("Control+f");
    await sleep(800);
    const scope = await m.evaluate(() => {
      const s = document.querySelector("select[aria-label='Search in']");
      return s.options[s.selectedIndex].textContent;
    });
    ok(/\d+ blocks/.test(scope), `Find names the blocks (${scope})`);
    await typeInFind(m, "an");
    await sleep(2500);
    const cnt = await m.evaluate(() => document.querySelector(".count")?.textContent || "");
    info(`1,000 blocks: count shown: ${cnt}`);
    const t2 = await timeKey("F3", "R\\d+ C\\d+");
    ok(t2 >= 0 && t2 < 2000, `Find next over 1,000 blocks answers in ${t2.toFixed(0)} ms`);
    const t3 = await timeKey("F3", "R\\d+ C\\d+");
    info(`1,000 blocks: next Find next ${t3.toFixed(0)} ms`);
    const hold = /9\d\d cells in 9\d\d blocks|1,000 cells in 1,000 blocks/.test(await foot(m));
    ok(hold, "all the blocks stay selected");
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

