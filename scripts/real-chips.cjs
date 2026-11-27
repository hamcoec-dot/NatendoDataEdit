// Drives the REAL built app to check Filter chips (C8) on a made-up voter list: two rules from Filter by selected
// value become two chips with the count line; a click switches one off (one undo step, the file stays saved and its
// bytes unchanged); Match any; Show duplicates joins as a third chip with "Added as a filter chip beside 2 others";
// an edit marks the tool chip as made before it, and Run again makes it fresh; the keyboard (Delete); the README
// screenshot of the bar. The app's own session, recent-file and recovery data are set aside and put back.
// Made-up data only.
// Usage: NODE_PATH=<playwright> node scripts/real-chips.cjs <exe> <evidence folder>
const { chromium } = require("playwright");
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const [exe, evidence] = process.argv.slice(2);
if (!exe || !evidence) {
  console.log("usage: node scripts/real-chips.cjs <exe> <evidence folder>");
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

// ---- the app (only the instance this script started is ever stopped) ----
let browser = null;
let ctx = null;
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

const OUT = path.join(WORK, `out-${Date.now()}`);
fs.mkdirSync(OUT, { recursive: true });
const FILE = path.join(OUT, "voters.csv");
const CRLF = "\r\n";
// Made-up voters: party DEM/REP/IND, three towns, a repeated name.
const NAMES = ["Ada Stone", "Bo Rivera", "Cy Okafor", "Dee Lind", "Eli Quinn", "Fay Varga", "Gus Wells", "Hal Moss", "Ivy Park", "Jo Lane", "Ada Stone", "Kim Shaw"];
const PARTY = ["DEM", "REP", "DEM", "IND", "DEM", "REP", "DEM", "REP", "DEM", "IND", "DEM", "DEM"];
const TOWN = ["Elmford", "Elmford", "Brook", "Elmford", "Brook", "Hillcrest", "Elmford", "Brook", "Hillcrest", "Elmford", "Elmford", "Brook"];
const SRC = ["voter_id,name,party,town", ...NAMES.map((n, i) => `${1001 + i},${n},${PARTY[i]},${TOWN[i]}`), ""].join(CRLF);
const facts = (m, doc = 1) => invoke(m, "doc_info", { doc });
const chips = (m) => invoke(m, "filter_chips", { doc: 1 });
const bar = "[data-testid=filter-chips]";
const chip = (i) => `${bar} [data-chip="${i}"]`;
const countLine = async (m) => ((await m.textContent("[data-testid=chips-count]").catch(() => "")) || "").trim();
const status = async (m) => ((await m.textContent(".msg").catch(() => "")) || "").replace(/\s+/g, " ").trim();
const idle = async (m) => {
  await sleep(500);
  await waitFor(async () => !(await m.$(`${bar} button:disabled`)), 15000);
  await sleep(400);
};
const opened = async (m) => {
  await waitFor(async () => (await facts(m)).indexed, 60000, 300);
  await sleep(1500);
  await dismiss(m);
};
// The active cell: Ctrl+Home (the first data row), then rows down and columns right.
const goTo = async (m, row, col) => {
  await m.locator("canvas").first().click({ position: { x: 300, y: 300 } });
  await m.keyboard.press("Control+Home");
  for (let i = 0; i < row; i++) await m.keyboard.press("ArrowDown");
  for (let i = 0; i < col; i++) await m.keyboard.press("ArrowRight");
  await sleep(200);
};

const proof = async () => {
  fs.writeFileSync(FILE, SRC);
  const before = hash(FILE);
  const m = await start([FILE]);
  await opened(m);
  ok((await m.$(bar)) === null, "no chips bar before a filter");
  // Party DEM (row 1 is Ada Stone, DEM), then town Elmford among the shown rows.
  await goTo(m, 0, 2);
  await cmd(m, "Filter by selected value");
  await m.waitForSelector(bar, { timeout: 10000 });
  await goTo(m, 0, 3);
  await cmd(m, "Filter by selected value");
  await idle(m);
  let c = await chips(m);
  const dems = PARTY.filter((p) => p === "DEM").length;
  const both = PARTY.filter((p, i) => p === "DEM" && TOWN[i] === "Elmford").length;
  ok(c.chips.length === 2 && c.shown === both, `two rules, two chips, ${c.shown} rows shown (want ${both})`);
  ok((await countLine(m)) === `${both} of ${NAMES.length} rows show.`, `the bar's count line: ${await countLine(m)}`);
  // A chip's own count reads the table: it comes when the bar is pointed at (C8 review M1).
  await m.hover(chip(0));
  await waitFor(async () => ((await m.textContent(chip(0))) || "").includes(String(PARTY.filter((p) => p === "DEM").length)), 5000);
  const t0 = ((await m.textContent(chip(0))) || "").replace(/\s+/g, " ");
  ok(/party/.test(t0) && t0.includes(String(dems)), `the party chip with its own count: ${t0.trim()}`);

  // Switch the town chip off: one undo step, nothing saved changes.
  const h0 = (await facts(m)).history_pos;
  await m.click(chip(1));
  await idle(m);
  let f = await facts(m);
  ok(f.rows === dems + 1 && f.history_pos === h0 + 1 && !f.modified, `town off: ${f.rows - 1} rows, one undo step, file still saved`);
  ok(/^Switched the chip “town .*” off\./.test(await status(m)), `status: ${await status(m)}`);
  await m.keyboard.press("Control+z");
  await idle(m);
  f = await facts(m);
  ok(f.rows === both + 1, "Ctrl+Z switches it on again");

  // Match any.
  await m.getByRole("button", { name: "Match any" }).click();
  await idle(m);
  const either = PARTY.filter((p, i) => p === "DEM" || TOWN[i] === "Elmford").length;
  f = await facts(m);
  ok(f.rows === either + 1, `Match any: ${f.rows - 1} rows (want ${either})`);
  await m.getByRole("button", { name: "Match all" }).click();
  await idle(m);
  ok(hash(FILE) === before, "the file's bytes are unchanged by every chip change");

  // Show duplicates on voter_id finds none: the two chips stay (only the tool's own chip goes).
  const dlg = "[role=dialog]:has(h2:has-text('Show duplicates'))";
  const dupsOn = async (nth) => {
    await cmd(m, "Show duplicates");
    await m.waitForSelector(dlg, { timeout: 10000 });
    await m.click(`${dlg} .all-none ~ div input[type=checkbox] >> nth=${nth}`);
    await m.click(`${dlg} button.primary`);
  };
  await dupsOn(0);
  await sleep(1500);
  await idle(m);
  c = await chips(m);
  ok(/^No duplicates found\./.test(await status(m)) && !/Added as a filter chip/.test(await status(m)), `none found, said plainly: ${await status(m)}`);
  ok(c.chips.length === 2 && c.shown === both, `and the two chips stay, ${c.shown} rows shown`);
  // Show duplicates on name joins as a third chip.
  await dupsOn(1);
  await sleep(1500);
  await idle(m);
  c = await chips(m);
  ok(c.chips.length === 3 && c.chips[2].kind === "duplicates", `Show duplicates is a third chip (${c.chips.map((x) => x.kind).join(", ")})`);
  ok(/Added as a filter chip beside 2 others\./.test(await status(m)), `status: ${await status(m)}`);
  ok(c.chips[2].not_looked === NAMES.length - both, `the tool chip looked only at the rows shown (${c.chips[2].not_looked} not looked at)`);
  await m.screenshot({ path: path.join(evidence, "chips-real.png") });
  const box = await m.locator(bar).boundingBox();
  if (box) await m.screenshot({ path: path.join(evidence, "filter-chips.png"), clip: { x: 0, y: Math.max(0, box.y - 4), width: Math.min(1100, box.x + box.width), height: box.height + 230 } });

  // An edit: the tool chip is marked as made before it; Run again makes it fresh.
  await goTo(m, 0, 3);
  await m.keyboard.type("Elmford Town");
  await m.keyboard.press("Enter");
  await sleep(1000);
  await idle(m);
  c = await chips(m);
  ok(c.chips[2].stale, "after an edit the tool chip is marked as made before it");
  ok(/⟳/.test((await m.textContent(chip(2))) || ""), "and the bar marks it");
  await m.click(chip(2), { button: "right" });
  await sleep(400);
  await m.locator(".chip-menu button", { hasText: "Run again" }).click();
  await sleep(1500);
  await idle(m);
  c = await chips(m);
  ok(c.chips.length === 3 && !c.chips[2].stale, `Run again: the chip is fresh (${JSON.stringify(c.chips.map((x) => [x.kind, x.stale, x.rows]))}; ${await status(m)})`);

  // Delete on a focused chip takes it out.
  await m.focus(chip(2));
  await m.keyboard.press("Delete");
  await idle(m);
  c = await chips(m);
  ok(c.chips.length === 2, "Delete takes the focused chip out");
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
