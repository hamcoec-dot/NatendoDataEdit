// Drives the REAL built app over made-up GIS files: opening a .shp shows the import screen of the .dbf beside
// it (encoding and rule shown, FID column, a change of encoding previewed), the import is read back, and a
// GeoPackage in write-ahead mode and one in rollback mode open with the table list, without their shapes and
// without any file appearing or changing beside them. A GeoPackage with unsaved changes is refused.
//
// Usage: NODE_PATH=<folder with playwright> node scripts/real-gis.cjs <exe> <evidence folder>
// Needs Node 22.13 or newer (node:sqlite). The app's own data is backed up and put back.
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
const PORT = 9401;
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

// ---- made-up GIS files ----
const { DatabaseSync } = require("node:sqlite");
const GIS = path.join(WORK, `gis-${Date.now()}`);
fs.mkdirSync(GIS, { recursive: true });
const g = (n) => path.join(GIS, n);
/** A dBase III table: fields [name, type, length], records of byte strings. */
const dbf = (fields, records, language) => {
  const start = 32 + 32 * fields.length + 1;
  const len = 1 + fields.reduce((a, f) => a + f[2], 0);
  const head = Buffer.alloc(32);
  head[0] = 3;
  head.writeUInt32LE(records.length, 4);
  head.writeUInt16LE(start, 8);
  head.writeUInt16LE(len, 10);
  head[29] = language;
  const parts = [head];
  for (const [name, type, n] of fields) {
    const f = Buffer.alloc(32);
    f.write(name);
    f.write(type, 11);
    f[16] = n;
    parts.push(f);
  }
  parts.push(Buffer.from([13]));
  for (const [mark, values] of records) {
    parts.push(Buffer.from(mark));
    values.forEach((v, i) => {
      const cell = Buffer.alloc(fields[i][2], " ");
      Buffer.from(v, "latin1").copy(cell);
      parts.push(cell);
    });
  }
  parts.push(Buffer.from([26]));
  return Buffer.concat(parts);
};
fs.writeFileSync(
  g("sample roads.dbf"),
  dbf(
    [["NAME", "C", 24], ["CODE", "N", 8], ["LENGTH", "N", 10], ["CHECKED", "D", 8]],
    [
      [" ", ["Made Up Avenue", "00123", "1.50", "20240301"]],
      ["*", ["Removed Lane", "00124", "2.00", "20240302"]],
      [" ", ["Caf\u00e9 Example Road", "00125", "12.25", "20240303"]],
    ],
    0x57,
  ),
);
fs.writeFileSync(g("sample roads.shp"), Buffer.alloc(100));
fs.writeFileSync(g("sample roads.prj"), 'GEOGCS["Made up datum"]');
const pack = (name, wal) => {
  const db = new DatabaseSync(g(name));
  if (wal) db.exec("PRAGMA journal_mode = WAL");
  db.exec(`PRAGMA application_id = 1196444487;
    CREATE TABLE gpkg_contents(table_name TEXT PRIMARY KEY, data_type TEXT);
    INSERT INTO gpkg_contents VALUES('parcels', 'features'), ('notes', 'attributes');
    CREATE TABLE gpkg_geometry_columns(table_name TEXT, column_name TEXT, geometry_type_name TEXT, srs_id INTEGER);
    INSERT INTO gpkg_geometry_columns VALUES('parcels', 'geom', 'POLYGON', 4326);
    CREATE TABLE parcels(fid INTEGER PRIMARY KEY, geom BLOB, owner TEXT, lot TEXT);
    INSERT INTO parcels VALUES(1, zeroblob(2048), 'Sample Owner', '007'), (2, zeroblob(2048), 'Another Sample', '008');
    CREATE TABLE notes(note TEXT);`);
  db.close();
};
pack("rollback parcels.gpkg", false);
pack("wal parcels.gpkg", true);
pack("busy parcels.gpkg", true);
const listing = () => fs.readdirSync(GIS).sort().map((n) => {
  const s = fs.statSync(g(n));
  return `${n} ${s.size} ${hash(g(n))}`;
});
const text = async (m, sel) => ((await m.locator(sel).first().textContent({ timeout: 3000 }).catch(() => "")) || "").replace(/\s+/g, " ").trim();
const table = async (m, doc, rows, cols) => {
  const out = [];
  for (let r = 0; r < rows; r++) {
    const row = [];
    for (let c = 0; c < cols; c++) row.push(await cell(m, doc, r, c));
    out.push(row);
  }
  return out;
};
const shot = (m, n) => m.screenshot({ path: path.join(evidence, n) });

(async () => {
  backup();
  // Screenshots must show made-up files only: the app starts with no recent files and no session (the
  // person's own lists were copied just above and are put back at the end).
  const clean = () => {
    for (const f of ["session.json", "recent.json"]) {
      const mine = path.join(DATA, f);
      if (fs.existsSync(mine) && hash(path.join(BACKUP, f)) !== null) fs.rmSync(mine);
    }
  };
  clean();
  // A second connection holds unsaved changes in the busy file's -wal.
  const writer = new DatabaseSync(g("busy parcels.gpkg"));
  writer.exec("PRAGMA wal_autocheckpoint = 0; INSERT INTO notes VALUES('unsaved')");
  try {
    const before = listing();
    fs.writeFileSync(path.join(evidence, "folder-before.txt"), before.join("\n"));
    // 1. A .shp opens the import screen of its .dbf.
    let m = await start([g("sample roads.shp")]);
    await waitFor(() => m.getByTestId("import-notes").isVisible(), 20000);
    const why = await text(m, "[data-testid=import-why]");
    ok(why.includes("sample roads.dbf") && why.includes("shapes are not shown"), `why it opened: ${why}`);
    let notes = await text(m, "[data-testid=import-notes]");
    ok(notes.includes("windows-1252") && notes.includes("the code page stored in the table") && notes.includes("Made up datum"), `notes: ${notes}`);
    await shot(m, "import-dbf.png");
    const preview = () => text(m, ".table-preview");
    ok((await preview()).includes("Caf\u00e9 Example Road"), "accented text in the preview");
    await m.getByLabel(/Text encoding/).selectOption("cp437");
    ok(await waitFor(async () => (await text(m, "[data-testid=import-notes]")).includes("CP437 (your choice)"), 8000), "the picked encoding is named");
    ok(await waitFor(async () => !(await preview()).includes("Caf\u00e9"), 8000), "the preview was read again");
    await m.getByLabel(/Text encoding/).selectOption("");
    await waitFor(async () => (await preview()).includes("Caf\u00e9"), 8000);
    await m.getByRole("button", { name: "Import", exact: true }).click();
    ok(await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed, 20000), "imported");
    const got = await table(m, 1, 3, 5);
    const want = [["FID", "NAME", "CODE", "LENGTH", "CHECKED"], ["0", "Made Up Avenue", "00123", "1.50", "2024-03-01"], ["2", "Caf\u00e9 Example Road", "00125", "12.25", "2024-03-03"]];
    ok(JSON.stringify(got) === JSON.stringify(want), `table ${JSON.stringify(got)}`);
    await shot(m, "dbf-tab.png");
    await killApp();
    clean();
    // 2. GeoPackages, rollback and write-ahead.
    for (const name of ["rollback parcels.gpkg", "wal parcels.gpkg"]) {
      const kind = name.split(" ")[0];
      m = await start([g(name)]);
      const dlg = m.getByRole("button", { name: "Open", exact: true });
      ok(await waitFor(() => dlg.isVisible(), 20000), `${kind}: the table list opened`);
      const options = await m.locator("select option").allTextContents();
      ok(JSON.stringify(options) === JSON.stringify(["notes (attributes, 0 rows)", "parcels (features, 2 rows)"]), `${kind}: tables ${JSON.stringify(options)}`);
      await m.locator("select").first().selectOption("parcels");
      await shot(m, `gpkg-${kind}-list.png`);
      await dlg.click();
      const doc = await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed, 20000);
      ok(!!doc, `${kind}: the table opened`);
      const rows = await table(m, 1, 3, 3);
      ok(JSON.stringify(rows) === JSON.stringify([["fid", "owner", "lot"], ["1", "Sample Owner", "007"], ["2", "Another Sample", "008"]]), `${kind}: ${JSON.stringify(rows)}`);
      const said = await msg(m);
      ok(said.includes("geom (POLYGON") && said.includes("not shown"), `${kind}: ${said}`);
      await shot(m, `gpkg-${kind}-tab.png`);
    }
    // 3. Unsaved changes in the -wal: refused, plainly.
    m = await start([g("busy parcels.gpkg")]);
    const refused = await waitFor(async () => {
      const t = ((await m.locator("body").textContent()) || "").replace(/\s+/g, " ");
      return t.includes("unsaved changes in its -wal file") && t;
    }, 20000);
    ok(!!refused, "busy: refused with the reason");
    await shot(m, "gpkg-busy.png");
    await killApp();
    const after = listing();
    fs.writeFileSync(path.join(evidence, "folder-after.txt"), after.join("\n"));
    ok(JSON.stringify(after) === JSON.stringify(before), `the folder is unchanged (${before.length} files, names, sizes and contents)`);
  } catch (e) {
    ok(false, String(e && e.stack ? e.stack : e));
  } finally {
    await killApp();
    writer.close();
    restore();
    say(failed ? `${failed} FAILED` : "all passed");
    process.exit(failed ? 1 : 0);
  }
})();
