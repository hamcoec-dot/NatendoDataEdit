// UI test of Change encoding in the browser stand-in: the status-bar button, the list with the arrow keys scrubbing a
// preview, Esc changing nothing, Apply in place as one undo step, the refusal with unsaved edits, Reopen for a
// non-ASCII encoding, the scan result with "show only these rows", and the Fix mojibake step.
// Run: (cd ui && npx vite --port 5281) then NODE_PATH=<folder with playwright> node scripts/ui-encoding.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5281";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1280, height: 820 } });
  const p = await ctx.newPage();
  const errors = [];
  p.on("pageerror", (e) => errors.push(String(e)));
  await p.goto(url);
  await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });
  let failed = 0;
  const ok = (cond, msg) => {
    if (!cond) failed++;
    console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  };
  const TAB = String.fromCharCode(9);
  const EOL = String.fromCharCode(13, 10);
  const freshFile = async () => {
    await p.evaluate(() => window.__nde.reg.run("file.new"));
    await p.waitForTimeout(500);
    await p.evaluate(
      async (text) => {
        const t = window.__nde.tab;
        t.update(await window.__nde.backend.dataOp(t.id, { op: "paste", spec: { row: 0, col: 0, op: "none", skip_blanks: false, transpose: false }, text }).then((r) => r.info), true);
        t.header = true;
        // Saved (like the engine, that starts the undo history again) and with a place on disk, so the picker is on.
        t.update(await window.__nde.backend.save(t.id, { saveAs: true, overwriteChanged: false, allowLossy: false, options: null }).catch(() => window.__nde.backend.save(t.id, {})), true);
        window.__nde.backend.get(t.id).info.path = "C:/samples/names.csv";
        t.info.path = "C:/samples/names.csv";
        t.bump();
      },
      [["name", "city"], ["Jose", "Zurich"], ["Renee", "Liege"], ["Ann", "Oslo"]].map((r) => r.join(TAB)).join(EOL),
    );
    await p.waitForTimeout(400);
  };
  const dialog = () => p.getByRole("dialog", { name: "Change encoding" });
  const open = async () => {
    await p.getByTestId("encoding-button").click();
    await dialog().waitFor({ timeout: 5000 });
    await p.waitForTimeout(300);
  };
  const button = async () => ((await p.getByTestId("encoding-button").textContent()) || "").trim();

  await freshFile();
  ok((await button()).startsWith("UTF-8"), `the status bar names the encoding (${await button()})`);
  await open();
  const options = await dialog().getByRole("option").allTextContents();
  ok(options.length === 9 && options[0].includes("UTF-8 (now)"), `nine encodings, the one in use marked now (${options.length})`);
  if (out) await p.screenshot({ path: path.join(out, "encoding.png") });

  // The arrow keys scrub.
  await dialog().getByRole("listbox").focus();
  await p.keyboard.press("ArrowDown");
  await p.waitForTimeout(500);
  let preview = ((await p.getByTestId("enc-preview").textContent()) || "").replace(/\s+/g, " ");
  ok(/Jos\u00c3\u00a9/.test(preview), `ArrowDown to Windows-1252 shows the first rows read that way (${preview.slice(0, 60)})`);
  await p.keyboard.press("ArrowDown");
  await p.waitForTimeout(500);
  preview = ((await p.getByTestId("enc-preview").textContent()) || "").replace(/\s+/g, " ");
  ok(preview.includes("\ufffd"), "the next encoding shows replacement characters: not the right one");
  await p.keyboard.press("ArrowUp");
  await p.keyboard.press("ArrowUp");
  await p.waitForTimeout(400);
  preview = ((await p.getByTestId("enc-preview").textContent()) || "").replace(/\s+/g, " ");
  ok(/Jos\u00e9/.test(preview), "and back to UTF-8 shows the right text");

  // Esc changes nothing.
  await p.keyboard.press("ArrowDown");
  await p.waitForTimeout(300);
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);
  ok((await dialog().count()) === 0 && (await button()).startsWith("UTF-8"), "Esc closes the picker and the encoding is as it was");

  // Apply in place: one undo step.
  await open();
  await dialog().getByRole("listbox").focus();
  await p.keyboard.press("ArrowDown");
  await p.waitForTimeout(400);
  await p.keyboard.press("Enter");
  await p.waitForTimeout(600);
  ok((await dialog().count()) === 0 && (await button()).startsWith("windows-1252"), `Enter applies it (${await button()})`);
  await p.evaluate(() => window.__nde.grid.focus());
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(500);
  ok((await button()).startsWith("UTF-8"), "one Ctrl+Z reads the file as UTF-8 again");
  await p.keyboard.press("Control+y");
  await p.waitForTimeout(500);
  ok((await button()).startsWith("windows-1252"), "Ctrl+Y reads it as Windows-1252 again");

  // Unsaved edits: refused, and it says why.
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    t.update(await window.__nde.backend.setCells(t.id, [[1, 0, "Edited"]]), true);
  });
  await open();
  await dialog().getByRole("listbox").focus();
  await p.keyboard.press("ArrowUp");
  await p.waitForTimeout(300);
  await p.keyboard.press("Enter");
  await p.waitForTimeout(500);
  const why = ((await p.locator("#app").textContent()) || "").includes("Save or undo your changes first");
  ok(why && (await button()).startsWith("windows-1252"), "with unsaved edits Apply is refused and says to save or undo first");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);

  // Reopen for a non-ASCII encoding (a fresh, unchanged file).
  await freshFile();
  await open();
  await dialog().getByRole("option", { name: /Shift_JIS/ }).click();
  await p.waitForTimeout(400);
  ok(((await dialog().getByRole("button", { name: /Reopen as this|Apply/ }).first().textContent()) || "").includes("Reopen"), "a non-ASCII encoding says it reopens the file");
  await dialog().getByRole("button", { name: "Reopen as this" }).click();
  await p.waitForTimeout(800);
  ok((await button()).startsWith("Shift_JIS"), `the file is open again as Shift_JIS (${await button()})`);

  // The scan.
  await freshFile();
  await open();
  await dialog().getByRole("button", { name: "Look for rows in another encoding" }).click();
  await p.getByTestId("enc-scan").waitFor({ timeout: 5000 });
  const scan = ((await p.getByTestId("enc-scan").textContent()) || "").replace(/\s+/g, " ");
  ok(/not valid UTF-8/.test(scan) && /double-encoded/.test(scan) && /mixes two encodings/.test(scan), `the scan says what it found (${scan.slice(0, 90)})`);
  ok((await dialog().getByRole("button", { name: /Show only these 2 rows/ }).count()) === 1, "and offers to show only those rows");
  await dialog().getByRole("button", { name: "Fix mojibake…" }).click();
  await p.waitForTimeout(500);
  ok((await p.getByText(/Fix mojibake/).count()) >= 1, "Fix mojibake opens the clean-up with that tool");

  ok(errors.length === 0, `no page errors ${errors.join(" | ").slice(0, 200)}`);
  await b.close();
  process.exit(failed ? 1 : 0);
})();
