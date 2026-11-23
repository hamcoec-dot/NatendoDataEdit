// UI test of a tab in its own window, in the browser preview (the mock backend
// stands in for the app): the move and move-back commands, the second window
// mode (no start-up offers, no session, no settings written, no file opening),
// and the quit question built from the app's list of unsaved files. The real
// two-window behaviour is checked with scripts/real-own-window.cjs.
// Run: (cd ui && npx vite --port 5212) then
// NODE_PATH=<folder with playwright> node scripts/ui-window.cjs [screenshot folder] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5212";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1360, height: 860 }, permissions: ["clipboard-read", "clipboard-write"] });
  const errors = [];
  let failed = 0;
  const ok = (cond, msg) => {
    if (!cond) failed++;
    console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  };
  const newPage = async (query) => {
    const p = await ctx.newPage();
    p.on("pageerror", (e) => errors.push(String(e)));
    await p.goto(url + query);
    await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });
    await p.waitForTimeout(700);
    return p;
  };
  const command = async (p, q) => {
    await p.keyboard.press("Alt+q");
    await p.keyboard.type(q);
    await p.waitForTimeout(300);
    await p.keyboard.press("Enter");
    await p.waitForTimeout(500);
  };
  const calls = (p) => p.evaluate(() => window.__nde.backend.calls.slice());

  // ---- the main window ----
  const main = await newPage("");
  await main.keyboard.press("Control+n");
  await main.waitForTimeout(600);
  const id = await main.evaluate(() => window.__nde.tab.id);
  await command(main, "Move tab to a new window");
  const after = await main.evaluate(() => ({ tabs: window.__nde.tabs.length, moved: window.__nde.backend.moved.map((h) => ({ doc: h.doc, hasState: !!h.state && "snap" in h.state, view: h.view !== undefined })) }));
  ok(after.tabs === 0 && after.moved.length === 1 && after.moved[0].doc === id, "Move tab to a new window hands the tab over and it leaves this window");
  ok(after.moved[0].hasState && after.moved[0].view, "with its look and place, and its layout view for the session");
  // The app hands a tab back (or it came back on its own): it is adopted.
  await main.evaluate((h) => window.__nde.backend.handoverArrives(h), { doc: id, state: after.moved[0] && null, row: 0, col: 0, note: "The second window did not respond, so the tab is back here." });
  await main.waitForTimeout(700);
  ok((await main.evaluate(() => window.__nde.tabs.length)) === 1, "a tab handed to this window shows up here");
  ok(/did not respond/.test(await main.textContent("footer.status")), "and the reason it came back is shown");
  ok((await main.locator("[role=menuitem]").count()) === 0, "(no menu left open)");

  // ---- the quit question comes from the app's list, and carries it back ----
  await main.keyboard.press("Control+n");
  await main.waitForTimeout(600);
  const doc2 = await main.evaluate(() => window.__nde.tab.id);
  await main.evaluate(async (d) => {
    await window.__nde.backend.setCells(d, [[0, 0, "edited"]]);
  }, doc2);
  await command(main, "Exit");
  const dlg = main.locator(".dialog[role=alertdialog]");
  const text = (await dlg.textContent()) ?? "";
  ok(/Quit without saving\?/.test(text) && /1 file has unsaved changes/.test(text), "Exit asks about the unsaved file the app lists");
  await dlg.getByRole("button", { name: /Quit and lose changes/ }).click();
  await main.waitForTimeout(600);
  const quit = (await calls(main)).filter((c) => c.startsWith("quitApp"));
  ok(quit.length === 1 && quit[0] === `quitApp:${doc2}`, `Quit hands the app the list the question showed (${quit})`);

  // ---- the second window ----
  const second = await newPage("?window=2");
  const c2 = await calls(second);
  const forbidden = ["sessionList", "sessionSave", "takeOpened", "recoveryList", "setAgentLink", "versionsKeep"].filter((n) => c2.includes(n));
  ok(forbidden.length === 0, `window 2 starts none of the main window's start-up work (${forbidden.join(", ") || "none"})`);
  ok(c2.includes("windowTake"), "it takes the tabs waiting for it");
  ok((await second.locator(".dialog[role=alertdialog]").count()) === 0, "and offers nothing (no reopen or recovery question)");
  // A tab arrives.
  const sid = await second.evaluate(async () => (await window.__nde.backend.newDocument()).id);
  await second.evaluate((d) => window.__nde.backend.handoverArrives({ doc: d, state: null, row: 0, col: 0 }), sid);
  await second.waitForTimeout(700);
  ok((await second.evaluate(() => window.__nde.tabs.length)) === 1, "a tab handed to window 2 shows up there");
  await second.evaluate(() => window.__nde.backend.calls.length = 0);
  await new Promise((r) => setTimeout(r, 3500));
  ok((await calls(second)).some((c) => c.startsWith("windowSync:1")), "window 2 reports its tabs to the app regularly (it is alive)");
  // Settings are changed in the main window.
  const prefsBefore = await second.evaluate(() => localStorage.getItem("natendo.prefs.v1"));
  await second.keyboard.press("Control+Equal");
  await second.keyboard.press("Control+=");
  await command(second, "Zoom in");
  const prefsAfter = await second.evaluate(() => localStorage.getItem("natendo.prefs.v1"));
  ok(prefsBefore === prefsAfter, "window 2 writes no preference");
  ok(/main window/.test(await second.textContent("footer.status")), "and says where settings are changed");
  await command(second, "Options");
  ok((await second.locator("[role=dialog]:has(h2:has-text('Options'))").count()) === 0, "Options does not open there");
  // Back to the grid (a disabled command leaves the search open).
  await second.keyboard.press("Escape");
  await second.keyboard.press("Escape");
  await second.locator("canvas").click({ position: { x: 200, y: 80 } });
  await second.waitForTimeout(300);
  // Opening a file is refused; the person is told what to do.
  await second.keyboard.press("Control+o");
  await second.waitForTimeout(600);
  ok(/Open files in the main window/.test(await second.textContent("footer.status")), "opening a file in window 2 says to open it in the main window");
  ok((await second.evaluate(() => window.__nde.tabs.length)) === 1, "and opens nothing there");
  // Moving back.
  await command(second, "Move tab back to the main window");
  const c3 = await calls(second);
  ok(c3.includes(`windowMoveBack:${sid}`) && (await second.evaluate(() => window.__nde.tabs.length)) === 0, "Move tab back hands the tab to the main window");
  // Closing window 2 reports, then asks the app to close it.
  await second.evaluate((d) => window.__nde.backend.handoverArrives({ doc: d, state: null, row: 0, col: 0 }), sid);
  await second.waitForTimeout(600);
  await second.evaluate(() => window.__nde.backend.calls.length = 0);
  await command(second, "Exit");
  const c4 = await calls(second);
  ok(c4.indexOf("windowCloseSelf") > c4.findIndex((c) => c.startsWith("windowSync")) && c4.includes("windowCloseSelf") && !c4.some((c) => c.startsWith("quitApp")), "its Exit reports its tabs, closes only that window, and never quits the app");
  if (out) await second.screenshot({ path: path.join(out, "window2.png") });

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
