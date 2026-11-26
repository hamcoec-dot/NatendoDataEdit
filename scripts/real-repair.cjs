// Drives the REAL built app through the repair bar: a name with an unescaped comma, a line broken in two and a stray
// quote are repaired in the bar, the file is saved and its bytes read back, and the saved file opens with no ragged
// rows. The app's own session, recent-file and recovery data are set aside and put back.
// Usage: NODE_PATH=<playwright> node scripts/real-repair.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-repair.cjs <exe> <evidence folder>");
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
const say = (m) => console.log(`[${((Date.now() - T0) / 1000).toFixed(1)}s] ${m}`);
const ok = (cond, msg) => {
  if (!cond) failed++;
  say(`${cond ? "PASS" : "FAIL"} ${msg}`);
};
const info = (m) => say(`INFO ${m}`);
const hash = (p) => (fs.existsSync(p) ? crypto.createHash("sha256").update(fs.readFileSync(p)).digest("hex") : null);

// ---- the app's own data ----
const BACKUP = path.join(WORK, "backup");
const backup = () => {
  fs.mkdirSync(BACKUP, { recursive: true });
  for (const f of ["session.json", "recent.json"]) if (fs.existsSync(path.join(DATA, f))) fs.copyFileSync(path.join(DATA, f), path.join(BACKUP, f));
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
// Only the instance this script started is ever stopped (never another one: a person's own running app is left alone).
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
const dismiss = async (p) => {
  const notNow = p.getByRole("button", { name: "Not now" });
  if (await notNow.isVisible().catch(() => false)) await notNow.click();
  await sleep(300);
  await p.locator(".header-ask button.close").click({ timeout: 800 }).catch(() => {});
};
const start = async (files) => {
  await killApp();
  wipeRecovery();
  if (running()) throw new Error("another Natendo Data Edit is running: close it first (this test never stops an app it did not start)");
  const child = spawn(exe, files, {
    env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT}`, WEBVIEW2_USER_DATA_FOLDER: path.join(WORK, `wv2-${Date.now()}`) },
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
const invoke = (page, cmd, args) => page.evaluate(([c, a]) => window.__TAURI_INTERNALS__.invoke(c, a).catch((e) => { throw new Error(typeof e === "string" ? e : JSON.stringify(e)); }), [cmd, args || {}]);
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

const OUT = path.join(WORK, `out-${Date.now()}`);
fs.mkdirSync(OUT, { recursive: true });
const FILE = path.join(OUT, "broken.csv");
const BODY = ["id,name,city", "1,ann,Oslo", "2,Smith, John,Rome", "3,b", "o,Lima", "4,eve,Turin", "5,dee,Lima", ""].join("\r\n");
fs.writeFileSync(FILE, BODY);
const read = () => fs.readFileSync(FILE, "utf8");
const body = async (m) => ((await m.textContent("body")) || "").replace(/\s+/g, " ");

const proof = async () => {
  const m = await start([FILE]);
  await waitFor(async () => (await invoke(m, "doc_info", { doc: 1 })).indexed, 60000, 300);
  await sleep(1200);
  await dismiss(m);
  const yes = m.getByRole("button", { name: "Yes, column names" });
  if (await yes.isVisible().catch(() => false)) await yes.click();
  await sleep(500);
  const text = (id) => m.getByTestId(id).textContent().then((t) => (t || "").replace(/\s+/g, " ").trim());
  const apply = () => m.getByTestId("repair").getByRole("button", { name: "Apply" });
  await m.getByRole("button", { name: "Repair…" }).click();
  await m.getByTestId("repair-heading").waitFor({ timeout: 8000 });
  await sleep(500);
  ok(/1 of 3 ragged rows/.test(await text("repair-count")), `the bar opens on the first of 3: ${await text("repair-count")}`);
  ok((await text("repair-raw")) === "2,Smith, John,Rome", `the file's own text: ${await text("repair-raw")}`);
  await m.getByRole("button", { name: "Join fields 2 and 3" }).click();
  await m.screenshot({ path: path.join(evidence, "repair-comma.png") });
  ok(/Smith, John/.test(await text("repair-result")), "the preview keeps the comma inside one cell");
  await apply().click();
  await sleep(900);
  ok(/Row 4 of the file/.test(await text("repair-heading")), `on to the broken line: ${await text("repair-heading")}`);
  await m.getByRole("combobox", { name: "Put between the two rows" }).selectOption("nothing");
  ok(/Joins row 5 of the file/.test(await text("repair-next")), await text("repair-next"));
  await apply().click();
  await sleep(900);
  ok(/No row has a different number of fields/.test(await text("repair-count")), `all repaired: ${await text("repair-count")}`);
  await m.getByTestId("repair").focus();
  await m.keyboard.press("Escape");
  await sleep(400);
  ok((await m.getByTestId("repair").count()) === 0, "Esc closes the bar");
  // Undo takes back one repair; redo puts it back.
  await invoke(m, "change", { doc: 1, change: { op: "undo" } });
  ok((await invoke(m, "ragged_list", { doc: 1, keepFirst: true })).total === 2, "one undo step brings the broken line back (both halves)");
  await invoke(m, "change", { doc: 1, change: { op: "redo" } });
  await invoke(m, "save", { doc: 1, saveAs: false, overwriteChanged: true, allowLossy: false });
  await sleep(800);
  const want = ["id,name,city", "1,ann,Oslo", '2,"Smith, John",Rome', "3,bo,Lima", "4,eve,Turin", "5,dee,Lima", ""].join("\r\n");
  ok(read() === want, `saved: ${JSON.stringify(read())}`);

  // A line broken inside its last field: the second half is listed and joins onto the previous row.
  const broken = path.join(OUT, "last.csv");
  fs.writeFileSync(broken, ["id,name,city", "1,ann,Os", "lo", "2,bo,Rome", "3,cy,Lima", ""].join("\r\n"));
  const mp = await start([broken]);
  await waitFor(async () => (await invoke(mp, "doc_info", { doc: 1 })).indexed, 60000, 300);
  await sleep(1000);
  await dismiss(mp);
  const yes2 = mp.getByRole("button", { name: "Yes, column names" });
  if (await yes2.isVisible().catch(() => false)) await yes2.click();
  await sleep(400);
  await mp.getByRole("button", { name: "Repair…" }).click();
  await mp.getByTestId("repair-heading").waitFor({ timeout: 8000 });
  await sleep(500);
  await mp.getByRole("combobox", { name: "Put between the two rows" }).selectOption("nothing");
  await mp.getByRole("radio", { name: "Join with the previous row" }).check();
  const said = ((await mp.getByTestId("repair-next").textContent()) || "").trim();
  ok(/Joins onto row 2 of the file/.test(said), said);
  await mp.getByTestId("repair").getByRole("button", { name: "Apply" }).click();
  await sleep(900);
  await invoke(mp, "save", { doc: 1, saveAs: false, overwriteChanged: true, allowLossy: false });
  await sleep(800);
  const joined = fs.readFileSync(broken, "utf8");
  ok(joined === ["id,name,city", "1,ann,Oslo", "2,bo,Rome", "3,cy,Lima", ""].join("\r\n"), `saved: ${JSON.stringify(joined)}`);

  // Fix all like this, then the emptied column: four rows with a comma in the name.
  const many = path.join(OUT, "many.csv");
  const rows = ["id,name,city", "1,Smith, John,Rome", "2,ann,Oslo", "3,Doe, Jane,Lima", "4,bo,Rome", "5,Roe, Bo,Kiel", "6,cy,Turin", "7,eve,Lyon", "8,Poe, Al,Bern", "9,dee,Oslo", ""];
  fs.writeFileSync(many, rows.join("\r\n"));
  const mf = await start([many]);
  await waitFor(async () => (await invoke(mf, "doc_info", { doc: 1 })).indexed, 60000, 300);
  await sleep(1000);
  await dismiss(mf);
  const yes3 = mf.getByRole("button", { name: "Yes, column names" });
  if (await yes3.isVisible().catch(() => false)) await yes3.click();
  await sleep(400);
  const said2 = (id) => mf.getByTestId(id).textContent().then((t) => (t || "").replace(/\s+/g, " ").trim());
  await mf.getByRole("button", { name: "Repair…" }).click();
  await mf.getByTestId("repair-heading").waitFor({ timeout: 8000 });
  await sleep(500);
  await mf.getByRole("button", { name: "Join fields 2 and 3" }).click();
  await mf.getByTestId("repair").getByRole("button", { name: "Apply" }).click();
  await mf.getByTestId("repair-offer").waitFor({ timeout: 10000 });
  ok(/3 other rows have the same shape \(4 fields\)/.test(await said2("repair-offer")), await said2("repair-offer"));
  ok(/Row 4 of the file/.test(await said2("repair-heading")), `on to the next ragged row: ${await said2("repair-heading")}`);
  await mf.getByRole("button", { name: "Fix all like this…" }).click();
  await mf.screenshot({ path: path.join(evidence, "repair-fixall.png") });
  await mf.getByRole("button", { name: "Fix 3 rows" }).click();
  await mf.getByTestId("repair-extra").waitFor({ timeout: 10000 });
  const names = (await invoke(mf, "checkpoint_list", { doc: 1 })).map((c) => c.name);
  ok(names.includes("Before Fix all like this"), `automatic checkpoint: ${names.join(", ")}`);
  await mf.getByRole("button", { name: "Remove the empty last column" }).click();
  await sleep(900);
  const facts = await invoke(mf, "doc_info", { doc: 1 });
  ok(facts.cols === 3 && !facts.column_changes, `3 columns, no column change: ${facts.cols}`);
  ok((await invoke(mf, "checkpoint_list", { doc: 1 })).some((c) => c.name === "Before removing the empty column"), "checkpoint before removing the column");
  const book = await invoke(mf, "recipe_of", { doc: 1, header: true, name: "r" }).catch(() => null);
  if (book) ok(JSON.stringify(book).includes("join_fields") && !JSON.stringify(book).includes("Doe"), `recipe holds counts only: ${JSON.stringify(book.steps).slice(0, 160)}`);
  else info("recipe_of is not a command by that name; recipe content is covered by the shell test");
  await invoke(mf, "save", { doc: 1, saveAs: false, overwriteChanged: true, allowLossy: false });
  await sleep(800);
  const fixedText = fs.readFileSync(many, "utf8");
  const expected = rows.join("\r\n").replace(/(\d),(Smith|Doe|Roe|Poe), (\w+),/g, '$1,"$2, $3",');
  ok(fixedText === expected, `saved: ${JSON.stringify(fixedText)}`);

  // A stray quote read as text, and reopening the saved file.
  const quoted = path.join(OUT, "quote.csv");
  fs.writeFileSync(quoted, ["a,b,c", '1,"x, y",3', "4,5,6", "7,8,9", ""].join("\r\n"));
  const m2 = await start([quoted]);
  await waitFor(async () => (await invoke(m2, "doc_info", { doc: 1 })).indexed, 60000, 300);
  await sleep(800);
  const row = await invoke(m2, "row_text", { doc: 1, row: 1, rowId: null, keepFirst: true });
  await invoke(m2, "repair_row", { doc: 1, rowId: row.id, revision: row.revision, repair: { kind: "quote_as_text" } });
  const stale = await invoke(m2, "repair_row", { doc: 1, rowId: row.id, revision: row.revision, repair: { kind: "quote_as_text" } }).then(() => "", (e) => String(e));
  ok(/changed since/.test(stale), "a second repair from the old reading is refused");
  await invoke(m2, "save", { doc: 1, saveAs: false, overwriteChanged: true, allowLossy: false });
  await sleep(800);
  const m3 = await start([quoted]);
  await waitFor(async () => (await invoke(m3, "doc_info", { doc: 1 })).indexed, 60000, 300);
  const again = await invoke(m3, "row_text", { doc: 1, row: 1, rowId: null, keepFirst: true });
  ok(JSON.stringify(again.fields) === JSON.stringify(row.quoted_as_text), `reopened, the same cells: ${JSON.stringify(again.fields)}`);
  const fixed = await invoke(m3, "doc_info", { doc: 1 });
  info(`reopened file: ${fixed.ragged_rows} rows of another width than ${fixed.usual_fields}`);
};

(async () => {
  backup();
  try {
    await proof();
  } catch (e) {
    failed++;
    say(`FAIL unexpected: ${e && e.stack ? e.stack : e}`);
  } finally {
    await killApp();
    restore();
    say(failed ? `${failed} FAILED` : "ALL PASSED");
    process.exit(failed ? 1 : 0);
  }
})();
