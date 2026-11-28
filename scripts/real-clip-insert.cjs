// Real-app check of the clipboard table and Insert copied rows (Ctrl+Shift++), driven over CDP (no SendKeys).
// Reads the clipboard formats back from Windows, pastes into Excel through COM (HTML and text-only, side by side),
// checks a private tab puts no table, inserts copied rows with Ctrl+Shift++, saves and compares bytes, one Ctrl+Z.
// Times Copy of 20,000 and 1,000,000 cells. Made-up data only.
// Run: node scripts/real-clip-insert.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence, mode] = process.argv.slice(2);
/** "times": only the Copy times (to run the same steps against main). */
const TIMES_ONLY = mode === "times";
if (!exe || !evidence) {
  console.log(
    "usage: node scripts/real-clip-insert.cjs <exe> <evidence folder>",
  );
  process.exit(2);
}
const WORK = path.join(evidence, "work");
fs.mkdirSync(WORK, { recursive: true });
const DATA = path.join(process.env.APPDATA, "com.natendo.dataedit");
const LOCAL = path.join(process.env.LOCALAPPDATA, "com.natendo.dataedit");
const PORT = 9389;
const T0 = Date.now();
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let failed = 0;
const say = (m) =>
  console.log(`[${((Date.now() - T0) / 1000).toFixed(1)}s] ${m}`);
const ok = (cond, msg) => {
  if (!cond) failed++;
  say(`${cond ? "PASS" : "FAIL"} ${msg}`);
};
const info = (m) => say(`INFO ${m}`);
const hash = (p) =>
  fs.existsSync(p)
    ? crypto.createHash("sha256").update(fs.readFileSync(p)).digest("hex")
    : null;

// ---- the app's own data ----
const BACKUP = path.join(WORK, "backup");
const backup = () => {
  fs.mkdirSync(BACKUP, { recursive: true });
  for (const f of ["session.json", "recent.json"])
    if (fs.existsSync(path.join(DATA, f)))
      fs.copyFileSync(path.join(DATA, f), path.join(BACKUP, f));
  fs.rmSync(path.join(BACKUP, "no-private-json"), { force: true });
  if (!fs.existsSync(path.join(DATA, "private.json")))
    fs.writeFileSync(path.join(BACKUP, "no-private-json"), "");
  else
    fs.copyFileSync(
      path.join(DATA, "private.json"),
      path.join(BACKUP, "private.json"),
    );
  const rec = path.join(LOCAL, "recovery");
  const aside = path.join(BACKUP, "recovery");
  if (fs.existsSync(rec) && !fs.existsSync(aside)) fs.renameSync(rec, aside);
};
const wipeRecovery = () => {
  const rec = path.join(LOCAL, "recovery");
  if (fs.existsSync(rec))
    fs.renameSync(rec, path.join(BACKUP, `recovery-run-${Date.now()}`));
};
const restore = () => {
  const pj = path.join(DATA, "private.json");
  if (fs.existsSync(path.join(BACKUP, "no-private-json"))) {
    if (fs.existsSync(pj))
      fs.renameSync(
        pj,
        path.join(BACKUP, `private.from-run-${Date.now()}.json`),
      );
  } else if (fs.existsSync(path.join(BACKUP, "private.json")))
    fs.copyFileSync(path.join(BACKUP, "private.json"), pj);
  wipeRecovery();
  const aside = path.join(BACKUP, "recovery");
  if (fs.existsSync(aside)) fs.renameSync(aside, path.join(LOCAL, "recovery"));
  for (const f of ["session.json", "recent.json"]) {
    const b = path.join(BACKUP, f);
    if (fs.existsSync(b)) fs.copyFileSync(b, path.join(DATA, f));
  }
  const same = ["session.json", "recent.json"].every(
    (f) => hash(path.join(BACKUP, f)) === hash(path.join(DATA, f)),
  );
  say(`the app's session.json and recent.json restored byte for byte: ${same}`);
};

// ---- the app (only the instance this script started is ever stopped) ----
let browser = null;
let ctx = null;
let appPid = null;
const running = () =>
  spawnSync(
    "powershell",
    [
      "-NoProfile",
      "-Command",
      "(Get-Process -Name natendo-data-edit -ErrorAction SilentlyContinue | Measure-Object).Count",
    ],
    { encoding: "utf8" },
  ).stdout.trim() !== "0";
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
  if (appPid !== null && alive())
    spawnSync("taskkill", ["/PID", String(appPid), "/T", "/F"], {
      encoding: "utf8",
    });
  for (let i = 0; i < 20 && alive(); i++) await sleep(250);
  appPid = null;
};
const dismiss = async (p) => {
  const notNow = p.getByRole("button", { name: "Not now" });
  if (await notNow.isVisible().catch(() => false)) await notNow.click();
  await sleep(300);
  await p
    .locator(".header-ask button.close")
    .click({ timeout: 800 })
    .catch(() => {});
};
const start = async (files) => {
  await killApp();
  wipeRecovery();
  if (running())
    throw new Error(
      "another Natendo Data Edit is running: close it first (this test never stops an app it did not start)",
    );
  const child = spawn(exe, files, {
    env: {
      ...process.env,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT}`,
      WEBVIEW2_USER_DATA_FOLDER: path.join(WORK, `wv2-${Date.now()}`),
    },
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
  const m = ctx.pages().find((p) => !p.url().includes("window=2"));
  await m.waitForSelector("canvas, .start, button", { timeout: 30000 });
  await sleep(1500);
  await dismiss(m);
  return m;
};
const invoke = (page, cmd, args) =>
  page.evaluate(
    ([c, a]) =>
      window.__TAURI_INTERNALS__.invoke(c, a).catch((e) => {
        throw new Error(typeof e === "string" ? e : JSON.stringify(e));
      }),
    [cmd, args || {}],
  );
const cmd = async (page, q) => {
  await page.keyboard.press("Alt+q");
  await page.keyboard.type(q);
  await sleep(350);
  await page.keyboard.press("Enter");
  await sleep(900);
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

const msg = async (m) =>
  (
    (await m
      .locator("footer.status .msg")
      .textContent({ timeout: 1000 })
      .catch(() => "")) || ""
  )
    .replace(/\s+/g, " ")
    .trim();
const shot = (m, n) => m.screenshot({ path: path.join(evidence, n) });
const clean = () => {
  for (const f of ["session.json", "recent.json"]) {
    const mine = path.join(DATA, f);
    if (fs.existsSync(mine) && hash(path.join(BACKUP, f)) !== null)
      fs.rmSync(mine);
  }
};
const key = async (m, k) => {
  const tb = k === "Control+z" ? "Undo" : null;
  if (tb)
    await m.locator(`.toolbar button[title^="${tb}"]`).click({ timeout: 3000 });
  else await m.keyboard.press(k);
  await sleep(700);
};
const goto = async (m, text) => {
  await m.keyboard.press("Control+g");
  await sleep(150);
  await m.keyboard.type(text);
  await m.keyboard.press("Enter");
  await sleep(500);
};
const tabTo = async (m, i) => {
  await m.locator(`[data-tab-index="${i}"]`).click();
  await sleep(600);
};

// ---- Windows' clipboard, read back (PowerShell in a single-threaded apartment, as the clipboard needs) ----
const PS_CLIP = `
Add-Type -AssemblyName System.Windows.Forms
$o = [System.Windows.Forms.Clipboard]::GetDataObject()
$f = @($o.GetFormats())
$h = $null
if ($f -contains 'HTML Format') { $s = $o.GetData('HTML Format'); if ($s -is [System.IO.MemoryStream]) { $h = [Text.Encoding]::UTF8.GetString($s.ToArray()) } else { $h = [string]$s } }
$flag = { param($n) if ($f -contains $n) { $d = $o.GetData($n); if ($d -is [System.IO.MemoryStream]) { ($d.ToArray() -join ',') } else { [string]$d } } else { $null } }
[pscustomobject]@{ formats = $f; html = $h; text = [System.Windows.Forms.Clipboard]::GetText();
  history = (& $flag 'CanIncludeInClipboardHistory'); cloud = (& $flag 'CanUploadToCloudClipboard'); monitor = (& $flag 'ExcludeClipboardContentFromMonitorProcessing') } | ConvertTo-Json -Compress -Depth 3
`;
const readClip = () => {
  const r = spawnSync(
    "powershell",
    ["-NoProfile", "-STA", "-Command", PS_CLIP],
    { encoding: "utf8", timeout: 30000 },
  );
  try {
    return JSON.parse(r.stdout.trim());
  } catch {
    return { formats: [], error: (r.stderr || r.stdout).slice(0, 300) };
  }
};

// ---- Excel through COM: the clipboard pasted as Excel picks it (the table), and as text only ----
const PS_EXCEL = (rows, cols) => `
$xl = New-Object -ComObject Excel.Application
$xl.Visible = $false; $xl.DisplayAlerts = $false
try {
  $wb = $xl.Workbooks.Add()
  $read = { param($ws) $out = @(); for ($r = 1; $r -le ${rows}; $r++) { for ($c = 1; $c -le ${cols}; $c++) { $x = $ws.Cells.Item($r, $c); $out += [pscustomobject]@{ r = $r; c = $c; text = $x.Text; value = [string]$x.Value2; formula = [bool]$x.HasFormula; type = $(if ($x.Value2 -eq $null) { 'empty' } else { $x.Value2.GetType().Name }) } } }; $out }
  $a = $wb.Worksheets.Item(1); $a.Paste($a.Range('A1'))
  $b = $wb.Worksheets.Add(); $b.PasteSpecial('Unicode Text')
  [pscustomobject]@{ table = @(& $read $a); text = @(& $read $b) } | ConvertTo-Json -Compress -Depth 4
  $wb.Close($false)
} finally { $xl.Quit(); [void][Runtime.InteropServices.Marshal]::ReleaseComObject($xl) }
`;
const excelPaste = (rows, cols) => {
  const r = spawnSync(
    "powershell",
    ["-NoProfile", "-STA", "-Command", PS_EXCEL(rows, cols)],
    { encoding: "utf8", timeout: 120000 },
  );
  try {
    return JSON.parse(r.stdout.trim());
  } catch {
    return { error: (r.stderr || r.stdout).slice(0, 400) };
  }
};

const write = (name, text) => {
  const f = path.join(WORK, name);
  fs.writeFileSync(f, text);
  return f;
};
const PEOPLE =
  "id,city,zip,paid,acct\r\n1,Ames <b>&,01234,1234.5,9876543210987654\r\n2,=1+1,1-2,20,0012\r\n3,Cody,82414,7,1e5\r\n4,Dell,50010,8,+7\r\n";

(async () => {
  while (running()) {
    say("waiting for the running app to close");
    await sleep(5000);
  }
  backup();
  clean();
  try {
    const people = write("people.csv", PEOPLE);
    const secret = write(
      "secret.csv",
      "name,ssn\r\nAnn,000-00-0000\r\nBo,000-00-0001\r\n",
    );
    let big = "a,b,c,d,e,f,g,h,i,j\r\n";
    for (let i = 0; i < 100_000; i++)
      big += `${i},x${i},${i * 2},y,z,${i % 7},q,r,s,t\r\n`;
    const bigf = write("big.csv", big);
    let mid = "a,b,c,d,e,f,g,h,i,j\r\n";
    for (let i = 0; i < 2_000; i++)
      mid += `${i},x${i},${i * 2},y,z,${i % 7},q,r,s,t\r\n`;
    const midf = write("mid.csv", mid);
    const m = await start([people, secret, midf, bigf]);
    await tabTo(m, 0);
    if (!TIMES_ONLY) {
      // ---- Copy two whole rows: text, table, flags ----
      await goto(m, "R2 C1");
      await m.keyboard.press("Shift+Space");
      await m.keyboard.press("Shift+ArrowDown");
      await sleep(300);
      await m.keyboard.press("Control+c");
      await sleep(1200);
      const c1 = readClip();
      info(
        `formats after Copy: ${JSON.stringify(c1.formats)}; history ${c1.history}; cloud ${c1.cloud}; monitor ${c1.monitor}`,
      );
      ok(
        c1.text ===
          "1\tAmes <b>&\t01234\t1234.5\t9876543210987654\r\n2\t=1+1\t1-2\t20\t0012\r\n",
        `the text is as before (${JSON.stringify(c1.text)})`,
      );
      ok(
        (c1.formats || []).includes("HTML Format") &&
          /<meta charset="utf-8">/.test(c1.html || ""),
        "the table is on the clipboard as CF_HTML, with its character set",
      );
      ok(
        /Ames &lt;b&gt;&amp;/.test(c1.html || "") && !/<b>/.test(c1.html || ""),
        "values escaped in the table",
      );
      fs.writeFileSync(path.join(evidence, "cf_html.txt"), c1.html || "");

      // ---- Excel: the table pasted keeps the values; side by side with a text-only paste ----
      const xl = excelPaste(2, 5);
      if (xl.error) ok(false, `Excel: ${xl.error}`);
      else {
        fs.writeFileSync(
          path.join(evidence, "excel-paste.json"),
          JSON.stringify(xl, null, 1),
        );
        const t = (k, r, c) => xl[k].find((x) => x.r === r && x.c === c);
        const want = [
          ["1", "Ames <b>&", "01234", "1234.5", "9876543210987654"],
          ["2", "=1+1", "1-2", "20", "0012"],
        ];
        const kept = want.every((row, r) =>
          row.every(
            (v, c) =>
              t("table", r + 1, c + 1).text.replace(/,/g, "") === v ||
              t("table", r + 1, c + 1).value === v,
          ),
        );
        ok(
          kept,
          `Excel keeps every value of the table as copied (${xl.table.map((x) => x.text).join(" | ")})`,
        );
        ok(
          !t("table", 2, 2).formula && t("table", 2, 2).value === "=1+1",
          "=1+1 arrives as text, not a formula",
        );
        ok(
          t("table", 1, 1).type === "Double" &&
            t("table", 1, 4).type === "Double",
          "plain numbers stay numbers",
        );
        info(
          `text-only paste for comparison: ${xl.text.map((x) => `${x.text}${x.formula ? "(formula)" : ""}`).join(" | ")}`,
        );
      }

      // ---- Ctrl+Shift++: the copied rows go in above row 5 (data row 4), one undo step; bytes saved ----
      await goto(m, "R5 C1");
      await m.keyboard.press("Shift+Space");
      await sleep(200);
      await m.keyboard.press("Control+Shift+Equal");
      await sleep(1200);
      const s1 = await msg(m);
      ok(/Inserted 2 copied rows above row 5/.test(s1), `status: ${s1}`);
      await shot(m, "inserted.png");
      await m.keyboard.press("Control+s");
      await sleep(1500);
      const after = fs.readFileSync(people, "utf8");
      const expect =
        "id,city,zip,paid,acct\r\n1,Ames <b>&,01234,1234.5,9876543210987654\r\n2,=1+1,1-2,20,0012\r\n3,Cody,82414,7,1e5\r\n1,Ames <b>&,01234,1234.5,9876543210987654\r\n2,=1+1,1-2,20,0012\r\n4,Dell,50010,8,+7\r\n";
      ok(after === expect, `saved bytes: ${JSON.stringify(after)}`);
      await key(m, "Control+z");
      await m.keyboard.press("Control+s");
      await sleep(1500);
      ok(
        fs.readFileSync(people, "utf8") === PEOPLE,
        "one Ctrl+Z, saved: the file is byte for byte as it was",
      );

      // ---- a private tab: no table on the clipboard ----
      await tabTo(m, 1);
      await cmd(m, "Private file mode (this file)");
      await sleep(800);
      await goto(m, "R2 C1");
      await m.keyboard.press("Shift+Space");
      await m.keyboard.press("Control+c");
      await sleep(1200);
      const c2 = readClip();
      info(
        `formats after a private copy: ${JSON.stringify(c2.formats)}; history ${c2.history}; cloud ${c2.cloud}; monitor ${c2.monitor}`,
      );
      ok(
        !(c2.formats || []).includes("HTML Format") &&
          /000-00-0000/.test(c2.text || ""),
        "a private tab: only the text, no table",
      );
      await cmd(m, "Private file mode (this file)");
      await sleep(800);
    }

    // ---- times ----
    const timeCopy = async (i, label) => {
      await tabTo(m, i);
      await goto(m, "R2 C1");
      await m.keyboard.press("Control+Shift+End");
      await sleep(400);
      await m.evaluate(() => navigator.clipboard.writeText("x"));
      const t0 = Date.now();
      await m.keyboard.press("Control+c");
      const done = await waitFor(
        async () => /^Copied /.test(await msg(m)),
        60000,
        20,
      );
      const ms = Date.now() - t0;
      const q = await m
        .locator(".dialog[role=alertdialog]")
        .isVisible()
        .catch(() => false);
      info(
        `${label}: ${ms} ms ${done ? "" : "(no Copied message)"}${q ? " (a question was asked)" : ""}; ${await msg(m)}`,
      );
      return ms;
    };
    const mids = [];
    for (let k = 0; k < 3; k++)
      mids.push(await timeCopy(2, "Copy 20,000 cells (with the table)"));
    const c3 = readClip();
    if (!TIMES_ONLY)
      ok(
        (c3.formats || []).includes("HTML Format"),
        "20,000 cells: the table is there",
      );
    const bigs = [];
    for (let k = 0; k < 3; k++)
      bigs.push(await timeCopy(3, "Copy 1,000,000 cells (text only)"));
    const c4 = readClip();
    if (!TIMES_ONLY)
      ok(
        !(c4.formats || []).includes("HTML Format"),
        "1,000,000 cells: text only",
      );
    info(`TIMES mid ${mids.join(",")} big ${bigs.join(",")}`);
  } catch (e) {
    ok(false, String(e && e.stack ? e.stack : e));
  } finally {
    await killApp();
    restore();
    say(failed ? `${failed} FAILED` : "all passed");
    process.exit(failed ? 1 : 0);
  }
})();
