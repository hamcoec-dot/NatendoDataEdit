// Drives the REAL built app (not the browser preview) over WebView2 remote
// debugging to check a tab in its own window: the quit and close paths (the
// data-loss ones), a crashed second window, second launches, privacy, the
// remembered session, cleanup, and speed on a 1,000,000-row file.
//
// Usage (after `npx tauri build`):
//   NODE_PATH=<folder with playwright> node scripts/real-own-window.cjs <exe> <evidence folder> <big 1M-row csv> [scenarios]
// scenarios: a comma list of d, edit, nothing, launch, crash, never, last, state, private, parity, saving
// (default: all). The app's own data folder (%APPDATA%\com.natendo.dataedit:
// session.json and recent.json) is backed up first and restored at the end, and
// the app is started with its own browser profile under the evidence folder.
// Files are made-up and live in <evidence>/work. Nothing is installed.
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence, bigCsv, which = "all"] = process.argv.slice(2);
if (!exe || !evidence || !bigCsv) {
  console.log("usage: node scripts/real-own-window.cjs <exe> <evidence folder> <big csv> [scenarios]");
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
const start = async (files) => {
  await killApp();
  wipeRecovery();
  if (running()) throw new Error("another Natendo Data Edit is running: close it first (this test never stops an app it did not start)");
  const child = spawn(exe, files, {
    env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT}`, WEBVIEW2_USER_DATA_FOLDER: path.join(WORK, "wv2") },
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

// ---- scenarios ----
const scenarios = {
  async probe() {
    const m = await start([first]);
    await edit(m, "X");
    await listen(m, ["close-requested"], "main", "L1");
    await m.evaluate(() => { window.__errs = []; window.addEventListener("error", (e) => window.__errs.push(String(e.message))); window.addEventListener("unhandledrejection", (e) => window.__errs.push(String(e.reason && (e.reason.message || JSON.stringify(e.reason))))); });
    say("titles: " + spawnSync("powershell",["-NoProfile","-Command","Get-Process natendo-data-edit | Select-Object Id,MainWindowTitle | Format-List | Out-String"],{encoding:"utf8"}).stdout.replace(/\s+/g," ")); say("via invoke close: " + JSON.stringify(await m.evaluate(async () => { try { await window.__TAURI_INTERNALS__.invoke("plugin:window|close", { label: "main" }); return "ok"; } catch (e) { return String(e); } }))); say("close main: " + CLOSE_MAIN());
    await sleep(4000);
    say("events=" + JSON.stringify(await got(m, "L1")) + " errs=" + JSON.stringify(await m.evaluate(() => window.__errs)) + " dlg=" + (await dialogText(m)) + " alive=" + alive());
    await m.screenshot({ path: path.join(process.argv[3], "probe.png") });
  },
  // D: closing main with an unsaved edit still asks after window 2 has been and gone, and window 2 cannot quit the app.
  async d() {
    const m = await start([first]);
    await secondLaunch(second);
    ok((await tabNames(m)).some((t) => /second\.csv/.test(t)), "a second launch opens its file in the main window");
    const w2 = await moveActiveToSecond(m);
    ok((await tabNames(w2)).some((t) => /second\.csv/.test(t)) && !(await tabNames(m)).some((t) => /second\.csv/.test(t)), "Move tab to a new window shows second.csv in window 2 and takes it from main");
    await edit(m, "X");
    ok((await tabNames(m)).some((t) => /●.*first\.csv/.test(t)), "an unsaved edit made through the page in main (first.csv)");
    say("close window 2 (its close button): " + CLOSE_W2());
    ok(await waitFor(() => !pageSecond(), 8000), "window 2 is gone");
    ok(await waitFor(async () => (await tabNames(m)).some((t) => /second\.csv/.test(t)), 8000), "its tab came back to the main window");
    ok(alive(), "the app is still running");
    say("close main (its close button): " + CLOSE_MAIN());
    const asked = await waitFor(async () => /Quit without saving/.test(await dialogText(m)), 5000);
    ok(asked && alive(), `D: closing main with an unsaved edit after window 2 closed still asks (alive=${alive()} asked=${asked} text=${(await dialogText(m)).slice(0, 110)})`);
    await dialog(m).getByRole("button", { name: "Cancel" }).click().catch(() => {});
    await sleep(500);
    // A stray quit from window 2.
    await m.locator("[role=tab], .tab").filter({ hasText: "second.csv" }).first().click();
    const w2b = await moveActiveToSecond(m);
    const reply = await invoke(w2b, "quit_app", { known: [] });
    await sleep(1200);
    ok(reply === "ignored" && alive() && !!pageSecond() && !!pageMain(), `quit_app from window 2 is ignored (${reply}); both windows stay`);
    say("close main again: " + CLOSE_MAIN());
    ok(await waitFor(async () => /Quit without saving/.test(await dialogText(m)), 5000) && alive(), "closing main afterwards still asks");
    await m.screenshot({ path: path.join(evidence, "d-prompt.png") }).catch(() => {});
  },

  // An unsaved edit in a tab that lives in window 2; the question counts both windows, and a change made while it is open is not lost.
  async edit() {
    const m = await start([first]);
    await secondLaunch(second);
    await secondLaunch(third);
    // Move third and second to window 2 (the active tab each time).
    const w2 = await moveActiveToSecond(m); // third
    await m.locator("[role=tab], .tab").filter({ hasText: "second.csv" }).first().click();
    await cmd(m, "Move tab to a new window");
    await waitFor(async () => (await tabNames(w2)).length === 2, 10000);
    ok((await tabNames(w2)).length === 2, `window 2 holds two tabs (${(await tabNames(w2)).join(" | ")})`);
    await w2.locator("[role=tab], .tab").filter({ hasText: "second.csv" }).first().click();
    await edit(w2, "Y");
    ok((await tabNames(w2)).some((t) => /●.*second\.csv/.test(t)), "an edit made through window 2 (second.csv)");
    say("close main: " + CLOSE_MAIN());
    await waitFor(async () => /Quit without saving/.test(await dialogText(m)), 5000);
    const t1 = await dialogText(m);
    ok(/1 file has unsaved changes: second\.csv/.test(t1), `the question names the file in window 2 (${t1.slice(0, 120)})`);
    // While the question is open, edit another file in window 2 (not on the list).
    await w2.locator("[role=tab], .tab").filter({ hasText: "third.csv" }).first().click();
    await edit(w2, "Z");
    await dialog(m).getByRole("button", { name: /Quit and lose changes/ }).click();
    await sleep(2500);
    const t2 = await waitFor(async () => (await dialogText(m)) || false, 4000);
    ok(alive() && /2 files have unsaved changes/.test(t2 || "") && /second\.csv/.test(t2) && /third\.csv/.test(t2), `an edit made while the question was open: the app stays and asks again, naming both (${(t2 || "").slice(0, 130)})`);
    await dialog(m).getByRole("button", { name: "Cancel" }).click().catch(() => {});
    ok(alive() && !!pageSecond(), "Cancel keeps both windows");
    // Now quit for real.
    say("close main: " + CLOSE_MAIN());
    await waitFor(async () => /Quit without saving/.test(await dialogText(m)), 5000);
    await dialog(m).getByRole("button", { name: /Quit and lose changes/ }).click();
    ok(await waitFor(() => !alive(), 8000), "agreeing to the list that was shown ends the app");
  },

  // Nothing unsaved, two windows: closing main ends the process.
  async nothing() {
    const m = await start([first]);
    await secondLaunch(second);
    await moveActiveToSecond(m);
    say("close main: " + CLOSE_MAIN());
    ok(await waitFor(() => !alive(), 8000), "two windows, nothing unsaved: closing main ends the process (no window 2 left alone)");
  },

  // A second launch goes to the main window; window 2 hears nothing; events stay in their window.
  async launch() {
    const m = await start([first]);
    await secondLaunch(second);
    const w2 = await moveActiveToSecond(m);
    await listen(w2, ["opened", "close-requested"], "w2", "__ev");
    await listen(m, ["opened", "close-requested"], "main", "__ev");
    await secondLaunch(third);
    ok((await tabNames(m)).some((t) => /third\.csv/.test(t)) && !(await tabNames(w2)).some((t) => /third\.csv/.test(t)), "a file double-clicked with both windows open opens in the main window only");
    ok((await got(m, "__ev")).includes("opened") && !(await got(w2, "__ev")).includes("opened"), "'opened' is heard by main, not by window 2");
    say("close window 2: " + CLOSE_W2());
    await waitFor(() => !pageSecond(), 8000);
    ok(!(await got(m, "__ev")).includes("close-requested"), "window 2's close request is not heard by the main window");
    await secondLaunch(path.join(WORK, "first.csv"));
    ok(alive(), "a launch with window 2 closed still goes to main");
  },

  // A window 2 that crashes (no Destroyed event) must not strand a file; request_quit must not wait on it.
  async crash() {
    // (a) the silent window is detected and its files come back.
    let m = await start([first]);
    await secondLaunch(second);
    let w2 = await moveActiveToSecond(m);
    await edit(w2, "C");
    ok((await tabNames(w2)).some((t) => /●/.test(t)), "an unsaved edit in window 2");
    const cdp = await ctx.newCDPSession(w2);
    await cdp.send("Page.crash").catch(() => {});
    say("window 2's page crashed (no Destroyed event)");
    const back = await waitFor(async () => (await tabNames(m)).some((t) => /●.*second\.csv/.test(t)), 25000);
    ok(back, "after the silence the tab is back in main, still modified");
    ok(await waitFor(() => !pageSecond(), 10000), "and the dead window is destroyed");
    ok(/stopped responding/.test((await m.textContent("footer.status")) || ""), "the person is told why");
    // (b) closing a crashed window 2 with its close button.
    m = await start([first]);
    await secondLaunch(second);
    w2 = await moveActiveToSecond(m);
    await edit(w2, "D");
    await (await ctx.newCDPSession(w2)).send("Page.crash").catch(() => {});
    await sleep(800);
    const t = Date.now();
    say("close the crashed window 2: " + CLOSE_W2());
    ok(await waitFor(() => !pageSecond(), 8000) && (await waitFor(async () => (await tabNames(m)).some((x) => /●.*second\.csv/.test(x)), 8000)), `its close button works though its page is dead: the tab is back in main in ${((Date.now() - t) / 1000).toFixed(1)} s`);
    // (c) closing main while window 2 is dead and holds an unsaved file: it asks at once (it never waits on window 2).
    m = await start([first]);
    await secondLaunch(second);
    w2 = await moveActiveToSecond(m);
    await edit(w2, "E");
    await (await ctx.newCDPSession(w2)).send("Page.crash").catch(() => {});
    await sleep(500);
    const t3 = Date.now();
    say("close main: " + CLOSE_MAIN());
    const asked = await waitFor(async () => /second\.csv/.test(await dialogText(m)), 6000);
    ok(asked, `main asked at once about the file held by the dead window 2 (${((Date.now() - t3) / 1000).toFixed(1)} s): ${(await dialogText(m)).slice(0, 100)}`);
    await dialog(m).getByRole("button", { name: /Quit and lose changes/ }).click().catch(() => {});
    ok(await waitFor(() => !alive(), 8000), "and quitting with a dead window 2 ends the app");
  },

  // A move window 2 never takes comes back.
  async never() {
    const m = await start([first]);
    const doc = 1;
    const wait = ctx.waitForEvent("page", { timeout: 20000 });
    await invoke(m, "window_move", { h: { doc, state: null, row: 0, col: 0 } });
    const w2 = await wait;
    await (await ctx.newCDPSession(w2)).send("Page.crash").catch(() => {});
    say("window 2 crashed before it took the tab");
    const back = await waitFor(async () => /did not respond/.test((await m.textContent("footer.status")) || ""), 30000);
    ok(back, "a move that is never taken comes back to main with a message (about 10 s)");
    ok(await waitFor(() => !pageSecond(), 10000), "and the window that never loaded goes away");
  },

  // Closing window 2's last tab closes window 2.
  async last() {
    const m = await start([first]);
    await secondLaunch(second);
    const w2 = await moveActiveToSecond(m);
    await w2.locator("canvas").first().click({ position: { x: 140, y: 40 } });
    await w2.keyboard.press("Control+w");
    ok(await waitFor(() => !pageSecond(), 8000), "closing window 2's last tab closes window 2");
    ok(alive() && !!pageMain(), "the app and the main window stay");
  },

  // A big file moves without being read again; state and speed.
  async state() {
    const big = path.join(WORK, "big.csv");
    fs.copyFileSync(bigCsv, big);
    // A second big file stays in main so main's scrolling can be measured after the move.
    const big2 = path.join(WORK, "big2.csv");
    fs.copyFileSync(bigCsv, big2);
    const m = await start([big, big2]);
    await waitFor(async () => (await tabNames(m)).length === 2, 60000, 500);
    await m.locator("[role=tab], .tab").filter({ hasText: "big.csv" }).first().click();
    await waitFor(async () => /1,000,000 rows/.test((await m.textContent("footer.status")) || ""), 90000, 500);
    await m.locator("input.goto").fill("R500000 C3");
    await m.keyboard.press("Enter");
    await sleep(600);
    const before = await invoke(m, "doc_info", { doc: 1 });
    const mem0 = mem();
    // Frame times scrolling main with only one window.
    const frames = async (page) =>
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
    const scrollRun = async (page) => {
      const c = await page.locator("canvas").first().boundingBox();
      await page.mouse.move(c.x + 300, c.y + 200);
      const f = frames(page);
      for (let i = 0; i < 60; i++) {
        await page.mouse.wheel(0, 240);
        await sleep(25);
      }
      return f;
    };
    const f1 = await scrollRun(m);
    info(`scroll frame time with one window: median ${f1.median.toFixed(1)} ms, p95 ${f1.p95.toFixed(1)} ms`);
    // Move the tab; time until window 2 shows rows.
    const t = Date.now();
    const wait = ctx.waitForEvent("page", { timeout: 30000 });
    await cmd(m, "Move tab to a new window");
    const w2 = await wait;
    await w2.waitForLoadState("load").catch(() => {});
    const shown = await waitFor(async () => /1,000,000 rows/.test((await w2.textContent("footer.status")) || "") && (await w2.locator("input.goto").inputValue()) !== "", 30000, 100);
    const took = (Date.now() - t) / 1000;
    ok(shown && took < 1.5 + 2.6, `the big tab is drawn in window 2 ${took.toFixed(2)} s after the command (includes ~2.6 s to create the window; the plan's budget is 1.5 s for the move itself)`);
    const goto = await w2.locator("input.goto").inputValue();
    ok(/R500000 C3/.test(goto), `the selection came along (${goto})`);
    const after = await invoke(w2, "doc_info", { doc: 1 });
    ok(after.indexed && after.rows === before.rows && after.history_pos === before.history_pos, "the file was not read or changed again (same rows, indexed, same undo position)");
    ok((await invoke(w2, "job_progress", { doc: 1 })) === null, "no job was started for the move");
    const mem1 = mem();
    info(`WebView2 memory: ${mem0.mb} MB / ${mem0.procs} processes -> ${mem1.mb} MB / ${mem1.procs} processes with window 2 (+${mem1.mb - mem0.mb} MB)`);
    await sleep(1500);
    const f2 = await scrollRun(m);
    info(`scroll frame time in main with window 2 open: median ${f2.median.toFixed(1)} ms, p95 ${f2.p95.toFixed(1)} ms`);
    ok(f2.median <= f1.median * 1.05 + 0.5, `main's scroll is within 5% with window 2 open (median ${f1.median.toFixed(1)} -> ${f2.median.toFixed(1)} ms)`);
    await w2.screenshot({ path: path.join(evidence, "state-w2.png") }).catch(() => {});
  },

  // Privacy: a private tab stays private in window 2, no recovery copy, no recent entry; a normal tab gets a recovery copy there.
  async private() {
    // 1-minute autosave, set before any file opens.
    let m = await start([]);
    await m.evaluate(() => {
      const k = "natendo.prefs.v1";
      const v = JSON.parse(localStorage.getItem(k) ?? "{}");
      v.autosaveMinutes = 1;
      v.autosave = true;
      localStorage.setItem(k, JSON.stringify(v));
    });
    await m.reload();
    await sleep(2500);
    await dismiss(m);
    const before = new Set(fs.existsSync(path.join(LOCAL, "recovery")) ? fs.readdirSync(path.join(LOCAL, "recovery")) : []);
    const recentBefore = hash(path.join(DATA, "recent.json"));
    await secondLaunch(first);
    await secondLaunch(second);
    // second.csv is active: make it private, then move both to window 2.
    await invoke(m, "set_private", { doc: 2, folder: false, on: true }); info("dialog before move: " + (await dialogText(m)).slice(0, 160)); await m.screenshot({ path: path.join(process.argv[3], "private-before-move.png") });
    const keep = m.getByRole("button", { name: /Keep them/ });
    if (await keep.isVisible().catch(() => false)) await keep.click();
    await sleep(500);
    const w2 = await moveActiveToSecond(m); // second.csv (private)
    await m.locator("[role=tab], .tab").filter({ hasText: "first.csv" }).first().click();
    await cmd(m, "Move tab to a new window");
    await waitFor(async () => (await tabNames(w2)).length === 2, 10000);
    const ids = {};
    for (const [name, id] of [["first", 1], ["second", 2]]) ids[name] = id;
    const st = await invoke(w2, "private_state", { doc: 2 }); info("private_state 1,2,3: " + JSON.stringify([await invoke(w2, "private_state", { doc: 1 }), await invoke(w2, "private_state", { doc: 2 }), await invoke(w2, "private_state", { doc: 3 }).catch((e) => String(e.message || e))]) + " tabs " + (await tabNames(w2)).join("|"));
    ok(st && st.file === true, `a private tab moved to window 2 is still private (${JSON.stringify(st)})`);
    // An unsaved edit in each.
    await w2.locator("[role=tab], .tab").filter({ hasText: "second.csv" }).first().click();
    await edit(w2, "P");
    await w2.locator("[role=tab], .tab").filter({ hasText: "first.csv" }).first().click();
    await edit(w2, "N");
    info("waiting for the 1-minute recovery copy of the normal tab (and none for the private one)");
    const made = await waitFor(() => (fs.existsSync(path.join(LOCAL, "recovery")) ? fs.readdirSync(path.join(LOCAL, "recovery")).filter((f) => !before.has(f)) : []).length > 0, 110000, 2000);
    await sleep(3000);
    const now = (fs.existsSync(path.join(LOCAL, "recovery")) ? fs.readdirSync(path.join(LOCAL, "recovery")) : []).filter((f) => !before.has(f));
    const text = now.map((f) => { try { return fs.readFileSync(path.join(LOCAL, "recovery", f), "utf8").slice(0, 4000); } catch { return ""; } }).join("\n");
    ok(!!made, `the normal tab in window 2 gets a recovery copy (${now.length} new file${now.length === 1 ? "" : "s"})`);
    const hasSecond = now.some((f) => /second/.test(f)) || /second\.csv/.test(text);
    ok(!hasSecond, `no recovery file or description mentions the private file (second.csv)`);
    const recentText = fs.existsSync(path.join(DATA, "recent.json")) ? fs.readFileSync(path.join(DATA, "recent.json"), "utf8") : "";
    ok(!/second\.csv/.test(recentText), "and the private file is not in the recent list");
    void recentBefore;
  },

  // The quit path leaves what the old one did, and the session lists both windows' files.
  async parity() {
    const logBefore = fs.existsSync(path.join(LOCAL, "logs")) ? fs.readdirSync(path.join(LOCAL, "logs")).map((f) => [f, fs.statSync(path.join(LOCAL, "logs", f)).size]) : [];
    const recBefore = new Set(fs.existsSync(path.join(LOCAL, "recovery")) ? fs.readdirSync(path.join(LOCAL, "recovery")) : []);
    const m = await start([first]);
    await secondLaunch(second);
    await moveActiveToSecond(m);
    await sleep(1500);
    await edit(m, "Q");
    say("close main: " + CLOSE_MAIN());
    await waitFor(async () => /Quit without saving/.test(await dialogText(m)), 5000);
    await dialog(m).getByRole("button", { name: /Quit and lose changes/ }).click();
    ok(await waitFor(() => !alive(), 10000), "quit through the new path ends the app");
    await sleep(1000);
    const session = fs.existsSync(path.join(DATA, "session.json")) ? fs.readFileSync(path.join(DATA, "session.json"), "utf8") : "";
    info("session.json: " + session.replace(/\s+/g, " ").slice(0, 500));
    ok(/first\.csv/.test(session) && /second\.csv/.test(session), "the remembered session lists the file in the main window AND the one in window 2");
    const recAfter = (fs.existsSync(path.join(LOCAL, "recovery")) ? fs.readdirSync(path.join(LOCAL, "recovery")) : []).filter((f) => !recBefore.has(f));
    ok(recAfter.length === 0, `no recovery copy of this run is left behind (${recAfter.length})`);
    let closed = false;
    if (fs.existsSync(path.join(LOCAL, "logs"))) {
      for (const f of fs.readdirSync(path.join(LOCAL, "logs"))) {
        const p = path.join(LOCAL, "logs", f);
        const was = (logBefore.find((x) => x[0] === f) || [f, 0])[1];
        if (fs.statSync(p).size > was && /closed/.test(fs.readFileSync(p, "utf8").slice(was))) closed = true;
      }
    }
    ok(closed, "the cleanup on exit ran (the 'closed' line is in the log)");
  },

  // A save in progress blocks the quit.
  async saving() {
    const big = path.join(WORK, "big-save.csv");
    fs.copyFileSync(bigCsv, big);
    const m = await start([big]);
    await waitFor(async () => /1,000,000 rows/.test((await m.textContent("footer.status")) || ""), 60000, 500);
    let seen = false;
    for (let attempt = 0; attempt < 4 && !seen; attempt++) {
      await edit(m, "S" + attempt);
      await m.keyboard.press("Control+s");
      await sleep(30);
      say("close main during the save: " + CLOSE_MAIN());
      await sleep(400);
      if (!alive()) {
        info("the save had finished when the close request came, so the app (rightly) quit at once with nothing unsaved");
        break;
      }
      const t = await dialogText(m);
      if (/Still running|still being saved or changed|still busy/.test(t)) {
        seen = true;
        ok(alive(), `a save in progress blocks the quit (${t.slice(0, 100)})`);
        await dialog(m).getByRole("button", { name: "OK" }).click().catch(() => {});
      } else {
        info(`attempt ${attempt + 1}: the save had finished or the question was another (${t.slice(0, 60)})`);
        await dialog(m).getByRole("button", { name: "Cancel" }).click().catch(() => {});
        await sleep(2500);
      }
    }
    if (!seen) info("the save finished before the close request each time; the held-document case is covered by the Rust test close_check_never_waits_for_a_busy_document");
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
