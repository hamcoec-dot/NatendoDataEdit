<script lang="ts">
  import { onMount, tick } from "svelte";
  import { backend, errorText, isCmdError, type AppInfo, type Change, type DocInfo, type Layout } from "./lib/backend";
  import { Tab } from "./lib/tab.svelte";
  import { prefs, customFont, ACCENTS, FONTS, type Theme } from "./lib/prefs.svelte";
  import { Registry, altOf, keyOf, keyOfLoose, watchAlt, type Command, type Menu } from "./lib/commands";
  import { parseTsv } from "./lib/tsv";
  import { cellCount, type Rect } from "./lib/selection";
  import { dropIndex, MOUSE_MOVES } from "./lib/dragmove";
  import { looksLikeHeader } from "./lib/header";
  import { separatorName } from "./lib/layout";
  import Grid from "./lib/Grid.svelte";
  import LayoutDialog from "./lib/LayoutDialog.svelte";
  import Icon from "./lib/Icon.svelte";
  import Dialog, { type DialogRequest, type DialogButton } from "./lib/Dialog.svelte";
  import FilterMenu from "./lib/FilterMenu.svelte";
  import SortDialog from "./lib/SortDialog.svelte";
  import FindPanel from "./lib/FindPanel.svelte";
  import { describeRule, ruleFor, withRule, type FindOptions } from "./lib/filters";
  import type { Pattern, Rule, SortKey, SortSpec, DataOp, Dir, FillSpec, OpReport, TableOp } from "./lib/backend";
  import OpDialog, { type OpKind } from "./lib/OpDialog.svelte";
  import FormView from "./lib/FormView.svelte";
  import SymbolPicker from "./lib/SymbolPicker.svelte";
  import SaveAsDialog from "./lib/SaveAsDialog.svelte";
  import ExportDialog from "./lib/ExportDialog.svelte";
  import ImportDialog from "./lib/ImportDialog.svelte";
  import JoinDialog from "./lib/JoinDialog.svelte";
  import ValueEditor from "./lib/ValueEditor.svelte";
  import DatePicker from "./lib/DatePicker.svelte";
  import PrivacyDialog from "./lib/PrivacyDialog.svelte";
  import MailDialog from "./lib/MailDialog.svelte";
  import ChangesDialog from "./lib/ChangesDialog.svelte";
  import RecipeDialog from "./lib/RecipeDialog.svelte";
  import { withPhrase, type RecipeOpened, type RecipeView } from "./lib/recipe";
  import FolderSearchDialog, { type FolderHit } from "./lib/FolderSearchDialog.svelte";
  import { copyTouches, findingsText, LARGE_COPY } from "./lib/privacy";
  import EditTools, { type ToolKind as EditKind } from "./lib/EditTools.svelte";
  import CleanDialog from "./lib/CleanDialog.svelte";
  import SplitNameDialog from "./lib/SplitNameDialog.svelte";
  import InsertDialog from "./lib/InsertDialog.svelte";
  import { TOOLS, localNow, type Cleaner, type CleanSpec, type ToolId } from "./lib/clean";
  import { joinSummary, type OtherTable } from "./lib/transfer";
  import type { ExportOptions, ImportOptions, ImportStart, Opened, SaveAsOptions } from "./lib/backend";
  import SidePanel, { type PanelTab } from "./lib/SidePanel.svelte";
  import CommandSearch from "./lib/CommandSearch.svelte";
  import readme from "../../README.md?raw";
  import { FEATURE_LINKS, featureDocs, readmeBullets } from "./lib/featureIndex";
  import ShortcutsDialog from "./lib/ShortcutsDialog.svelte";
  import HighlightDialog from "./lib/HighlightDialog.svelte";
  import SubtotalDialog from "./lib/SubtotalDialog.svelte";
  import PivotDialog from "./lib/PivotDialog.svelte";
  import type { PivotSpec } from "./lib/pivot";
  import PromptDialog from "./lib/PromptDialog.svelte";
  import ComputeDialog from "./lib/ComputeDialog.svelte";
  import ChecksDialog from "./lib/ChecksDialog.svelte";
  import FieldCheckDialog from "./lib/FieldCheckDialog.svelte";
  import MatchDialog from "./lib/MatchDialog.svelte";
  import StreetDialog from "./lib/StreetDialog.svelte";
  import UpdatesDialog from "./lib/UpdatesDialog.svelte";
  import EncryptDialog from "./lib/EncryptDialog.svelte";
  import SqliteDialog from "./lib/SqliteDialog.svelte";
  import NotesDialog from "./lib/NotesDialog.svelte";
  import WatchDialog from "./lib/WatchDialog.svelte";
  import { streetSummary } from "./lib/streets";
  import PrintDialog from "./lib/PrintDialog.svelte";
  import OptionsDialog from "./lib/OptionsDialog.svelte";
  import QuickTotals from "./lib/QuickTotals.svelte";
  import FormatDialog from "./lib/FormatDialog.svelte";
  import KeysDialog from "./lib/KeysDialog.svelte";
  import RecentDialog from "./lib/RecentDialog.svelte";
  import SuggestDialog from "./lib/SuggestDialog.svelte";
  import RecoveryDialog from "./lib/RecoveryDialog.svelte";
  import ToolsDialog, { type ToolsRun } from "./lib/ToolsDialog.svelte";
  import { unchangedTabs, type ScaleKind, type ToolKind } from "./lib/m10";
  import { autosaveDue, type RecoveryItem, type Suggestion, type Suggestions, type VersionItem } from "./lib/m9";
  import { BASIC_KEYS, cleanFormat, describeFormat, formatSteps, keyGroups, MAX_ROW_LINES, type DisplayFormat } from "./lib/m9";
  import { defaultPage, printHtml, PRINT_MAX_CELLS, PRINT_MAX_ROWS } from "./lib/m8";
  import type { Check, CheckSpec, ComputeSpec, PageSetup } from "./lib/backend";
  import { exportDefaults } from "./lib/transfer";
  import { cleanShortcuts, matchColumns, safeFont, MAX_VIEWS, type RecentItem, type Shortcuts, type View } from "./lib/views";

  const DEFAULT_COL_WIDTH = 130;
  const MAX_CELLS_PER_CHANGE = 100_000;

  let info = $state<AppInfo | null>(null);
  let tabs = $state<Tab[]>([]);
  let active = $state(0);
  let grid = $state<ReturnType<typeof Grid>>();
  let dialog = $state<DialogRequest | null>(null);
  let message = $state<string>("");
  let messageTimer = 0;
  let openMenu = $state<number | null>(null);
  let menuIndex = $state(0);
  let menuByKeyboard = $state(false);
  let context = $state<{ x: number; y: number; items: string[]; index: number } | null>(null);
  let cellBarText = $state("");
  let cellBarDirty = $state(false);
  let cellBarTruncated = $state(false);
  let cellBarExpanded = $state(false);
  let gotoText = $state("");
  let gotoInput = $state<HTMLInputElement>();
  let cellBarInput = $state<HTMLTextAreaElement>();
  let filterMenu = $state<{ col: number; x: number; y: number } | null>(null);
  let sortDialog = $state<SortKey[] | null>(null);
  let opDialog = $state<{ kind: OpKind; clip: string } | null>(null);
  let formRow = $state<number | null>(null);
  /** Save As options screen, waiting for the user's choice. */
  let saveAsReq = $state<{ sample: string[][]; done: (o: SaveAsOptions | null) => void } | null>(null);
  let exportOpen = $state(false);
  let importing = $state<ImportStart | null>(null);
  let joinKind = $state<"join" | "append" | null>(null);
  let valuesOpen = $state(false);
  let datePick = $state<{ row: number; col: number; text: string } | null>(null);
  let privacyOpen = $state(false);
  /** Label sheets, once the mail screen is open. */
  let mailSheets = $state<[string, import("./lib/mail").LabelSheet][] | null>(null);
  let mailProblem = $state("");
  /** List of my changes: open, and what it found (null while it looks). */
  let changesOpen = $state(false);
  let recipeOpen = $state(false);
  let recipeCurrent = $state<RecipeView | null>(null);
  let recipeFile = $state<RecipeOpened | null>(null);
  let folderSearchOpen = $state(false);
  let changes = $state<import("./lib/backend").ChangeList | null>(null);
  let privacyProblem = $state("");
  /** The tab the clipboard was last filled from (cleared when a private tab closes). */
  let copiedFrom: number | null = null;
  let editKind = $state<EditKind | null>(null);
  /** The cleaning screen, open at a tool. */
  let cleanTool = $state<ToolId | null>(null);
  /** The cleaner being built (kept while the app runs). */
  let cleaner = $state<Cleaner>({ version: 1, name: "", steps: [] });
  let splitNameOpen = $state(false);
  /** "Insert rows…" / "Insert columns…" asking how many. */
  let insertAsk = $state<"rows" | "cols" | null>(null);
  /** Insert > Symbol is open; `caret` is where the cell editor's cursor was. */
  let symbols = $state<{ caret: number | null } | null>(null);
  let findOpen = $state(false);
  let findReplace = $state(false);
  let findPanel = $state<ReturnType<typeof FindPanel>>();
  let findOptions = $state<FindOptions>({ text: "", kind: "plain", matchCase: false, wholeCell: false, scope: "all" });
  /** Search inside the expanded cell bar (the multi-line editor). */
  let cellFind = $state<{ text: string; hits: [number, number][]; at: number } | null>(null);
  /** Messages shown this session, newest first (the Activity panel). */
  let activity = $state<{ time: string; text: string }[]>([]);
  /** Command search is open, starting with this text. */
  let search = $state<string | null>(null);
  let shortcutsOpen = $state(false);
  let keysOpen = $state(false);
  /** The shortcuts list was opened at its mouse section (Help > Mouse and drag moves). */
  let keysMouse = $state(false);
  /** Recovery copies from a run that stopped, or the current file's earlier saves. */
  let recovery = $state<{ mode: "recover" | "versions"; items: (RecoveryItem | VersionItem)[] | null; doc: number; name: string } | null>(null);
  /** When each tab last got a recovery copy, or first had unsaved changes (ms). */
  const autosavedAt = new Map<number, number>();
  let autosaveNote = $state("");
  /** Clean-up suggestions: what the last scan found (null while scanning). */
  let suggest = $state<{ found: Suggestions | null; busy: boolean } | null>(null);
  /** Display format screen for these columns. */
  let formatFor = $state<{ cols: number[]; what: string; current: DisplayFormat | null; samples: string[] } | null>(null);
  let highlightOpen = $state(false);
  /** The Tools dialog that is open, if any. */
  let toolsKind = $state<ToolKind | null>(null);
  let subtotalOpen = $state(false);
  /** The tab the pivot builder works on (null: closed). */
  let pivotDoc = $state<number | null>(null);
  /** Each tab's last pivot layout, and which tab each pivot tab came from. */
  const pivotSpecs = new Map<number, PivotSpec>();
  const pivotOf = new Map<number, { source: number; spec: PivotSpec }>();
  /** A one-line question (a view's name, a font name). */
  let prompt = $state<{ title: string; label: string; value: string; note?: string; check?: (v: string) => string; done: (v: string) => void } | null>(null);
  let recent = $state<RecentItem[]>([]);
  /** File > Open recent is showing. */
  let recentOpen = $state(false);
  let panelTab = $state<PanelTab>(prefs.data.panel ?? "summary");
  let computeOpen = $state(false);
  let checksOpen = $state(false);
  let fieldCheckOpen = $state(false);
  let matchOpen = $state(false);
  let streetOpen = $state(false);
  let updatesOpen = $state(false);
  let encryptMode = $state<"save" | "open" | null>(null);
  /** Save encrypted over this file (Save on a tab from an encrypted file). */
  let encryptSame = $state<string | null>(null);
  let sqliteMode = $state<"save" | "open" | null>(null);
  let notesRow = $state<number | null>(null);
  let watchOpen = $state(false);
  let optionsOpen = $state(false);
  /** The print screen: what would print, and whether PDF is the main button. */
  let printReq = $state<{ pdf: boolean; titles: string[] | null; sample: string[][]; cols: number[]; allRows: number; selRows: number; selFirst: number } | null>(null);
  /** Page setup used last (kept while the app runs). */
  let pageSetup = $state<PageSetup | null>(null);
  const READ = { decimal_comma: false, date_order: "mdy" as const };

  const tab = $derived<Tab | undefined>(tabs[active]);

  // ---- helpers ------------------------------------------------------------------
  function say(text: string): void {
    message = text;
    if (text) {
      activity = [{ time: new Date().toLocaleTimeString(), text }, ...activity].slice(0, 200);
      void backend().logEvent("info", text).catch(() => {});
    }
    clearTimeout(messageTimer);
    messageTimer = window.setTimeout(() => (message = ""), 6000);
  }

  function ask(title: string, msg: string, buttons: DialogButton[], cancel: string, detail?: string): Promise<string> {
    return new Promise((resolve) => {
      dialog = { title, message: msg, detail, buttons, cancel, resolve };
    });
  }

  const ready = () => !!tab && tab.info.indexed && !tab.busy;
  const hasTab = () => !!tab;

  /** Run an engine call for the active tab and take the new document state. */
  async function apply(f: (t: Tab) => Promise<DocInfo>, dataChanged = true): Promise<boolean> {
    const t = tab;
    if (!t) return false;
    try {
      t.update(await f(t), dataChanged);
      return true;
    } catch (e) {
      say(errorText(e));
      return false;
    }
  }

  function wholeRows(r: Rect, t: Tab): boolean {
    return r.col === 0 && r.cols >= t.info.cols;
  }
  function wholeCols(r: Rect, t: Tab): boolean {
    return r.row <= t.firstRow && r.row + r.rows >= t.info.rows;
  }

  // ---- documents -----------------------------------------------------------------
  function addTab(d: DocInfo, guess = true): void {
    const t = new Tab(d, DEFAULT_COL_WIDTH);
    tabs.push(t);
    active = tabs.length - 1;
    void tick().then(() => grid?.focus());
    if (guess) void guessHeader(t);
    void backend()
      .privateState(t.id)
      .then((p) => {
        t.private = p;
      })
      .catch(() => {});
  }

  // ---- private data (Milestone 13) ----------------------------------------------------
  /** What the finder says about a tab (looked once per tab; null if it failed). */
  async function findingsOf(t: Tab): Promise<import("./lib/backend").Finding[]> {
    if (t.findings) return t.findings;
    try {
      const d = new Date();
      t.findings = await backend().findPrivate(t.id, t.header, READ, Math.floor((d.getTime() - d.getTimezoneOffset() * 60_000) / 86_400_000));
    } catch {
      return [];
    }
    return t.findings;
  }

  /** Before data leaves the app: warn if the tab looks private. False to stop. */
  async function privacyGate(what: string, cols?: [number, number]): Promise<boolean> {
    const t = tab;
    if (!t || !prefs.data.privacyWarnings || t.warned) return true;
    let f = await findingsOf(t);
    if (cols) f = f.filter((x) => copyTouches([x], cols[0], cols[1]));
    if (!f.length) return true;
    const a = await ask(
      "This file looks like it holds private data",
      `Columns: ${findingsText(f)} (looked at the titles and the first 20,000 rows). ${what} A share-safe copy can leave those columns out or mask them first.`,
      [
        { id: "safe", label: "Make a share-safe copy…", primary: true },
        { id: "go", label: "Continue" },
        { id: "always", label: "Continue, and don't ask again for this file" },
        { id: "cancel", label: "Cancel" },
      ],
      "cancel",
      "You can switch these warnings off in Options.",
    );
    if (a === "safe") {
      void openPrivacy();
      return false;
    }
    if (a === "always") t.warned = true;
    return a === "go" || a === "always";
  }

  /** What the private data dialog shows (null while looking). */
  let privacyFindings = $state<import("./lib/backend").Finding[] | null>(null);

  /** What a split from a private tab did about private file mode. */
  function splitPrivateText(rep: import("./lib/backend").SplitReport): string {
    const on = rep.private_folder
      ? ` The folder was empty, so it is now in private file mode, like this file.`
      : rep.private
        ? ` The new files are in private file mode, like this one.`
        : "";
    const missed = rep.private_missed
      ? ` ${plural(rep.private_missed, "new file")} could not be put in private file mode (the list of private files is full); put their folder in private file mode or remove them.`
      : "";
    return on + missed;
  }

  // The scramble key is kept only while a screen that uses it is open.
  const keyScreenOpen = $derived(cleanTool !== null || privacyOpen);
  $effect(() => {
    if (!keyScreenOpen) void backend().forgetScrambleKey().catch(() => {});
  });

  async function openPrivacy(): Promise<void> {
    const t = tab;
    if (!t) return;
    privacyProblem = "";
    privacyOpen = true;
    privacyFindings = null;
    t.findings = null;
    await findingsOf(t);
    // A failed look leaves every column as it is rather than waiting forever
    // (the tab keeps no answer, so later warnings look again).
    privacyFindings = t.findings ?? [];
    if (t.findings === null) privacyProblem = "Could not look for private data; choose what to do with each column yourself.";
  }

  async function saveSafeCopy(spec: import("./lib/backend").SafeCopy): Promise<void> {
    const t = tab;
    if (!t) return;
    privacyProblem = "";
    try {
      const rep = await runJob("Writing a share-safe copy", () => backend().saveSafeCopy(t.id, spec, localNow()));
      if (!rep) return;
      privacyOpen = false;
      const blanks = (rep.blanked_cols ?? []).map(([c, n]) => `${plural(n, "value")} in ${columnTitle(t, c)}`).join(", ");
      say(
        `Saved a share-safe copy: ${plural(rep.rows, "row")}, ${plural(rep.cols, "column")}${rep.changed ? `, ${plural(rep.changed, "value")} masked` : ""}. The open file did not change.` +
          (rep.blanked ? ` ${blanks} could not be read and were left blank, not copied as they were.` : ""),
      );
    } catch (e) {
      privacyProblem = errorText(e);
    }
  }

  async function togglePrivate(folder: boolean): Promise<void> {
    const t = tab;
    if (!t) return;
    const on = folder ? !t.private.folder : !t.private.file;
    try {
      t.private = await backend().setPrivate(t.id, folder, on);
      void loadRecent();
      say(
        on
          ? `Private file mode is on for ${folder ? "this folder" : "this file"}: no recovery copies, no earlier saves, not in the recent files, and the clipboard is cleared when the tab closes.`
          : `Private file mode is off for ${folder ? "this folder" : "this file"}.`,
      );
      if (!on) return;
      // Copies kept from before: delete them only if the user says so.
      const [saves, copies] = await backend().privateLeftovers(t.id, folder);
      if (!saves && !copies) return;
      const parts = [saves ? plural(saves, "earlier save") : "", copies ? plural(copies, "recovery copy", "recovery copies") : ""].filter(Boolean).join(" and ");
      const a = await ask(
        "Delete the copies kept from before?",
        `The app still keeps ${parts} of ${folder ? "files in this folder" : "this file"} from before it was private, in its own data folder. Delete them now? Private file mode stays on either way.`,
        [
          { id: "delete", label: "Delete them", primary: true },
          { id: "keep", label: "Keep them" },
        ],
        "keep",
      );
      if (a === "delete") {
        const n = await backend().privateForget(t.id, folder);
        say(`Deleted ${plural(n, "kept copy", "kept copies")}. Private file mode is on.`);
      }
    } catch (e) {
      say(errorText(e));
    }
  }

  /** Set "First row is header" from a look at the first rows (the file is
   *  not changed; the user can flip the switch any time). */
  async function guessHeader(t: Tab): Promise<void> {
    try {
      // A file opened from Windows can arrive before its first rows are
      // read; wait briefly (up to 2 s) for them, or the guess sees nothing.
      let info = t.info;
      for (let i = 0; i < 40 && !info.indexed && (info.rows < 20 || info.cols === 0); i++) {
        await new Promise((r) => setTimeout(r, 50));
        info = await backend().docInfo(t.id);
      }
      const v = await backend().viewport(t.id, 0, 20, 0, Math.min(Math.max(info.cols, t.info.cols), 50), 200);
      const rows: string[][] = [];
      for (let r = 0; r < v.rows; r++) rows.push(v.cells.slice(r * v.cols, (r + 1) * v.cols));
      // Trailing empty columns (short rows padded by the viewport) are not titles.
      let width = 0;
      for (const r of rows) for (let c = r.length - 1; c >= width; c--) if (r[c] !== "") width = c + 1;
      const sample = rows.map((r) => r.slice(0, width));
      if (!t.headerTouched && looksLikeHeader(sample)) {
        t.header = true;
        if (t.sel.focus.row === 0) t.sel = { anchor: { row: 1, col: 0 }, focus: { row: 1, col: 0 } };
        t.bump();
      }
    } catch {
      // Keep the default (no header).
    }
  }

  async function openFile(): Promise<void> {
    try {
      opened(await backend().openFile());
    } catch (e) {
      say(errorText(e));
    }
  }

  /** Show a file that was opened (a table, or the import screen). */
  function opened(o: Opened | null): void {
    if (o?.doc) addTab(o.doc);
    else if (o?.import) importing = o.import;
    if (o) void loadRecent();
  }

  async function loadRecent(): Promise<void> {
    try {
      recent = await backend().recentFiles();
    } catch {
      recent = [];
    }
  }

  /** Recent files being opened, by id: a second middle-click while one is
   *  opening does not open it twice. */
  const recentBusy = new Set<string>();

  /** Open a recent file in a new tab, or switch to the tab already showing
   *  it. Files are named by id and matched to tabs by full path in the
   *  backend, so two files that look alike (2024\Data\voters.csv and
   *  2025\Data\voters.csv) are never mixed up. */
  async function openRecent(r: RecentItem): Promise<"opened" | "open" | "import" | "busy" | "failed"> {
    const tabOf = (doc: number | null | undefined) => (doc == null ? -1 : tabs.findIndex((t) => t.id === doc));
    const showing = tabOf(r.open_doc);
    if (showing >= 0) {
      active = showing;
      return "open";
    }
    if (recentBusy.has(r.id)) return "busy";
    recentBusy.add(r.id);
    try {
      const o = await backend().openRecent(r.id);
      // Opened elsewhere since the list was read: the backend hands back
      // that tab's document instead of opening the file again.
      const i = tabOf(o.doc?.id);
      if (i >= 0) {
        active = i;
        void loadRecent();
        return "open";
      }
      opened(o);
      return o.import ? "import" : o.doc ? "opened" : "failed";
    } catch (e) {
      say(errorText(e));
      void loadRecent();
      return "failed";
    } finally {
      recentBusy.delete(r.id);
    }
  }

  /** Tabs whose files share a name show the folder that tells them apart
   *  (worked out in the backend, as the recent list does). */
  let tabFolder = $state<Record<number, string>>({});
  let tabFolderAsk = 0;
  /** Which files the tabs show: changes only when a tab opens, closes or
   *  is saved under another name, not on every edit (the lookup below
   *  follows this string, which stays equal while cells change). */
  const tabFiles = $derived(tabs.map((t) => `${t.id}\u0001${t.info.name}\u0001${t.info.path ?? ""}`).join("\u0002"));
  $effect(() => {
    const files = tabFiles.split("\u0002").filter(Boolean).map((s) => s.split("\u0001"));
    const ids = files.map((f) => Number(f[0]));
    const names = files.map((f) => f[1]!.toLowerCase());
    const ask = ++tabFolderAsk;
    if (!names.some((n, i) => names.indexOf(n) !== i)) {
      tabFolder = {};
      return;
    }
    backend()
      .tabFolders(ids)
      .then((l) => {
        if (ask === tabFolderAsk) tabFolder = Object.fromEntries(ids.map((id, i) => [id, l[i] ?? ""]));
      })
      .catch(() => {});
  });

  /** The tabs' documents, to mark recent files that are open. */
  function openDocs(): Set<number> {
    return new Set(tabs.map((t) => t.id));
  }

  /** File > Open recent: open each picked file in its own tab, one after
   *  another. A file that needs the import screen stops the run there, so
   *  one import screen never replaces another. */
  async function openRecentMany(list: RecentItem[], keepList = false): Promise<void> {
    if (!keepList) recentOpen = false;
    let already = 0;
    let done = 0;
    for (const [n, r] of list.entries()) {
      const got = await openRecent(r);
      if (got === "open") already++;
      else if (got === "opened") done++;
      else if (got === "import") {
        recentOpen = false;
        const left = list.length - n - 1;
        if (left) say(`${done ? `Opened ${done}. ` : ""}${r.name} needs its import options, so ${left === 1 ? "the other file was" : `the other ${left} files were`} not opened.`);
        return;
      }
    }
    if (already && list.length > 1) say(`${already === 1 ? "One file was" : `${already} files were`} already open, so ${already === 1 ? "it was" : "they were"} not opened again.`);
    else if (already) say(`${list[0]!.name} is already open; switched to it.`);
    if (!keepList) void tick().then(() => grid?.focus());
    // Opening a tab focuses its grid; give the keys back to the list that
    // stays open (Esc closes it, the tick boxes still work).
    else void tick().then(() => document.querySelector<HTMLElement>(".recent-list input")?.focus());
  }

  /** Middle-click on the start page: open the file in a new tab and bring
   *  up the same list as File > Open recent, so more middle-clicks open
   *  more files (the start page itself goes once a file is open). */
  async function startMiddle(e: MouseEvent, r: RecentItem): Promise<void> {
    if (e.button !== 1) return;
    e.preventDefault();
    const got = await openRecent(r);
    if (got !== "opened" && got !== "open") return;
    await loadRecent();
    recentOpen = true;
    void tick().then(() => document.querySelector<HTMLElement>(".recent-list input")?.focus());
  }

  // ---- side panel, views, highlights -------------------------------------------
  function showPanel(which: PanelTab | null): void {
    if (which) panelTab = which;
    prefs.data.panel = which;
    prefs.save();
  }

  function shownTitles(t: Tab): string[] {
    return Array.from({ length: Math.min(t.info.cols, 2000) }, (_, c) => columnTitle(t, c));
  }

  async function toggleReadOnly(): Promise<void> {
    const t = tab;
    if (!t) return;
    const on = !t.info.read_only;
    if (await apply((x) => backend().setReadOnly(x.id, on), false))
      say(on ? "This file is locked: you can look, find and filter, but nothing changes until you unlock it." : "This file is unlocked; you can edit it again.");
  }

  function askFont(): void {
    prompt = {
      title: "Other font",
      label: "Font name",
      value: "",
      note: "Type the name of a font installed on this computer, such as Calibri or Fira Sans.",
      check: (v) => (safeFont(v) ? "" : "Use letters, digits, spaces and hyphens only (up to 60)."),
      done: (v) => {
        const f = customFont(v);
        if (f) {
          prefs.data.font = f;
          prefs.save();
        }
      },
    };
  }

  function setHighlights(rules: View["highlights"]): void {
    const t = tab;
    if (!t) return;
    t.highlights = rules;
    t.highlightVersion++;
    t.bump();
    say(rules.length ? `${plural(rules.length, "highlight rule")} in use. They are kept with saved views, not in the file.` : "No highlight rules.");
  }

  /** The active tab's columns, freeze, sort, filter and highlights. */
  function currentView(t: Tab, name: string): View {
    return {
      version: 1,
      name,
      header: t.header,
      columns: shownTitles(t).map((title, c) => ({
        title,
        width: t.columns.isHidden(c) ? null : Math.round(t.columns.width(c)),
        hidden: t.columns.isHidden(c),
        wrap: t.columns.wraps.has(c),
        format: t.columns.formats.get(c) ?? null,
      })),
      banded: t.banded,
      row_lines: t.rowLines,
      freeze_rows: t.frozenRows,
      freeze_cols: t.frozenCols,
      sort: t.sort ? $state.snapshot(t.sort) : null,
      filter: t.filter ? $state.snapshot(t.filter) : null,
      highlights: $state.snapshot(t.highlights),
      checks: $state.snapshot(t.checks),
    };
  }

  function saveView(): void {
    const t = tab;
    if (!t) return;
    prompt = {
      title: "Save view",
      label: "Name",
      value: "",
      note: "A view keeps column widths and hidden columns, frozen panes, the sort, the filter, highlight rules, data checks, display formats, wrapping and row colours. It works on any file with the same column titles.",
      check: (v) => (!v.trim() ? "Give the view a name." : v.trim().length > 100 ? "Use 100 characters or fewer." : ""),
      done: (name) => {
        const v = currentView(t, name);
        const rest = prefs.data.views.filter((x) => x.name.toLowerCase() !== name.toLowerCase());
        if (rest.length >= MAX_VIEWS) {
          say(`You can keep up to ${MAX_VIEWS} views; forget one first (Alt+Q, then type “forget”).`);
          return;
        }
        prefs.data.views = [...rest, v].sort((a, b) => a.name.localeCompare(b.name));
        prefs.save();
        say(`Saved the view “${name}”. Apply it from View › Apply a saved view.`);
      },
    };
  }

  async function saveViewFile(): Promise<void> {
    const t = tab;
    if (!t) return;
    try {
      const path = await backend().viewSave(currentView(t, t.info.name));
      if (path) say(`Saved the view to ${path}.`);
    } catch (e) {
      say(errorText(e));
    }
  }

  async function openViewFile(): Promise<void> {
    try {
      const v = await backend().viewOpen();
      if (v) await applyView(v);
    } catch (e) {
      say(errorText(e));
    }
  }

  /** Apply a view, matching its columns to this file's by title. */
  async function applyView(v: View): Promise<void> {
    const t = tab;
    if (!t) return;
    if (v.header !== t.header) {
      t.header = v.header;
      t.headerTouched = true;
      t.bump();
    }
    const map = matchColumns(v.columns, shownTitles(t));
    const to = (c: number): number | null => map[c] ?? null;
    let missing = 0;
    v.columns.forEach((vc, k) => {
      const c = map[k];
      if (c === null || c === undefined) {
        missing++;
        return;
      }
      t.columns.setWidth(c, vc.hidden ? 0 : (vc.width ?? DEFAULT_COL_WIDTH));
      if (vc.wrap) t.columns.wraps.add(c);
      else t.columns.wraps.delete(c);
      const f = cleanFormat(vc.format);
      if (f) t.columns.formats.set(c, f);
      else t.columns.formats.delete(c);
    });
    t.banded = v.banded === true;
    t.rowLines = Math.min(MAX_ROW_LINES, Math.max(1, Math.round(v.row_lines ?? 1) || 1));
    t.frozenRows = Math.min(v.freeze_rows, Math.max(0, t.info.rows - 1));
    t.frozenCols = Math.min(v.freeze_cols, Math.max(0, t.info.cols - 1));
    t.highlights = v.highlights.flatMap((h) => {
      const c = to(h.col);
      return c === null ? [] : [{ ...h, col: c }];
    });
    t.highlightVersion++;
    t.checks = (v.checks ?? []).flatMap((c) => {
      if (c.col === null) return [c];
      const k = to(c.col);
      return k === null ? [] : [{ ...c, col: k }];
    });
    t.checksMarked = t.checks.length > 0;
    t.checkVersion++;
    if (v.sort) {
      const keys = v.sort.keys.flatMap((k) => {
        const c = to(k.col);
        return c === null ? [] : [{ ...k, col: c }];
      });
      if (keys.length) await sortRows({ ...v.sort, keys, keep_first: v.header });
    }
    if (v.filter) {
      const columns = v.filter.columns.flatMap((f) => {
        const c = to(f.col);
        return c === null ? [] : [{ ...f, col: c }];
      });
      const spec = { ...v.filter, columns, keep_first: v.header };
      if (columns.length && (await applyJob("Filtering", (x) => backend().setFilter(x.id, spec)))) {
        t.filter = spec;
        t.filterButtons = true;
      }
    } else if (t.info.has_filter) await clearFilter();
    t.bump();
    say(`Applied the view “${v.name}”.${missing ? ` ${plural(missing, "column")} in the view had no match in this file.` : ""}`);
  }

  /** Saved views as commands (for command search). */
  const viewCommands = $derived<Command[]>(
    prefs.data.views.flatMap((v, i) => [
      { id: `view.saved.${i}`, label: `View: ${v.name}`, run: () => void applyView(v), enabled: ready },
      {
        id: `view.forget.${i}`,
        label: `Forget view: ${v.name}`,
        run: () => {
          prefs.data.views = prefs.data.views.filter((x) => x !== v);
          prefs.save();
          say(`Forgot the view “${v.name}”.`);
        },
      },
    ]),
  );

  function runSearched(id: string): void {
    search = null;
    const vc = viewCommands.find((c) => c.id === id);
    if (vc) {
      if (vc.enabled?.() !== false) vc.run();
    } else void runCmd(id);
    // Back to the grid, unless the command opened a dialog of its own.
    void tick().then(() => {
      if (!document.querySelector("[role=dialog]")) grid?.focus();
    });
  }

  function saveShortcuts(custom: Shortcuts): void {
    prefs.data.shortcuts = custom;
    prefs.save();
    reg.setOverrides(custom);
    shortcutsOpen = false;
    say("Shortcuts saved.");
    grid?.focus();
  }

  async function importFile(): Promise<void> {
    try {
      const s = await backend().importFile();
      if (s) importing = s;
    } catch (e) {
      say(errorText(e));
    }
  }

  async function importClipboard(): Promise<void> {
    let text = "";
    try {
      text = await navigator.clipboard.readText();
    } catch {
      say("The clipboard could not be read here.");
      return;
    }
    if (!text) {
      say("The clipboard holds no text.");
      return;
    }
    try {
      importing = await backend().importText(text);
    } catch (e) {
      say(errorText(e));
    }
  }

  /** Import into a new tab, with progress in the status bar. */
  async function finishImport(start: ImportStart, o: ImportOptions, header: boolean): Promise<void> {
    importing = null;
    say(`Importing ${start.name}…`);
    const poll = window.setInterval(async () => {
      const f = await backend().importProgress(start.id).catch(() => null);
      if (f !== null) say(`Importing ${start.name}… ${Math.round(f * 100)}%`);
    }, 300);
    try {
      const d = await backend().importFinish(start.id, o);
      addTab(d, false);
      const nt = tabs[tabs.length - 1];
      if (nt) {
        nt.header = header;
        nt.headerTouched = true;
        nt.bump();
      }
      say(`Imported ${plural(Math.max(0, d.rows - (header ? 1 : 0)), "row")} and ${plural(d.cols, "column")}. It is not saved yet: Save As (F12) writes it as delimited text.`);
    } catch (e) {
      say(errorText(e));
    } finally {
      clearInterval(poll);
      void backend().importClose(start.id).catch(() => {});
    }
  }

  function closeImport(): void {
    const s = importing;
    importing = null;
    if (s) void backend().importClose(s.id).catch(() => {});
    void tick().then(() => grid?.focus());
  }

  /** The first rows of a document, for the Save As preview. */
  async function sampleRows(t: Tab): Promise<string[][]> {
    try {
      const v = await backend().viewport(t.id, 0, Math.min(6, t.info.rows), 0, Math.min(t.info.cols, 30), 80);
      const rows: string[][] = [];
      for (let r = 0; r < v.rows; r++) rows.push(v.cells.slice(r * v.cols, (r + 1) * v.cols));
      return rows;
    } catch {
      return [];
    }
  }

  /** Show the Save As options screen; null when cancelled. */
  async function askSaveOptions(t: Tab): Promise<SaveAsOptions | null> {
    const sample = await sampleRows(t);
    return new Promise((resolve) => {
      saveAsReq = {
        sample,
        done: (o) => {
          saveAsReq = null;
          resolve(o);
        },
      };
    });
  }

  async function exportRun(o: ExportOptions, to: "file" | "clipboard"): Promise<void> {
    exportOpen = false;
    const t = tab;
    if (!t) return;
    try {
      if (to === "clipboard") {
        const text = await runJob("Exporting", () => backend().exportText(t.id, o));
        if (text === null) return;
        await navigator.clipboard.writeText(text);
        say("Copied the export to the clipboard.");
      } else {
        const r = await runJob("Exporting", () => backend().exportFile(t.id, o));
        if (!r) return;
        say(`Exported ${plural(r.rows, "row")}${r.sheets > 1 ? ` on ${plural(r.sheets, "sheet")}` : ""}.`);
      }
    } catch (e) {
      say(errorText(e));
    } finally {
      void tick().then(() => grid?.focus());
    }
  }

  /** Column titles of an open document (first row, or "Column N"). */
  async function titlesOf(id: number, header: boolean): Promise<string[]> {
    const t = tabs.find((x) => x.id === id);
    const cols = Math.min(t?.info.cols ?? 1, 2000);
    let first: string[] = [];
    if (header) {
      try {
        const v = await backend().viewport(id, 0, 1, 0, cols, 60);
        first = v.cells.slice(0, v.cols);
      } catch {
        // Numbers only.
      }
    }
    return Array.from({ length: cols }, (_, c) => (first[c] || `Column ${c + 1}`).replace(/[\r\n\t]+/g, " "));
  }

  function otherTables(): OtherTable[] {
    return tabs.map((t) => ({ id: t.id, name: t.info.name, header: t.header, cols: t.info.cols }));
  }

  /** Open a file for a join or append, staying on the current tab. */
  async function openOther(): Promise<OtherTable | null> {
    const keep = active;
    try {
      const o = await backend().openFile();
      let d: DocInfo | null = o?.doc ?? null;
      if (!d && o?.import) {
        const s = o.import;
        d = await backend().importFinish(s.id, { format: s.info.format, part: Math.max(0, s.info.hidden.indexOf(false)), raw_dates: false, record: "" });
        void backend().importClose(s.id).catch(() => {});
      }
      if (!d) return null;
      addTab(d);
      const nt = tabs[tabs.length - 1]!;
      // Wait for the file to load, so its columns are known.
      for (let i = 0; i < 600 && !nt.info.indexed; i++) {
        await new Promise((r) => setTimeout(r, 100));
        nt.update(await backend().docInfo(nt.id), false);
      }
      await new Promise((r) => setTimeout(r, 50));
      active = keep;
      return { id: nt.id, name: nt.info.name, header: nt.header, cols: nt.info.cols };
    } catch (e) {
      say(errorText(e));
      active = keep;
      return null;
    }
  }

  async function newFile(): Promise<void> {
    try {
      addTab(await backend().newDocument());
    } catch (e) {
      say(errorText(e));
    }
  }

  /** Save, answering the engine's questions with the user. */
  async function save(saveAs: boolean): Promise<boolean> {
    const t = tab;
    if (!t) return false;
    if (saveAs && !(await privacyGate("Save As writes another copy of all of it."))) return false;
    if (!t.info.indexed) {
      say("The file is still loading.");
      return false;
    }
    if (grid?.isEditing() && !(await grid.commitEdit())) return false;
    // A tab from an encrypted file saves encrypted unless the user says
    // plainly that a readable copy is wanted.
    const origin = await backend().encryptedOrigin(t.id).catch(() => null);
    if (origin) {
      const a = await ask(
        "Keep it encrypted?",
        `This tab came from the encrypted file ${origin}. A normal save writes a plain file that anyone can read.`,
        [
          { id: "enc", label: "Save encrypted…", primary: true },
          { id: "plain", label: "Save a plain copy anyone can read…", danger: true },
          { id: "cancel", label: "Cancel" },
        ],
        "cancel",
      );
      if (a === "cancel") return false;
      if (a === "enc") {
        encryptSame = origin;
        encryptMode = "save";
        return false;
      }
      saveAs = true;
    }
    if (t.info.line_by_line && !saveAs) {
      const a = await ask(
        "Save a file opened line by line?",
        "This file was opened reading every line as its own row. Any cell that really spanned several lines is now split across rows, and saving over the original makes that split permanent.",
        [
          { id: "as", label: "Save As a new file…", primary: true },
          { id: "save", label: "Save over the original", danger: true },
          { id: "cancel", label: "Cancel" },
        ],
        "cancel",
      );
      if (a === "cancel") return false;
      if (a === "as") saveAs = true;
    }
    let overwriteChanged = false;
    let allowLossy = false;
    let options: SaveAsOptions | null = null;
    t.busy = "Saving…";
    try {
      for (;;) {
        try {
          if ((saveAs || !t.info.path) && !options) {
            t.busy = null;
            options = await askSaveOptions(t);
            if (!options) return false;
            t.busy = "Saving…";
            saveAs = true;
          }
          const d = await backend().save(t.id, { saveAs, overwriteChanged, allowLossy, options });
          if (!d) return false; // dialog cancelled
          t.update(d, true);
          say(`Saved ${d.name}.`);
          return true;
        } catch (e) {
          if (!isCmdError(e)) throw e;
          if (e.kind === "changed_on_disk") {
            const a = await ask(
              "The file changed on disk",
              "Another program changed this file after you opened it. Saving now replaces their changes with yours.",
              [
                { id: "overwrite", label: "Replace their changes", danger: true },
                { id: "as", label: "Save As a new file…", primary: true },
                { id: "cancel", label: "Cancel" },
              ],
              "cancel",
            );
            if (a === "cancel") return false;
            if (a === "as") saveAs = true;
            else overwriteChanged = true;
          } else if (e.kind === "lossy") {
            const where = e.row !== null ? ` The first one is in row ${e.row + 1}.` : "";
            const a = await ask(
              "Some characters could not be read",
              `Parts of this file could not be read in its encoding and show as �. Saving writes them as � (or ?), which permanently replaces the original bytes.${where}`,
              [
                { id: "lossy", label: "Save anyway", danger: true },
                { id: "cancel", label: "Cancel", primary: true },
              ],
              "cancel",
            );
            if (a !== "lossy") return false;
            allowLossy = true;
          } else if (e.kind === "no_path") {
            saveAs = true;
          } else {
            await ask("Could not save", e.message, [{ id: "ok", label: "OK", primary: true }], "ok");
            return false;
          }
        }
      }
    } catch (e) {
      say(errorText(e));
      return false;
    } finally {
      t.busy = null;
    }
  }

  async function closeTab(i = active): Promise<boolean> {
    const t = tabs[i];
    if (!t) return false;
    if (t.info.modified) {
      active = i;
      const a = await ask(
        `Save changes to ${t.info.name}?`,
        "Your changes will be lost if you don't save them.",
        [
          { id: "save", label: "Save", primary: true },
          { id: "discard", label: "Don't save", danger: true },
          { id: "cancel", label: "Cancel" },
        ],
        "cancel",
      );
      if (a === "cancel") return false;
      if (a === "save" && !(await save(false))) return false;
    }
    await backend().closeDocument(t.id).catch(() => {});
    if ((t.private.file || t.private.folder) && copiedFrom === t.id) {
      await navigator.clipboard.writeText("").catch(() => {});
      copiedFrom = null;
    }
    tabs.splice(i, 1);
    active = Math.min(active, Math.max(0, tabs.length - 1));
    return true;
  }

  async function quit(): Promise<void> {
    const unsaved = tabs.filter((t) => t.info.modified);
    if (unsaved.length) {
      const a = await ask(
        "Quit without saving?",
        `${unsaved.length === 1 ? "1 file has" : `${unsaved.length} files have`} unsaved changes: ${unsaved.map((t) => t.info.name).join(", ")}.`,
        [
          { id: "quit", label: "Quit and lose changes", danger: true },
          { id: "cancel", label: "Cancel", primary: true },
        ],
        "cancel",
      );
      if (a !== "quit") return;
    }
    await backend().quitApp();
  }

  /** Document whose "File layout" dialog is open. */
  let layoutFor = $state<{ doc: number; modified: boolean } | null>(null);

  function chooseLayout(): void {
    const t = tab;
    if (t?.info.path) layoutFor = { doc: t.id, modified: t.info.modified };
  }

  async function reopenWithLayout(layout: Layout): Promise<void> {
    layoutFor = null;
    const t = tab;
    if (!t) return;
    await reopenTab(t, t.info.line_by_line, layout, true);
  }

  async function reopenLineByLine(): Promise<void> {
    const t = tab;
    if (!t) return;
    if (t.info.modified) {
      const a = await ask(
        "Reopen and lose changes?",
        "Reopening reads the file again from disk; changes you have not saved are dropped.",
        [
          { id: "go", label: "Reopen", danger: true },
          { id: "cancel", label: "Cancel", primary: true },
        ],
        "cancel",
      );
      if (a !== "go") return;
    }
    // Keep a layout the user chose; otherwise detect again.
    const keep: Layout | null =
      t.info.layout_confidence === null
        ? { delimiter: t.info.delimiter, quote: t.info.quote, escape: t.info.escape, merge: t.info.merge, skip_lines: t.info.preamble_lines }
        : null;
    await reopenTab(t, !t.info.line_by_line, keep);
  }

  /** Replace tab `t` with its file opened again. */
  async function reopenTab(t: Tab, lineByLine: boolean, layout: Layout | null, fresh = false): Promise<void> {
    try {
      const d = await backend().reopen(t.id, lineByLine, layout);
      const i = tabs.indexOf(t);
      if (i < 0) {
        await backend().closeDocument(d.id).catch(() => {});
        return;
      }
      await backend().closeDocument(t.id).catch(() => {});
      const nt = new Tab(d, DEFAULT_COL_WIDTH);
      // Another layout means other columns: guess the header row afresh.
      nt.header = fresh ? false : t.header;
      nt.headerTouched = !fresh;
      tabs[i] = nt;
      active = i;
      if (fresh) void tick().then(() => guessHeader(nt));
    } catch (e) {
      say(errorText(e));
    }
  }

  // ---- editing ----------------------------------------------------------------------
  async function commitCell(row: number, col: number, value: string, fill: boolean): Promise<boolean> {
    const t = tab;
    if (!t) return false;
    if (fill) {
      const r = t.rect;
      if (cellCount(r) > MAX_CELLS_PER_CHANGE) {
        say(`One change is limited to ${MAX_CELLS_PER_CHANGE.toLocaleString()} cells.`);
        return false;
      }
      return apply((x) => backend().fillCells(x.id, r.row, r.rows, r.col, r.cols, value));
    }
    if (value === t.cache.get(row, col) && !t.cache.isTruncated(row, col)) return true;
    return apply((x) => backend().setCells(x.id, [[row, col, value]]));
  }

  async function clearSelection(): Promise<void> {
    const t = tab;
    if (!t) return;
    const r = t.rect;
    if (cellCount(r) > MAX_CELLS_PER_CHANGE) {
      say(`Clearing is limited to ${MAX_CELLS_PER_CHANGE.toLocaleString()} cells at once for now. To remove whole rows or columns, use Delete (Ctrl+-).`);
      return;
    }
    await apply((x) => backend().fillCells(x.id, r.row, r.rows, r.col, r.cols, ""));
  }

  async function copy(cut: boolean): Promise<void> {
    const t = tab;
    if (!t) return;
    const r = t.rect;
    // Cut works as in Excel and Google Sheets: nothing changes until Paste,
    // which moves what was cut there (whole columns or rows go in before the
    // one picked, and the gap they leave closes up).
    const kind = !cut ? null : wholeCols(r, t) && !wholeRows(r, t) ? "cols" : wholeRows(r, t) ? "rows" : "cells";
    if (kind && (t.info.read_only || !t.info.indexed)) {
      say(t.info.read_only ? "This file is read-only, so nothing can be cut from it. Copy (Ctrl+C) works." : "Please wait until the file has been read.");
      return;
    }
    const epoch = ++cutEpoch;
    const pos = t.info.history_pos;
    for (const x of tabs) {
      if (x.cut) {
        x.cut = null;
        x.bump();
      }
    }
    const what =
      kind === "cols" ? `${r.cols === 1 ? "1 column" : `${r.cols.toLocaleString()} columns`}` : kind === "rows" ? `${r.rows === 1 ? "1 row" : `${r.rows.toLocaleString()} rows`}` : `${cellCount(r).toLocaleString()} cell${cellCount(r) === 1 ? "" : "s"}`;
    // Many whole rows or columns cut to move them: the move never needs the
    // clipboard, so their data stays inside the app (no privacy question);
    // a short note goes on the clipboard instead, so a later paste of
    // something copied elsewhere is never taken for this cut.
    const inside = kind === "cols" || kind === "rows" ? cellCount(r) >= LARGE_COPY : false;
    let clip: string;
    if (inside) {
      clip = `Natendo Data Edit: ${what} marked to move. Paste them inside the app with Ctrl+V.`;
      await navigator.clipboard.writeText(clip).catch(() => {});
    } else {
      try {
        if (cellCount(r) >= LARGE_COPY && !(await privacyGate("The copy goes to the clipboard, where other programs can read it.", [r.col, r.cols]))) return;
        clip = await backend().copyCells(t.id, r.row, r.rows, r.col, r.cols);
        await navigator.clipboard.writeText(clip);
        copiedFrom = t.id;
      } catch (e) {
        say(errorText(e));
        return;
      }
    }
    if (!kind) {
      say(`Copied ${what}.`);
      return;
    }
    // Esc (or another change) while the copy was being made: no cut.
    if (epoch !== cutEpoch || t.info.history_pos !== pos) return;
    t.cut = { kind, rect: r, pos, clip, inside };
    t.bump();
    const one = kind === "cols" ? r.cols === 1 : kind === "rows" ? r.rows === 1 : cellCount(r) === 1;
    const it = one ? "it" : "they";
    const where = kind === "cols" ? `the column ${it} should go before` : kind === "rows" ? `the row ${it} should go above` : `where ${it} should go`;
    say(`Cut ${what}${inside ? " (kept inside the app, not put on the clipboard)" : ""}. Click ${where} and press Ctrl+V to move ${one ? "it" : "them"}; Esc cancels.`);
  }

  /** Drop what Cut marked (Esc, or the right-click menu). */
  function cancelCut(): void {
    cutEpoch++;
    const t = tab;
    if (!t?.cut) return;
    t.cut = null;
    t.bump();
    say("Cut cancelled; nothing was moved.");
  }
  /** Bumped by Esc, so a cut still being copied is not marked. */
  let cutEpoch = 0;
  /** Said once a paste finishes (a copy pasted from a cut in another file). */
  let afterPaste: string | null = null;

  /** Clipboard text compared loosely (line ends, a trailing line break). */
  const sameClip = (s: string): string => s.replace(/\r\n?/g, "\n").replace(/\n+$/, "");

  /** Paste after a Cut in this tab: move what was cut. Returns false when
   *  the paste is something else (another copy since, or nothing cut). */
  async function pasteCut(text: string | null): Promise<boolean> {
    const t = tab;
    const c = t?.cut;
    if (!t || !c) return false;
    if (c.pos !== t.info.history_pos || (text !== null && sameClip(text) !== sameClip(c.clip))) {
      t.cut = null;
      t.bump();
      return false;
    }
    t.cut = null;
    t.bump();
    const r = t.rect;
    const from = c.rect;
    if (c.kind === "cols") {
      const to = dropIndex(from.col, from.cols, r.col, t.info.cols);
      if (to === null) {
        say("Pick a column outside the cut ones, then paste: the cut columns go in before it.");
        return true;
      }
      await dragCols(from.col, from.cols, to);
    } else if (c.kind === "rows") {
      const to = dropIndex(from.row, from.rows, r.row, t.info.rows, t.firstRow);
      if (to === null) {
        say("Pick a row outside the cut ones, then paste: the cut rows go in above it.");
        return true;
      }
      await dragRows(from.row, from.rows, to);
    } else {
      if (r.row + from.rows > t.info.rows || r.col + from.cols > t.info.cols) {
        say("There is not room for the cut cells there: they would go past the end of the table.");
        return true;
      }
      if (t.columns.hiddenIn(r.col, r.col + from.cols) || t.columns.hiddenIn(from.col, from.col + from.cols)) {
        say("Cut cells can't be moved over or out of hidden columns. Show them first (Ctrl+Shift+0).");
        return true;
      }
      if (r.row === from.row && r.col === from.col) return true;
      await dragCells(from, r.row, r.col, false);
    }
    return true;
  }

  async function pasteText(text: string): Promise<void> {
    const t = tab;
    if (!t || !ready()) return;
    if (await pasteCut(text)) return;
    // Cut in another file: paste a copy here and leave those cells where
    // they are (a move between two files would be two undo steps).
    const other = tabs.find((x) => x !== t && x.cut && sameClip(x.cut.clip) === sameClip(text));
    if (other?.cut) {
      const inside = other.cut.inside;
      other.cut = null;
      other.bump();
      if (inside) {
        say(`What was cut in ${other.info.name} can only be moved inside that file. To copy it here, copy fewer rows or columns.`);
        return;
      }
      afterPaste = `Pasted a copy here. The cut cells in ${other.info.name} stay where they were.`;
    }
    const rows = parseTsv(text);
    if (rows.length === 0) return;
    const r = t.rect;
    const width = Math.max(...rows.map((x) => x.length));
    if (rows.length === 1 && width === 1 && cellCount(r) > 1 && cellCount(r) <= MAX_CELLS_PER_CHANGE) {
      await commitCell(r.row, r.col, rows[0]![0] ?? "", true);
      if (afterPaste) say(afterPaste);
      afterPaste = null;
      return;
    }
    // The engine parses the text again and writes it as one undo step,
    // adding rows past the end if needed.
    const done = await runOp({ op: "paste", spec: { row: r.row, col: r.col, op: "none", skip_blanks: false, transpose: false }, text }, "Pasting");
    if (done) grid?.select({ row: r.row, col: r.col }, { row: r.row + rows.length - 1, col: r.col + width - 1 });
    if (done && afterPaste) say(afterPaste);
    afterPaste = null;
  }

  async function pasteFromMenu(): Promise<void> {
    let text: string;
    try {
      text = await navigator.clipboard.readText();
    } catch {
      // Something cut here can still be moved without the clipboard.
      if (tab?.cut && ready() && (await pasteCut(null))) return;
      say("Paste from the menu is not allowed here; press Ctrl+V instead.");
      return;
    }
    await pasteText(text);
  }

  /** Ctrl+D / Ctrl+R (and fill up / left): copy the first row (column)
   *  of the selection over the rest; one cell selected copies the one above
   *  (left of) it, as in Excel. */
  async function fill(dir: Dir): Promise<void> {
    const t = tab;
    if (!t) return;
    const r = { ...t.rect };
    if (dir === "down" && r.rows === 1 && r.row > t.firstRow) {
      r.row -= 1;
      r.rows = 2;
    }
    if (dir === "right" && r.cols === 1 && r.col > 0) {
      r.col -= 1;
      r.cols = 2;
    }
    const vertical = dir === "down" || dir === "up";
    const spec: FillSpec = {
      row: dir === "up" ? r.row + r.rows - 1 : r.row,
      col: dir === "left" ? r.col + r.cols - 1 : r.col,
      rows: vertical ? 1 : r.rows,
      cols: vertical ? r.cols : 1,
      dir,
      count: vertical ? r.rows - 1 : r.cols - 1,
      mode: { kind: "copy" },
    };
    if (spec.count > 0) await runOp({ op: "fill", spec }, "Filling");
  }

  /** The fill handle was dragged. */
  async function fillHandle(dir: Dir, count: number, swap: boolean): Promise<void> {
    const t = tab;
    if (!t || !ready()) return;
    const r = { ...t.rect };
    const spec: FillSpec = { row: r.row, col: r.col, rows: r.rows, cols: r.cols, dir, count, mode: { kind: swap ? "auto_swap" : "auto" } };
    const rep = await runOp({ op: "fill", spec }, "Filling");
    if (!rep) return;
    const last = { row: r.row + r.rows - 1, col: r.col + r.cols - 1 };
    if (dir === "down") grid?.select({ row: r.row, col: r.col }, { row: last.row + count, col: last.col }, false);
    else if (dir === "up") grid?.select({ row: last.row, col: last.col }, { row: Math.max(t.firstRow, r.row - count), col: r.col }, false);
    else if (dir === "right") grid?.select({ row: r.row, col: r.col }, { row: last.row, col: last.col + count }, false);
    else grid?.select({ row: last.row, col: last.col }, { row: r.row, col: Math.max(0, r.col - count) }, false);
  }

  /** Double-click on the fill handle: fill down as far as the column next
   *  to the selection has data. */
  async function fillHandleAuto(): Promise<void> {
    const t = tab;
    if (!t || !ready()) return;
    const r = t.rect;
    const bottom = r.row + r.rows - 1;
    if (bottom + 1 >= t.info.rows) return;
    const sides = [r.col - 1, r.col + r.cols].filter((c) => c >= 0 && c < t.info.cols);
    let end = bottom;
    try {
      for (const c of sides) {
        const below = await backend().copyCells(t.id, bottom + 1, 1, c, 1);
        if ((parseTsv(below)[0]?.[0] ?? "") === "") continue;
        const [er] = await backend().dataEdge(t.id, bottom + 1, c, "down");
        end = Math.max(end, er);
        break;
      }
    } catch (e) {
      say(errorText(e));
      return;
    }
    if (end > bottom) await fillHandle("down", end - bottom, false);
    else say("Nothing next to the selection to fill alongside; drag the fill handle instead.");
  }

  /** Run a data operation as a long job; returns its report. */
  async function runOp(op: DataOp, label: string): Promise<OpReport | null> {
    const t = tab;
    if (!t) return null;
    try {
      const rep = await runJob(label, () => backend().dataOp(t.id, op));
      if (!rep) return null;
      if (rep.info) t.update(rep.info, true);
      return rep;
    } catch (e) {
      say(errorText(e));
      return null;
    }
  }

  const plural = (n: number, one: string, many = `${one}s`) => `${n.toLocaleString()} ${n === 1 ? one : many}`;

  /** Run a cleaning tool or a cleaner and say what happened. */
  async function runClean(spec: CleanSpec, header: boolean, what: string): Promise<void> {
    const t = tab;
    if (!t) return;
    if (header !== t.header && spec.scope.kind !== "range") {
      t.header = header;
      t.headerTouched = true;
      t.bump();
    }
    const rep = await runOp({ op: "clean", spec, now: localNow() }, "Cleaning");
    if (!rep) return;
    const left =
      (rep.left ? ` ${plural(rep.left, "cell")} left as they were (not a date or number).` : "") +
      (rep.other_order ? ` ${plural(rep.other_order, "date")} could only be read the other way round (day and month swapped): the column may mix the two orders; check the result.` : "");
    say(rep.cells ? `${what}: changed ${plural(rep.cells, "cell")}.${left} Undo with Ctrl+Z.` : `${what}: nothing to change.${left}`);
  }

  async function runSplitName(op: DataOp, header: boolean): Promise<void> {
    const t = tab;
    if (!t) return;
    if (header !== t.header) {
      t.header = header;
      t.headerTouched = true;
      t.bump();
    }
    const rep = await runOp(op, "Splitting names");
    if (rep) say(`Split the names into ${plural(rep.cols, "new column")}.`);
  }

  /** Fill the column's empty cells from the results typed in it (Ctrl+E). */
  async function fillByExample(): Promise<void> {
    const t = tab;
    if (!t) return;
    const col = t.sel.focus.col;
    const rep = await runOp({ op: "fill_example", spec: { col, skip_first: t.header } }, "Filling by example");
    if (!rep) return;
    const from = `from ${plural(rep.examples ?? 0, "example")}`;
    const missed = rep.left ? ` ${plural(rep.left, "row")} left empty (a piece the pattern needs is missing).` : "";
    say(rep.cells ? `Filled ${plural(rep.cells, "cell")} ${from}: ${rep.pattern ?? ""}.${missed}` : `Nothing to fill ${from}.${missed}`);
  }

  /** Run what a data dialog asked for and say what happened. */
  async function runDialog(r: { data?: DataOp; table?: TableOp }, header: boolean): Promise<void> {
    const t = tab;
    if (!t) return;
    if (header !== t.header) {
      t.header = header;
      t.headerTouched = true;
      t.bump();
    }
    if (r.table) {
      const req = r.table;
      try {
        const d = await runJob("Building the table", () => backend().newTable(t.id, req));
        if (!d) return;
        addTab(d, false);
        const nt = tabs[tabs.length - 1];
        if (nt) {
          nt.header = true;
          nt.headerTouched = true;
          nt.bump();
        }
        say(`The new table has ${plural(Math.max(0, d.rows - 1), "row")} and ${plural(d.cols, "column")}. It is not saved yet (Ctrl+S).`);
      } catch (e) {
        say(errorText(e));
      }
      return;
    }
    const op = r.data;
    if (!op) return;
    const rep = await runOp(op, op.op === "fill" ? "Filling" : op.op === "paste" ? "Pasting" : "Working");
    if (!rep) return;
    switch (op.op) {
      case "paste":
        say(`Changed ${plural(rep.cells, "cell")}.${rep.skipped ? ` ${plural(rep.skipped, "cell")} left as they were (not numbers, or division by zero).` : ""}`);
        break;
      case "fill":
        say(`Filled ${plural(rep.cells, "cell")}.`);
        break;
      case "show_duplicates":
        t.filter = null;
        if (rep.rows) say(`${plural(rep.rows, "row")} in ${plural(rep.groups, "group")} have duplicates. Click “Filtered ✕” to show every row again.`);
        else {
          say("No duplicates found.");
          void clearFilter();
        }
        break;
      case "remove_duplicates":
        say(
          (rep.rows ? `Removed ${plural(rep.rows, "duplicate row")} (${plural(rep.groups, "group")}). Undo (Ctrl+Z) brings them back.` : "No duplicates found.") +
            (rep.left ? ` ${plural(rep.left, "copy", "copies")} had a date that could not be read and were not kept over a copy with one.` : ""),
        );
        break;
      case "split_column":
        if (rep.cols) t.columns.shift(op.spec.col + 1, rep.cols);
        say(rep.cols ? `Split into ${plural(rep.cols, "new column")}.` : "No cell contains that separator.");
        break;
      case "merge_columns":
        t.columns.shift(op.spec.col + 1, -(op.spec.count - 1));
        say(`Merged ${plural(op.spec.count, "column")}.`);
        break;
      case "split_rows":
        say(rep.rows ? `Added ${plural(rep.rows, "row")}.` : "No cell contains that separator.");
        break;
      case "merge_rows":
        say(rep.rows ? `Merged ${plural(rep.rows + rep.groups, "row")} into ${plural(rep.groups, "row")}.` : "No rows share those values.");
        break;
      case "join":
      case "append":
        if (op.op === "join" && rep.join?.cols && op.spec.place === "after_key") t.columns.shift(Math.max(...op.spec.keys) + 1, rep.join.cols);
        if (rep.join) say(`${joinSummary(op.op, rep.join, op.op === "join" ? op.spec : undefined)} Undo (Ctrl+Z) takes it back.`);
        break;
      case "map_values":
        say(rep.cells ? `Changed ${plural(rep.cells, "cell")}. Undo (Ctrl+Z) takes it back.` : "No cell held those values.");
        break;
      case "group_calc":
        t.columns.shift(op.spec.at ?? t.info.cols - 1, 1);
        say(`Added a column with figures for ${plural(rep.groups, "group")}. Undo (Ctrl+Z) takes it back.`);
        break;
      case "extract":
        t.columns.shift(op.spec.col + 1, 1);
        say(rep.cells ? `Extracted into a new column (${plural(rep.cells, "cell")} written). Undo (Ctrl+Z) takes it back.` : "Nothing like that was found; an empty column was added. Undo (Ctrl+Z) takes it back.");
        break;
      case "arrange":
        t.columns.reset();
        say(`Kept ${plural(op.spec.order.length, "column")}${rep.cols ? `, deleted ${plural(rep.cols, "column")}` : ""}. Undo (Ctrl+Z) takes it back.`);
        break;
      case "compute":
        say(`Built a new column (${plural(rep.cells, "cell")}). The values are saved as text. Undo (Ctrl+Z) takes it back.`);
        break;
      default:
        break;
    }
  }

  async function removeBlanks(cols: boolean): Promise<void> {
    const t = tab;
    if (!t) return;
    const spec = { spaces: true, keep_first: t.header };
    const rep = await runOp(cols ? { op: "remove_blank_cols", spec } : { op: "remove_blank_rows", spec }, "Looking for blanks");
    if (!rep) return;
    if (cols) say(rep.cols ? `Removed ${plural(rep.cols, "blank column")}.` : "No blank columns.");
    else say(rep.rows ? `Removed ${plural(rep.rows, "blank row")}.` : "No blank rows.");
  }

  async function sortColumns(descending: boolean): Promise<void> {
    const t = tab;
    if (!t) return;
    if (!t.header) {
      say("Sorting columns uses their titles: turn on Format > First row is header first.");
      return;
    }
    const r = t.rect;
    const whole = r.cols <= 1;
    const rep = await runOp({ op: "sort_columns", col: whole ? 0 : r.col, count: whole ? 0 : r.cols, descending }, "Sorting columns");
    if (rep) say(rep.cols ? "Columns sorted by their titles. Undo (Ctrl+Z) puts them back." : "The columns were already in that order.");
  }

  /** Open the pivot builder. On a pivot made earlier, it opens on the
   *  tab that pivot came from, with that pivot's layout. */
  function openPivot(): void {
    const t = tab;
    if (!t) return;
    const from = pivotOf.get(t.id);
    const src = from ? tabs.findIndex((x) => x.id === from.source) : -1;
    if (from && src >= 0) {
      pivotSpecs.set(from.source, from.spec);
      active = src;
      pivotDoc = from.source;
      return;
    }
    pivotDoc = t.id;
  }

  function pivotTab(): Tab | undefined {
    return tabs.find((x) => x.id === pivotDoc);
  }

  /** A few values of a column (to guess whether to add it up or count it). */
  function pivotSample(t: Tab, c: number): string[] {
    const out: string[] = [];
    for (let r = t.header ? 1 : 0; r < Math.min(t.info.rows, 80); r++) {
      const v = t.cache.get(r, c);
      if (v !== undefined) out.push(v);
    }
    return out;
  }

  /** Saving a layout with picked items writes values from the file. */
  async function pivotSaveAsk(t: Tab, items: number): Promise<boolean> {
    const priv = t.private.file || t.private.folder;
    const id = await ask(
      "Save the picked items?",
      `This layout keeps the ${plural(items, "item")} you picked (or left out), which are values from this file. Anyone who gets the layout file can read them.`,
      [
        { id: "save", label: "Save anyway", primary: !priv, danger: priv },
        { id: "cancel", label: "Cancel", primary: priv },
      ],
      "cancel",
      priv ? "This file is in private file mode. Use “Use all items” first to save the layout without them." : undefined,
    );
    return id === "save";
  }

  function pivotDone(spec: PivotSpec): void {
    const id = pivotDoc;
    if (id === null) return;
    pivotSpecs.set(id, spec);
    pivotDoc = null;
    void backend().pivotClose(id).catch(() => {});
    void tick().then(() => grid?.focus());
  }

  async function pivotToTab(spec: PivotSpec): Promise<void> {
    const id = pivotDoc;
    const t = pivotTab();
    if (id === null || !t) return;
    pivotDone(spec);
    try {
      const d = await runJob("Building the pivot table", () => backend().pivotTable(id, spec));
      if (!d) return;
      addTab(d, false);
      const nt = tabs[tabs.length - 1];
      if (nt) {
        nt.header = true;
        nt.headerTouched = true;
        nt.bump();
        pivotOf.set(nt.id, { source: id, spec });
      }
      say(`The pivot table has ${plural(d.rows, "row")} and ${plural(d.cols, "column")}. It is not saved yet (Ctrl+S). Data › Pivot table on it changes the layout.`);
    } catch (e) {
      say(errorText(e));
    }
  }

  async function pivotCopy(spec: PivotSpec): Promise<void> {
    const id = pivotDoc;
    if (id === null) return;
    try {
      if (!(await privacyGate("The copy goes to the clipboard, where other programs can read it."))) return;
      const text = await backend().pivotText(id, spec);
      await navigator.clipboard.writeText(text);
      copiedFrom = id;
      say("Copied the pivot table; paste it into a spreadsheet or document.");
    } catch (e) {
      say(errorText(e));
    }
  }

  async function pivotDrill(spec: PivotSpec, rowPath: string[], colPath: string[]): Promise<void> {
    const id = pivotDoc;
    if (id === null) return;
    pivotDone(spec);
    try {
      const d = await runJob("Finding the rows", () => backend().pivotDrill(id, spec, rowPath, colPath));
      if (!d) return;
      addTab(d, false);
      const nt = tabs[tabs.length - 1];
      if (nt) {
        nt.header = true;
        nt.headerTouched = true;
        nt.bump();
      }
      say(`${plural(Math.max(0, d.rows - 1), "row")} make up that number; they are in a new tab (not saved).`);
    } catch (e) {
      say(errorText(e));
    }
  }

  async function openOp(kind: OpKind): Promise<void> {
    let clip = "";
    if (kind === "paste_special") {
      try {
        clip = await navigator.clipboard.readText();
      } catch {
        // Not allowed here: the dialog has a box to paste into.
      }
    }
    opDialog = { kind, clip };
  }

  /** Ctrl+arrow: jump to the edge of the data (Excel's rule). */
  async function jump(dir: Dir, extend: boolean): Promise<void> {
    const t = tab;
    if (!t || !grid) return;
    const f = t.sel.focus;
    let target: [number, number];
    try {
      target = await backend().dataEdge(t.id, f.row, f.col, dir);
    } catch {
      const last = Math.max(t.firstRow, t.info.rows - 1);
      target = dir === "up" ? [t.firstRow, f.col] : dir === "down" ? [last, f.col] : dir === "left" ? [f.row, 0] : [f.row, Math.max(0, t.info.cols - 1)];
    }
    if (t !== tab || !grid) return;
    const cell = { row: Math.max(t.firstRow, target[0]), col: target[1] };
    if (extend) grid.select(t.sel.anchor, cell);
    else grid.select(cell);
  }

  function openSymbols(): void {
    symbols = { caret: grid?.editorCaret() ?? null };
  }

  async function change(c: Change, dataChanged = true): Promise<boolean> {
    return apply((x) => backend().change(x.id, c), dataChanged);
  }

  async function insertRows(below: boolean, howMany?: number): Promise<void> {
    const t = tab;
    if (!t) return;
    const r = t.rect;
    const at = below ? r.row + r.rows : r.row;
    const count = howMany ?? r.rows;
    if (await change({ op: "insert_rows", at, count })) {
      grid?.select({ row: at, col: r.col }, { row: at + count - 1, col: r.col + r.cols - 1 });
    }
  }

  async function deleteRows(): Promise<void> {
    const t = tab;
    if (!t) return;
    const r = t.rect;
    const count = Math.min(r.rows, t.info.rows - r.row);
    if (count <= 0) return;
    if (await change({ op: "delete_rows", at: r.row, count })) grid?.select({ row: r.row, col: r.col });
  }

  async function insertCols(right: boolean, howMany?: number): Promise<void> {
    const t = tab;
    if (!t) return;
    const r = t.rect;
    const at = right ? r.col + r.cols : r.col;
    const count = howMany ?? r.cols;
    if (await change({ op: "insert_cols", at, count })) {
      t.columns.shift(at, count);
      grid?.select({ row: r.row, col: at }, { row: r.row + r.rows - 1, col: at + count - 1 });
    }
  }

  async function deleteCols(): Promise<void> {
    const t = tab;
    if (!t) return;
    const r = t.rect;
    if (r.cols >= t.info.cols) {
      say("At least one column must remain.");
      return;
    }
    if (await change({ op: "delete_cols", at: r.col, count: r.cols })) {
      t.columns.shift(r.col, -r.cols);
      grid?.select({ row: r.row, col: Math.min(r.col, t.info.cols - 1) });
      if (t.info.filter_on_deleted) {
        say("The deleted column had a filter, so some rows stay hidden. Undo brings the column back, or click Filtered ✕ to show every row.");
      }
    }
  }

  /** Excel's Ctrl++ / Ctrl+-: whole columns selected act on columns, otherwise rows. */
  async function insertSmart(): Promise<void> {
    const t = tab;
    if (t && wholeCols(t.rect, t) && !wholeRows(t.rect, t)) await insertCols(false);
    else await insertRows(false);
  }
  async function deleteSmart(): Promise<void> {
    const t = tab;
    if (t && wholeCols(t.rect, t) && !wholeRows(t.rect, t)) await deleteCols();
    else await deleteRows();
  }

  async function moveRows(dir: -1 | 1): Promise<void> {
    const t = tab;
    if (!t) return;
    const r = t.rect;
    const to = r.row + dir;
    if (to < t.firstRow || to + r.rows > t.info.rows) return;
    if (await change({ op: "move_rows", at: r.row, count: r.rows, to })) {
      grid?.select({ row: t.sel.anchor.row + dir, col: t.sel.anchor.col }, { row: t.sel.focus.row + dir, col: t.sel.focus.col });
    }
  }

  async function moveCols(dir: -1 | 1): Promise<void> {
    const t = tab;
    if (!t) return;
    const r = t.rect;
    const to = r.col + dir;
    if (to < 0 || to + r.cols > t.info.cols) return;
    if (await change({ op: "move_cols", at: r.col, count: r.cols, to })) {
      // Carry the moved columns' widths, formats and wrapping with them.
      t.noteColMove(r.col, r.cols, to);
      grid?.select({ row: t.sel.anchor.row, col: t.sel.anchor.col + dir }, { row: t.sel.focus.row, col: t.sel.focus.col + dir });
    }
  }

  /** Columns dragged by their headers to start at `to`. */
  async function dragCols(at: number, count: number, to: number): Promise<void> {
    const t = tab;
    if (!t || !ready()) return;
    if (await change({ op: "move_cols", at, count, to })) {
      t.noteColMove(at, count, to);
      grid?.select({ row: t.firstRow, col: to }, { row: Math.max(t.firstRow, t.info.rows - 1), col: to + count - 1 }, false);
      say(`Moved ${count === 1 ? "1 column" : `${count.toLocaleString()} columns`}. Ctrl+Z puts ${count === 1 ? "it" : "them"} back.`);
    }
  }

  /** Rows dragged by their numbers to start at `to`. */
  async function dragRows(at: number, count: number, to: number): Promise<void> {
    const t = tab;
    if (!t || !ready()) return;
    if (t.info.filtered) {
      say("Rows can't be moved while a filter is on. Clear the filter first (Filtered ✕).");
      return;
    }
    if (await change({ op: "move_rows", at, count, to })) {
      grid?.select({ row: to, col: 0 }, { row: to + count - 1, col: Math.max(0, t.info.cols - 1) }, false);
      say(`Moved ${count === 1 ? "1 row" : `${count.toLocaleString()} rows`}. Ctrl+Z puts ${count === 1 ? "it" : "them"} back.`);
    }
  }

  /** The selected block dragged by its outline to start at (row, col);
   *  `copy` (Ctrl held) copies it instead, as in Excel. */
  async function dragCells(from: Rect, row: number, col: number, copy: boolean): Promise<void> {
    const t = tab;
    if (!t || !ready()) return;
    if (cellCount(from) * (copy ? 1 : 2) > MAX_CELLS_PER_CHANGE) {
      say(
        copy
          ? `Ctrl+drag copies up to ${MAX_CELLS_PER_CHANGE.toLocaleString()} cells at once. For more, use Copy (Ctrl+C) and Paste (Ctrl+V).`
          : `Moving cells works on up to ${(MAX_CELLS_PER_CHANGE / 2).toLocaleString()} at once. Whole rows and columns have no such limit.`,
      );
      return;
    }
    if (await apply((x) => backend().moveCells(x.id, from.row, from.rows, from.col, from.cols, row, col, copy))) {
      grid?.select({ row, col }, { row: row + from.rows - 1, col: col + from.cols - 1 });
      const n = cellCount(from);
      say(`${copy ? "Copied" : "Moved"} ${n.toLocaleString()} cell${n === 1 ? "" : "s"}. Ctrl+Z undoes it.`);
    }
  }

  // ---- file tabs: drag one along the bar to put it elsewhere -----------------
  let tabDrag = $state<{ from: number; x: number; moved: boolean; pointer: number } | null>(null);

  function tabPointerDown(e: PointerEvent, i: number): void {
    if (e.button !== 0 || (e.target as HTMLElement).closest("button")) return;
    tabDrag = { from: i, x: e.clientX, moved: false, pointer: e.pointerId };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function tabPointerMove(e: PointerEvent): void {
    const d = tabDrag;
    if (!d || e.pointerId !== d.pointer) return;
    if (!d.moved && Math.abs(e.clientX - d.x) < 6) return;
    d.moved = true;
    const over = document.elementFromPoint(e.clientX, e.clientY)?.closest<HTMLElement>("[data-tab-index]");
    const to = over ? Number(over.dataset.tabIndex) : NaN;
    if (!Number.isInteger(to) || to === d.from || to < 0 || to >= tabs.length) return;
    const current = tabs[active];
    const [moved] = tabs.splice(d.from, 1);
    tabs.splice(to, 0, moved!);
    if (current) active = tabs.indexOf(current);
    d.from = to;
  }

  function tabPointerUp(e: PointerEvent): void {
    if (!tabDrag || e.pointerId !== tabDrag.pointer) return;
    tabDrag = null;
  }

  async function makeHeader(): Promise<void> {
    const t = tab;
    if (!t) return;
    const row = t.sel.focus.row;
    if (row !== 0 && !(await change({ op: "move_rows", at: row, count: 1, to: 0 }))) return;
    t.header = true;
    t.headerTouched = true;
    t.cache.invalidate();
    grid?.select({ row: 1, col: t.sel.focus.col });
  }

  // ---- Milestone 9: rows, links, formats -------------------------------------------
  /** Shown data rows of the selection (the header row left out). */
  function selectedRows(t: Tab): { at: number; count: number } {
    const r = t.rect;
    const at = Math.max(r.row, t.firstRow);
    return { at, count: Math.max(0, r.row + r.rows - at) };
  }

  async function hideRows(): Promise<void> {
    const t = tab;
    if (!t) return;
    const { at, count } = selectedRows(t);
    if (count <= 0) return;
    if (count >= t.info.rows - t.firstRow) {
      say("At least one row must stay visible.");
      return;
    }
    if (await change({ op: "hide_rows", at, count }, false)) {
      grid?.select({ row: Math.min(at, Math.max(t.firstRow, t.info.rows - 1)), col: t.sel.focus.col });
      say(`${plural(count, "row")} hidden. Hidden rows are still saved; Ctrl+Shift+9 shows them again.`);
    }
  }

  async function showRows(all: boolean): Promise<void> {
    const t = tab;
    if (!t || !t.info.hidden_rows) return;
    const before = t.info.hidden_rows;
    const r = t.rect;
    // From the top when the selection starts at the first data row.
    const at = r.row <= t.firstRow ? 0 : r.row;
    const count = r.row + r.rows - at;
    const whole = r.rows >= t.info.rows - t.firstRow;
    const ok = await change({ op: "show_rows", which: all || whole ? "all" : { around: { at, count } } }, false);
    if (!ok || t !== tab) return;
    const n = before - t.info.hidden_rows;
    say(n ? `${plural(n, "hidden row")} shown again.` : "No hidden rows in the selection. Select the rows on both sides of the hidden ones, or use Unhide all rows.");
  }

  async function shuffleRows(): Promise<void> {
    const t = tab;
    if (!t) return;
    const one = t.rect.rows <= 1;
    const at = one ? t.firstRow : selectedRows(t).at;
    const count = one ? t.info.rows - t.firstRow : selectedRows(t).count;
    if (count < 2) {
      say("There must be at least two rows to shuffle.");
      return;
    }
    if (await applyJob("Shuffling", (x) => backend().change(x.id, { op: "shuffle_rows", at, count })))
      say(`Shuffled ${plural(count, "row")} into a random order${t.info.filtered ? " (hidden and filtered-out rows stayed where they were)" : ""}. Undo (Ctrl+Z) puts them back.`);
  }

  // ---- Milestone 10: tools, file chores, view options ---------------------------

  /** Run what a Tools dialog asked for and say what happened. */
  async function runTools(r: ToolsRun, header: boolean): Promise<void> {
    const t = tab;
    if (!t) return;
    if (header !== t.header && !r.combine) {
      t.header = header;
      t.headerTouched = true;
      t.bump();
    }
    try {
      if (r.combine) {
        const spec = r.combine;
        const d = await backend().combineFiles(spec);
        if (!d) return;
        addTab(d, false);
        const nt = tabs[tabs.length - 1];
        if (nt) {
          nt.header = spec.header;
          nt.headerTouched = true;
          nt.bump();
        }
        say(`Combined into a new table of ${plural(Math.max(0, d.rows - (spec.header ? 1 : 0)), "row")}. It is not saved yet (Ctrl+S).`);
        return;
      }
      if (r.split) {
        const spec = r.split;
        // null: the folder dialog was cancelled.
        const rep = await runJob("Splitting into files", () => backend().splitToFiles(t.id, spec, false));
        if (rep) say(`Wrote ${plural(rep.files, "file")} holding ${plural(rep.rows, "row")} into the folder you chose.${splitPrivateText(rep)}`);
        return;
      }
      if (r.table) {
        const req = r.table;
        const rep = await runJob("Building the table", () => backend().tableReport(t.id, req));
        if (!rep) return;
        addTab(rep.info, false);
        const nt = tabs[tabs.length - 1];
        if (nt) {
          nt.header = req.op === "compare" || header;
          nt.headerTouched = true;
          nt.bump();
        }
        const c = rep.compare;
        say(
          c
            ? `Compared: ${plural(c.changed, "changed row")} (${plural(c.cells, "cell")}), ${plural(c.added, "added row")}, ${plural(c.removed, "removed row")}, ${plural(c.same, "row")} the same. The list opened as a new tab.`
            : `The new table has ${plural(rep.info.rows, "row")} and ${plural(rep.info.cols, "column")}. It is not saved yet (Ctrl+S).`,
        );
        return;
      }
    } catch (e) {
      if (isCmdError(e) && e.kind === "exists" && r.split) {
        const spec = r.split;
        const a = await ask("Replace the files?", `${e.message}. Replace them with the new parts?`, [
          { id: "replace", label: "Replace them", danger: true },
          { id: "cancel", label: "Cancel", primary: true },
        ], "cancel");
        if (a !== "replace") return;
        try {
          const rep = await runJob("Splitting into files", () => backend().splitToFiles(t.id, spec, true));
          if (rep) say(`Wrote ${plural(rep.files, "file")} holding ${plural(rep.rows, "row")} (${plural(rep.replaced, "file")} replaced).${splitPrivateText(rep)}`);
        } catch (e2) {
          say(errorText(e2));
        }
        return;
      }
      say(errorText(e));
      return;
    }
    const op = r.data;
    if (!op) return;
    const rep = await runOp(op, "Working");
    if (!rep) return;
    switch (op.op) {
      case "fill_blanks":
        say(rep.groups ? `Filled ${plural(rep.groups, "blank cell")} in ${plural(rep.rows, "row")}. Undo with Ctrl+Z.` : "No blank cells to fill (or nothing above or below them).");
        break;
      case "count_repeats":
        say(`Added a Count column after the chosen ${op.spec.cols.length === 1 ? "column" : "columns"}.`);
        break;
      case "blank_repeats":
        say(rep.rows ? `Blanked ${plural(rep.rows, "repeat")} of ${plural(rep.groups, "value")} (the first of each stays).` : "No repeats found.");
        break;
      case "breaks":
        say(rep.rows ? `Added ${plural(rep.rows, "blank row")} at ${plural(rep.groups, "break")}.` : "No places to break.");
        break;
      default:
        break;
    }
  }

  function openTools(kind: ToolKind): void {
    if (tab) toolsKind = kind;
  }

  async function flipRows(): Promise<void> {
    const t = tab;
    if (!t) return;
    const one = t.rect.rows <= 1;
    const at = one ? t.firstRow : selectedRows(t).at;
    const count = one ? t.info.rows - t.firstRow : selectedRows(t).count;
    if (count < 2) {
      say("There must be at least two rows to flip.");
      return;
    }
    if (await applyJob("Flipping", (x) => backend().change(x.id, { op: "flip_rows", at, count })))
      say(`Flipped the order of ${plural(count, "row")}. Undo (Ctrl+Z) puts them back.`);
  }

  async function deleteHiddenRows(): Promise<void> {
    const t = tab;
    if (!t || !t.info.hidden_rows) {
      say("No rows are hidden (rows a filter leaves out are not deleted).");
      return;
    }
    const n = t.info.hidden_rows;
    const a = await ask("Delete the hidden rows?", `${plural(n, "hidden row")} will be deleted. Rows a filter leaves out stay. Undo (Ctrl+Z) brings them back.`, [
      { id: "go", label: "Delete them", danger: true },
      { id: "cancel", label: "Cancel", primary: true },
    ], "cancel");
    if (a !== "go") return;
    const rep = await runOp({ op: "delete_hidden" }, "Deleting hidden rows");
    if (rep) say(`Deleted ${plural(rep.rows, "hidden row")}.`);
  }

  async function deleteHiddenCols(): Promise<void> {
    const t = tab;
    if (!t) return;
    const cols: number[] = [];
    for (let c = 0; c < t.info.cols && cols.length < 16_384; c++) if (t.columns.isHidden(c)) cols.push(c);
    if (!cols.length) {
      say("No columns are hidden.");
      return;
    }
    const a = await ask("Delete the hidden columns?", `${plural(cols.length, "hidden column")} will be deleted. Undo (Ctrl+Z) brings them back.`, [
      { id: "go", label: "Delete them", danger: true },
      { id: "cancel", label: "Cancel", primary: true },
    ], "cancel");
    if (a !== "go") return;
    if (await applyJob("Deleting columns", (x) => backend().change(x.id, { op: "delete_col_list", cols }))) {
      t.columns.showAll();
      t.bump();
      say(`Deleted ${plural(cols.length, "hidden column")}.`);
    }
  }

  /** Move to the next (or previous) row whose cell in this column differs. */
  async function jumpChange(up: boolean): Promise<void> {
    const t = tab;
    if (!t) return;
    const f = t.sel.focus;
    try {
      const r = await backend().findChange(t.id, f.row, f.col, up);
      if (r === null || t !== tab) {
        say(up ? "No different value above." : "No different value below.");
        return;
      }
      grid?.select({ row: r, col: f.col }, { row: r, col: f.col });
    } catch (e) {
      say(errorText(e));
    }
  }

  async function jumpExtreme(largest: boolean): Promise<void> {
    const t = tab;
    if (!t) return;
    const col = t.sel.focus.col;
    try {
      const r = await runJob("Looking", () => backend().findExtreme(t.id, col, largest, t.header, { decimal_comma: false, date_order: "mdy" }));
      if (r === null || r === undefined || t !== tab) {
        say("This column holds no numbers.");
        return;
      }
      grid?.select({ row: r, col }, { row: r, col });
    } catch (e) {
      say(errorText(e));
    }
  }

  async function revertFile(): Promise<void> {
    const t = tab;
    if (!t?.info.path) return;
    if (t.info.modified) {
      const a = await ask("Go back to the saved file?", "Your changes since the last save will be lost. The file on disk is read again.", [
        { id: "go", label: "Revert", danger: true },
        { id: "cancel", label: "Cancel", primary: true },
      ], "cancel");
      if (a !== "go") return;
    }
    await reopenTab(t, false, null);
    say(`Reverted to the saved ${t.info.name}.`);
  }

  async function renameFile(): Promise<void> {
    const t = tab;
    if (!t?.info.path) return;
    try {
      const d = await backend().renameFile(t.id);
      if (!d) return;
      t.update(d, false);
      say(`Renamed to ${d.name}.`);
    } catch (e) {
      say(errorText(e));
    }
  }

  async function showInFolder(): Promise<void> {
    const t = tab;
    if (!t?.info.path) return;
    await backend().showInFolder(t.id).catch((e: unknown) => say(errorText(e)));
  }

  async function copyPath(): Promise<void> {
    const p = tab?.info.path;
    if (!p) return;
    try {
      await navigator.clipboard.writeText(p);
      say("Copied the file's path.");
    } catch {
      say("Could not reach the clipboard.");
    }
  }

  async function closeUnchanged(): Promise<void> {
    const keep = tab;
    const gone = unchangedTabs(tabs);
    for (const t of gone) {
      const i = tabs.indexOf(t);
      if (i >= 0) await closeTab(i);
    }
    const i = keep ? tabs.indexOf(keep) : -1;
    if (i >= 0) active = i;
    say(gone.length ? `Closed ${plural(gone.length, "tab")} with nothing unsaved.` : "Every tab has unsaved changes; nothing was closed.");
  }

  async function saveAll(): Promise<void> {
    const start = tab;
    let saved = 0;
    for (const t of tabs.filter((x) => x.info.modified)) {
      active = tabs.indexOf(t);
      await tick();
      if (!(await save(false))) break;
      saved++;
    }
    const i = start ? tabs.indexOf(start) : -1;
    if (i >= 0) active = i;
    say(saved ? `Saved ${plural(saved, "file")}.` : "Nothing to save.");
  }

  /** The selected cells as a new, unsaved tab. */
  async function selectionToTab(): Promise<void> {
    const t = tab;
    if (!t) return;
    const r = t.rect;
    try {
      const text = await backend().copyCells(t.id, r.row, r.rows, r.col, r.cols);
      const d = await backend().newDocument();
      addTab(d, false);
      const nt = tabs[tabs.length - 1];
      if (!nt) return;
      const rep = await backend().dataOp(nt.id, { op: "paste", spec: { row: 0, col: 0, op: "none", skip_blanks: false, transpose: false }, text });
      if (rep.info) nt.update(rep.info, true);
      say(`Copied ${plural(r.rows, "row")} × ${plural(r.cols, "column")} into a new tab. It is not saved yet (Ctrl+S).`);
    } catch (e) {
      say(errorText(e));
    }
  }

  function clearHighlights(): void {
    const t = tab;
    if (!t || (!t.highlights.length && !t.scales.length)) {
      say("There are no highlight rules here.");
      return;
    }
    setHighlights([]);
    t.scales = [];
    t.ranges.clear();
    t.bump();
    say("Removed every highlight rule, colour scale and data bar from this tab.");
  }

  function clearChecks(): void {
    const t = tab;
    if (!t || !t.checks.length) {
      say("There are no data checks here.");
      return;
    }
    keepChecks([], false);
    say("Removed every data check from this tab.");
  }

  function toggleValueTypes(): void {
    const t = tab;
    if (!t) return;
    t.valueTypes = !t.valueTypes;
    t.bump();
    say(t.valueTypes ? "Numbers show in blue and dates in green; text stays as it is." : "Value types are no longer coloured.");
  }

  function toggleDivider(): void {
    const t = tab;
    if (!t) return;
    const c = t.sel.focus.col;
    t.divider = t.divider === c ? null : c;
    t.bump();
    say(t.divider === null ? "Group lines are off." : `A line now marks each change in ${columnTitle(t, c)}.`);
  }

  /** A colour scale or data bar on the active column (again: off). */
  function toggleScale(kind: ScaleKind): void {
    const t = tab;
    if (!t) return;
    const c = t.sel.focus.col;
    const had = t.scales.find((s) => s.col === c);
    t.scales = t.scales.filter((s) => s.col !== c);
    if (!had || had.kind !== kind) t.scales = [...t.scales, { col: c, kind }];
    t.ranges.delete(c);
    void loadRanges(t);
  }

  /** Read the smallest and largest numbers of the scaled columns. */
  async function loadRanges(t: Tab): Promise<void> {
    for (const s of t.scales) {
      try {
        const r = await backend().numberRange(t.id, s.col, t.header, { decimal_comma: false, date_order: "mdy" });
        t.ranges.set(s.col, r);
      } catch {
        // Busy: the next change reads it again.
      }
    }
    t.bump();
  }

  // Scales follow the data: read the ranges again after each change.
  $effect(() => {
    const t = tab;
    if (!t || !t.scales.length) return;
    void t.info.modified;
    void t.info.can_undo;
    void t.info.rows;
    void t.header;
    const timer = setTimeout(() => void loadRanges(t), 300);
    return () => clearTimeout(timer);
  });

  async function openLink(url: string): Promise<void> {
    try {
      await backend().openLink(url);
      say(`Opening ${url.length > 80 ? `${url.slice(0, 80)}…` : url}`);
    } catch (e) {
      say(errorText(e));
    }
  }

  /** Selected columns that are shown (hidden ones left alone). */
  function selectedCols(t: Tab): number[] {
    const r = t.rect;
    const out: number[] = [];
    for (let c = r.col; c < r.col + r.cols && out.length < 16_384; c++) if (!t.columns.isHidden(c)) out.push(c);
    return out;
  }

  function toggleWrap(): void {
    const t = tab;
    if (!t) return;
    const cols = selectedCols(t);
    const on = !cols.every((c) => t.columns.wraps.has(c));
    for (const c of cols) {
      if (on) t.columns.wraps.add(c);
      else t.columns.wraps.delete(c);
    }
    if (on) grid?.fitRowLines();
    else if (!t.columns.wraps.size) t.rowLines = 1;
    t.bump();
    say(on ? `Text wraps in ${plural(cols.length, "column")}; rows grew to fit what is on screen (Format › Row height to change it).` : `Text no longer wraps in ${plural(cols.length, "column")}.`);
  }

  function askRowLines(): void {
    const t = tab;
    if (!t) return;
    prompt = {
      title: "Row height",
      label: "Lines of text per row",
      value: String(t.rowLines),
      note: `1 to ${MAX_ROW_LINES}. Every row has the same height; columns set to wrap show their text over these lines.`,
      check: (v) => (/^\d+$/.test(v.trim()) && Number(v) >= 1 && Number(v) <= MAX_ROW_LINES ? "" : `Type a whole number from 1 to ${MAX_ROW_LINES}.`),
      done: (v) => {
        t.rowLines = Number(v);
        t.bump();
      },
    };
  }

  function openFormat(): void {
    const t = tab;
    if (!t) return;
    const cols = selectedCols(t);
    if (!cols.length) return;
    const c0 = cols[0]!;
    const samples: string[] = [];
    const r0 = Math.max(t.firstRow, t.rect.row);
    for (let r = r0; r < r0 + 40 && samples.length < 8; r++) {
      const v = t.cache.get(r, c0);
      if (v) samples.push(v);
    }
    formatFor = {
      cols,
      what: cols.length === 1 ? columnTitle(t, c0) : `${cols.length} columns`,
      current: t.columns.formats.get(c0) ?? null,
      samples,
    };
  }

  function setFormat(cols: number[], f: DisplayFormat | null): void {
    const t = tab;
    if (!t) return;
    for (const c of cols) {
      if (f) t.columns.formats.set(c, f);
      else t.columns.formats.delete(c);
    }
    t.bump();
    say(f ? `${plural(cols.length, "column")} now show as ${describeFormat(f)}. The file itself does not change.` : `Removed the display format from ${plural(cols.length, "column")}.`);
  }

  /** Write the chosen columns' display formats into the cells (the file
   *  then holds the text as shown) and drop the formats. One clean per
   *  format; each is one undo step. */
  async function bakeFormats(): Promise<void> {
    const t = tab;
    if (!t) return;
    const groups = new Map<string, { f: DisplayFormat; cols: number[] }>();
    for (const c of selectedCols(t)) {
      const f = t.columns.formats.get(c);
      if (!f || f.kind === "checkbox") continue;
      const key = JSON.stringify(f);
      const g = groups.get(key) ?? { f, cols: [] };
      g.cols.push(c);
      groups.set(key, g);
    }
    if (!groups.size) {
      say("The selected columns have no number or date display format to write.");
      return;
    }
    for (const { f, cols } of groups.values()) {
      const spec: CleanSpec = {
        cleaner: { version: 1, name: "", steps: formatSteps(f) },
        scope: { kind: "columns", cols },
        skip_first: t.header,
        read: READ,
      };
      const rep = await runOp({ op: "clean", spec, now: localNow() }, "Writing the display format");
      if (!rep) return;
      for (const c of cols) t.columns.formats.delete(c);
      t.bump();
      const left = rep.left ? ` ${plural(rep.left, "cell")} left as they were (not a number or date).` : "";
      say(`Wrote ${describeFormat(f)} into ${plural(rep.cells, "cell")}.${left} Undo with Ctrl+Z.`);
    }
  }

  function toggleBanded(): void {
    const t = tab;
    if (!t) return;
    t.banded = !t.banded;
    prefs.data.banded = t.banded;
    prefs.save();
    t.bump();
  }

  async function openSuggestions(): Promise<void> {
    const t = tab;
    if (!t) return;
    suggest = { found: suggest?.found ?? null, busy: true };
    try {
      const r = await runJob("Looking for clean-ups", () => backend().cleanSuggestions(t.id, t.header, READ));
      if (!suggest) return;
      suggest = r ? { found: r, busy: false } : null;
    } catch (e) {
      suggest = null;
      say(errorText(e));
    }
  }

  /** Run one suggestion's fix (one undo step). */
  async function runSuggestion(s: Suggestion): Promise<OpReport | null> {
    const t = tab;
    if (!t) return null;
    const f = s.fix;
    const op: DataOp =
      f.fix === "clean"
        ? { op: "clean", spec: { cleaner: { version: 1, name: "", steps: [f.step] }, scope: { kind: "columns", cols: f.cols }, skip_first: t.header, read: READ }, now: localNow() }
        : f.fix === "remove_duplicates"
          ? { op: "remove_duplicates", spec: { cols: [], match_case: true, trim: false, keep_first: t.header } }
          : { op: "remove_blank_rows", spec: { spaces: true, keep_first: t.header } };
    return runOp(op, "Cleaning");
  }

  async function fixSuggestions(list: Suggestion[]): Promise<void> {
    if (!suggest) return;
    suggest.busy = true;
    // Cell fixes first (cleaned spaces can make more rows repeat), then rows.
    const order = [...list].sort((a, b) => Number(a.col === null) - Number(b.col === null));
    let cells = 0;
    let rows = 0;
    let done = 0;
    for (const s of order) {
      const rep = await runSuggestion(s);
      if (!rep) break;
      done++;
      cells += rep.cells;
      rows += rep.rows;
    }
    if (done) say(`Fixed ${plural(done, "suggestion")}: ${plural(cells, "cell")} changed${rows ? `, ${plural(rows, "row")} removed` : ""}. Undo (Ctrl+Z) takes back one fix at a time.`);
    await openSuggestions();
  }

  // ---- autosave and recovery -------------------------------------------------------
  /** Keep recovery copies of tabs with unsaved changes (called every few seconds). */
  let autosaveRunning = false;
  async function autosaveTick(): Promise<void> {
    // A copy of a big file can take longer than one tick: never start a
    // second round alongside it.
    if (!prefs.data.autosave || autosaveRunning) return;
    autosaveRunning = true;
    try {
      await autosaveRound();
    } finally {
      autosaveRunning = false;
    }
  }

  async function autosaveRound(): Promise<void> {
    const now = Date.now();
    for (const t of tabs) {
      if (!t.info.modified || t.private.file || t.private.folder) {
        autosavedAt.delete(t.id);
        continue;
      }
      // The first unsaved change starts the clock.
      if (!autosavedAt.has(t.id)) autosavedAt.set(t.id, now);
      if (!autosaveDue(now, autosavedAt.get(t.id) ?? null, prefs.data.autosaveMinutes, t.info.modified, t.info.indexed && !t.busy)) continue;
      try {
        const r = await backend().autosave(t.id);
        if (r.kind === "written" || r.kind === "unchanged") autosavedAt.set(t.id, Date.now());
        if (r.kind === "written" && t === tab) autosaveNote = `Recovery copy kept ${new Date(r.at * 1000).toLocaleTimeString(undefined, { timeStyle: "short" })}`;
        if (r.kind === "too_large") {
          // Asked again only after the usual wait.
          autosavedAt.set(t.id, Date.now());
          if (t === tab) autosaveNote = "Too large for a recovery copy";
        }
        if (r.kind === "full") {
          autosavedAt.set(t.id, Date.now());
          if (t === tab) autosaveNote = "No recovery copy: the recovery folder is full (File › Recover unsaved work)";
        }
      } catch {
        // Closed meanwhile, or the folder is not writable: try again later.
      }
    }
  }

  async function showRecovery(auto: boolean): Promise<void> {
    try {
      const items = await backend().recoveryList();
      if (auto && !items.length) return;
      recovery = { mode: "recover", items, doc: 0, name: "" };
    } catch (e) {
      if (!auto) say(errorText(e));
    }
  }

  async function showVersions(): Promise<void> {
    const t = tab;
    if (!t) return;
    recovery = { mode: "versions", items: null, doc: t.id, name: t.info.name };
    try {
      const items = await backend().versionsList(t.id);
      if (recovery?.doc === t.id) recovery.items = items;
    } catch (e) {
      recovery = null;
      say(errorText(e));
    }
  }

  async function openRecovered(index: number, label: string): Promise<void> {
    const r = recovery;
    if (!r) return;
    try {
      const d = r.mode === "recover" ? await backend().recoveryOpen(index) : await backend().versionOpen(r.doc, index, label);
      addTab(d);
      if (r.mode === "recover") {
        // Indexes change once one is taken: list again.
        const items = await backend().recoveryList();
        recovery = items.length ? { ...r, items } : null;
        say(`Recovered “${d.name}”. Save it (Ctrl+S) to choose where it goes; your original file was not changed.`);
      } else {
        recovery = null;
        say(`Opened an earlier save as “${d.name}”. Save it under a name to keep it.`);
      }
    } catch (e) {
      say(errorText(e));
    }
  }

  async function discardRecovered(index: number): Promise<void> {
    try {
      await backend().recoveryDiscard(index);
      const items = await backend().recoveryList();
      recovery = items.length ? { mode: "recover", items, doc: 0, name: "" } : null;
    } catch (e) {
      say(errorText(e));
    }
  }

  /** Alt+Down: the list of allowed values on a list column, else the filter menu. */
  async function altDown(): Promise<void> {
    if (grid && (await grid.openPick())) return;
    openFilterMenu();
  }

  function toggleHeader(): void {
    const t = tab;
    if (!t) return;
    t.header = !t.header;
    t.headerTouched = true;
    grid?.select(t.sel.anchor, t.sel.focus);
  }

  function freezeAtCursor(): void {
    const t = tab;
    if (!t) return;
    t.frozenRows = Math.max(0, t.sel.focus.row - t.firstRow);
    t.frozenCols = t.sel.focus.col;
    t.scrollX = 0;
    t.scrollY = 0;
    if (t.frozenRows === 0 && t.frozenCols === 0) say("Select the cell below and right of what should stay in view, then freeze.");
  }

  function hideCols(): void {
    const t = tab;
    if (!t) return;
    const r = t.rect;
    let visible = 0;
    for (let c = 0; c < t.info.cols && visible <= r.cols; c++) if (!t.columns.isHidden(c)) visible++;
    if (visible <= r.cols) {
      say("At least one column must stay visible.");
      return;
    }
    for (let c = r.col; c < r.col + r.cols; c++) t.columns.setWidth(c, 0);
    const next = t.columns.firstVisible(r.col + r.cols);
    // The top of the selection: a whole column's focus is the last row.
    grid?.select({ row: r.row, col: next >= 0 ? next : t.columns.firstVisible(0) });
    t.bump();
  }

  function goTo(text: string): void {
    const t = tab;
    if (!t) return;
    const nums = (text.match(/\d+/g) ?? []).map(Number);
    if (nums.length === 0) {
      say('Type a row number, or "row, column" (for example 250, 3).');
      return;
    }
    const row = Math.min(t.info.rows, Math.max(1, nums[0]!)) - 1;
    const col = nums.length > 1 ? Math.min(t.info.cols, Math.max(1, nums[1]!)) - 1 : t.sel.focus.col;
    grid?.select({ row, col });
    grid?.focus();
  }

  async function undoRedo(op: "undo" | "redo"): Promise<void> {
    if (grid?.isEditing()) return;
    const t = tab;
    if (!t) return;
    // One at a time per tab: a held Ctrl+Z repeats faster than a big undo.
    if (undoBusy.has(t)) return;
    undoBusy.add(t);
    if (op === "redo") t.redoing++;
    try {
      await change({ op });
    } finally {
      if (op === "redo") t.redoing--;
      undoBusy.delete(t);
    }
  }
  const undoBusy = new Set<Tab>();

  // ---- long operations ------------------------------------------------------------
  /** Run a long engine call: the status bar shows progress and offers
   *  Cancel. Returns null when cancelled. */
  async function runJob<T>(label: string, f: () => Promise<T>): Promise<T | null> {
    const t = tab;
    if (!t) return null;
    if (t.busy) {
      say("Please wait: another operation is still running.");
      return null;
    }
    t.busy = `${label}…`;
    t.progress = null;
    const poll = window.setInterval(async () => {
      try {
        const j = await backend().jobProgress(t.id);
        if (j && t.busy) t.progress = j.fraction;
      } catch {
        // Closed meanwhile.
      }
    }, 150);
    try {
      return await f();
    } catch (e) {
      if (isCmdError(e) && e.kind === "cancelled") {
        say("Cancelled; nothing was changed.");
        return null;
      }
      throw e;
    } finally {
      clearInterval(poll);
      t.busy = null;
      t.progress = null;
    }
  }

  function cancelJob(): void {
    if (tab) void backend().cancelJob(tab.id);
  }

  /** Run a long operation that returns the new document state. */
  async function applyJob(label: string, f: (t: Tab) => Promise<DocInfo>): Promise<boolean> {
    const t = tab;
    if (!t) return false;
    try {
      const d = await runJob(label, () => f(t));
      if (!d) return false;
      t.update(d, true);
      return true;
    } catch (e) {
      say(errorText(e));
      return false;
    }
  }

  // ---- sort and filter ---------------------------------------------------------------
  function columnTitle(t: Tab, c: number): string {
    const v = t.header ? t.cache.get(0, c) : undefined;
    if (v) return v.replace(/[\r\n\t]+/g, " ").slice(0, 60);
    const shown = t === tab ? grid?.title(c) : undefined;
    return shown && shown !== String(c + 1) ? shown.slice(0, 60) : `Column ${c + 1}`;
  }

  async function sortRows(spec: SortSpec): Promise<void> {
    const t = tab;
    if (await applyJob("Sorting", (x) => backend().sortRows(x.id, spec))) {
      if (t) t.sort = spec;
      say("Sorted. Undo (Ctrl+Z) puts the rows back.");
    }
  }

  function sortByColumn(descending: boolean, col = tab?.sel.focus.col ?? 0): void {
    const t = tab;
    if (!t) return;
    void sortRows({ keys: [{ col, descending, kind: "auto", match_case: false }], keep_first: t.header });
  }

  function openSortDialog(): void {
    const t = tab;
    if (!t) return;
    sortDialog = [{ col: t.sel.focus.col, descending: false, kind: "auto", match_case: false }];
  }

  async function setRule(col: number, rule: Rule | null): Promise<void> {
    const t = tab;
    if (!t) return;
    const spec = withRule(t.filter, col, rule, t.header);
    const ok = spec.columns.length
      ? await applyJob("Filtering", (x) => backend().setFilter(x.id, spec))
      : await applyJob("Showing all rows", (x) => backend().clearFilter(x.id));
    if (!ok) return;
    t.filter = spec.columns.length ? spec : null;
    if (spec.columns.length) t.filterButtons = true;
    grid?.select({ row: t.firstRow, col: t.sel.focus.col });
    if (t.info.filtered && t.info.rows <= t.firstRow) say("No rows match; the filter hides them all.");
  }

  function toggleFilterButtons(): void {
    const t = tab;
    if (!t) return;
    if (t.filterButtons || t.info.has_filter) {
      t.filterButtons = false;
      if (t.info.has_filter) void clearFilter();
    } else t.filterButtons = true;
    t.bump();
  }

  async function clearFilter(): Promise<void> {
    const t = tab;
    if (!t) return;
    if (await applyJob("Showing all rows", (x) => backend().clearFilter(x.id))) t.filter = null;
  }

  async function reapplyFilter(): Promise<void> {
    const t = tab;
    if (!t?.filter) return;
    const spec = { ...t.filter, keep_first: t.header };
    await applyJob("Filtering", (x) => backend().setFilter(x.id, spec));
  }

  /** Show only rows whose value in this column equals the active cell's. */
  async function filterBySelection(): Promise<void> {
    const t = tab;
    if (!t) return;
    const f = t.sel.focus;
    try {
      const v = await backend().cellText(t.id, f.row, f.col);
      if (v.truncated) {
        say("This cell is too long to filter by.");
        return;
      }
      await setRule(f.col, v.text === "" ? { type: "values", values: [], blanks: true } : { type: "values", values: [v.text], blanks: false });
    } catch (e) {
      say(errorText(e));
    }
  }

  function openFilterMenu(col = tab?.sel.focus.col ?? 0): void {
    const t = tab;
    if (!t || !grid) return;
    t.filterButtons = true;
    const p = grid.headerPoint(col);
    filterMenu = { col, x: p.x, y: p.y };
  }

  // ---- find --------------------------------------------------------------------------
  function openFind(replace: boolean): void {
    // In the expanded cell bar, Ctrl+F searches the text being edited.
    if (document.activeElement === cellBarInput && cellBarExpanded && !replace) {
      cellFind = { text: cellFind?.text ?? findOptions.text, hits: [], at: -1 };
      void tick().then(() => document.querySelector<HTMLInputElement>(".cellfind input")?.focus());
      return;
    }
    const sel = window.getSelection()?.toString() ?? "";
    findOpen = true;
    findReplace = replace;
    if (sel && sel.length < 200 && !sel.includes("\n")) findOptions.text = sel;
    void tick().then(() => findPanel?.focus());
  }

  function findAgain(backward: boolean): void {
    if (!findOptions.text) {
      openFind(false);
      return;
    }
    if (findPanel) {
      void findPanel.next(backward);
      return;
    }
    findOpen = true;
    void tick().then(() => findPanel?.next(backward));
  }

  async function cellFindRun(dir: 1 | -1): Promise<void> {
    const cf = cellFind;
    const box = cellBarInput;
    if (!cf || !box || !cf.text) return;
    const pattern: Pattern = { text: cf.text, kind: findOptions.kind, match_case: findOptions.matchCase, whole_cell: false };
    try {
      const hits = await backend().findInText(cellBarText, pattern);
      if (hits.length === 0) {
        cellFind = { ...cf, hits, at: -1 };
        return;
      }
      const cur = box.selectionStart ?? 0;
      let at: number;
      if (dir > 0) {
        at = hits.findIndex(([s]) => s > cur || (s === cur && cf.at < 0));
        if (at < 0) at = 0;
      } else {
        at = hits.length - 1;
        while (at >= 0 && hits[at]![0] >= cur) at--;
        if (at < 0) at = hits.length - 1;
      }
      cellFind = { ...cf, hits, at };
      const [s, e] = hits[at]!;
      box.focus();
      box.setSelectionRange(s, e);
      void tick().then(() => document.querySelector<HTMLInputElement>(".cellfind input")?.focus());
    } catch (e) {
      say(errorText(e));
    }
  }

  // ---- cell bar ---------------------------------------------------------------------
  let cellBarFor = "";
  $effect(() => {
    const t = tab;
    if (!t) {
      cellBarText = "";
      return;
    }
    const f = t.sel.focus;
    void t.version;
    const key = `${t.id}:${f.row}:${f.col}:${t.version}`;
    if (key === cellBarFor || cellBarDirty) return;
    cellBarFor = key;
    gotoText = `R${f.row + 1} C${f.col + 1}`;
    const cached = t.cache.get(f.row, f.col);
    if (cached !== undefined && !t.cache.isTruncated(f.row, f.col)) {
      cellBarText = cached;
      cellBarTruncated = false;
      return;
    }
    backend()
      .cellText(t.id, f.row, f.col)
      .then((v) => {
        if (cellBarFor !== key || cellBarDirty) return;
        cellBarText = v.text;
        cellBarTruncated = v.truncated;
      })
      .catch(() => {});
  });

  async function applyCellBar(): Promise<void> {
    const t = tab;
    if (!t) return;
    if (cellBarTruncated) {
      say("This cell is too long to edit here.");
      return;
    }
    const f = t.sel.focus;
    const multi = cellCount(t.rect) > 1;
    const ok = await commitCell(f.row, f.col, cellBarText, multi);
    if (ok) {
      cellBarDirty = false;
      cellBarFor = "";
      grid?.focus();
    }
  }

  function cellBarKey(e: KeyboardEvent): void {
    e.stopPropagation();
    if (e.key === "Escape") {
      cellBarDirty = false;
      cellBarFor = "";
      grid?.focus();
    } else if (e.key === "Enter" && (e.ctrlKey || !cellBarExpanded) && !e.altKey) {
      e.preventDefault();
      void applyCellBar();
    } else if (e.key === "Enter" && e.altKey && cellBarInput) {
      e.preventDefault();
      cellBarInput.setRangeText("\n", cellBarInput.selectionStart, cellBarInput.selectionEnd, "end");
      cellBarText = cellBarInput.value;
      cellBarDirty = true;
    } else if (e.key === "Tab" && cellBarExpanded && cellBarInput) {
      e.preventDefault();
      cellBarInput.setRangeText("\t", cellBarInput.selectionStart, cellBarInput.selectionEnd, "end");
      cellBarText = cellBarInput.value;
      cellBarDirty = true;
    } else if (keyOf(e) === "Ctrl+Shift+U") {
      e.preventDefault();
      cellBarExpanded = !cellBarExpanded;
    } else if (keyOf(e) === "Ctrl+F" && cellBarExpanded) {
      // Search the text being edited.
      e.preventDefault();
      openFind(false);
    } else if ((keyOf(e) === "F3" || keyOf(e) === "Shift+F3") && cellFind) {
      e.preventDefault();
      void cellFindRun(e.shiftKey ? -1 : 1);
    }
  }

  // ---- commands ------------------------------------------------------------------------
  const reg = new Registry();
  const setTheme = (t: Theme) => () => {
    prefs.data.theme = t;
    prefs.save();
  };
  const cmds: Command[] = [
    { id: "file.new", label: "&New", keys: ["Ctrl+N"], run: newFile },
    { id: "file.open", label: "&Open…", keys: ["Ctrl+O", "Ctrl+F12"], run: openFile },
    { id: "file.recent", label: "Open &recent…", keys: ["Ctrl+Shift+O"], run: () => void loadRecent().then(() => (recentOpen = true)) },
    { id: "file.openEncrypted", label: "Open an &encrypted file…", run: () => { encryptMode = "open"; } },
    { id: "file.saveEncrypted", label: "Save an encr&ypted copy…", run: () => { encryptMode = "save"; }, enabled: ready },
    { id: "file.openSqlite", label: "Open a table from a &database…", run: () => { sqliteMode = "open"; } },
    { id: "file.saveSqlite", label: "Save as a table in a data&base…", run: () => { sqliteMode = "save"; }, enabled: ready },
    { id: "file.save", label: "&Save", keys: ["Ctrl+S", "Shift+F12"], run: () => void save(false), enabled: ready },
    { id: "file.saveAs", label: "Save &As…", keys: ["F12"], run: () => void save(true), enabled: ready },
    { id: "file.import", label: "&Import…", run: () => void importFile() },
    { id: "file.importClipboard", label: "Import from clip&board…", run: () => void importClipboard() },
    { id: "file.export", label: "&Export…", run: () => void privacyGate("Exporting writes another copy of it.").then((ok) => { if (ok) exportOpen = true; }), enabled: ready },
    { id: "file.safeCopy", label: "Save a share-safe cop&y…", run: () => void openPrivacy(), enabled: ready },
    { id: "file.private", label: "Private file mode (this file)", run: () => void togglePrivate(false), enabled: hasTab, checked: () => !!tab?.private.file },
    { id: "file.privateFolder", label: "Private file mode (every file in this folder)", run: () => void togglePrivate(true), enabled: hasTab, checked: () => !!tab?.private.folder },
    { id: "tools.findPrivate", label: "Find &private data…", run: () => void openPrivacy(), enabled: ready },
    { id: "file.print", label: "&Print…", keys: ["Ctrl+P"], run: () => void openPrint(false), enabled: ready },
    { id: "file.pdf", label: "Save as P&DF…", run: () => void openPrint(true), enabled: ready },
    { id: "file.mail", label: "&Mailing labels and letters…", run: () => void openMail(), enabled: ready },
    { id: "edit.note", label: "&Note and flag on this row…", keys: ["Shift+F2"], run: () => { const t = tab; if (t) notesRow = t.sel.focus.row; }, enabled: ready },
    { id: "edit.changes", label: "List of &my changes…", run: () => void openChanges(), enabled: ready },
    { id: "tools.recipe", label: "&Recipe (save or run steps)…", run: () => void openRecipes(), enabled: ready },
    { id: "tools.searchFolder", label: "Search a &folder of files…", keys: ["Ctrl+Shift+F"], run: () => { folderSearchOpen = true; } },
    { id: "view.showEdits", label: "Show my &edits", run: () => setShowEdits(!tab?.showEdits), enabled: hasTab, checked: () => !!tab?.showEdits },
    { id: "file.options", label: "Op&tions…", run: () => { optionsOpen = true; } },
    { id: "data.compute", label: "Compu&ted column…", run: () => { computeOpen = true; }, enabled: ready },
    { id: "data.checks", label: "Data checks (&validation)…", run: () => { checksOpen = true; }, enabled: ready },
    { id: "data.fieldCheck", label: "Check &fields with patterns…", run: () => { fieldCheckOpen = true; }, enabled: ready },
    { id: "data.unmarkChecks", label: "Hide problem marks", run: () => { if (tab) keepChecks(tab.checks, false); }, enabled: () => !!tab?.checksMarked },
    { id: "help.logs", label: "Open the &log folder", run: () => void backend().openLogFolder().catch((e: unknown) => say(errorText(e))) },
    {
      id: "file.layout",
      label: "File &layout (separator, quotes)…",
      run: chooseLayout,
      enabled: () => !!tab?.info.path && !!tab?.info.indexed,
    },
    {
      id: "file.reopenLines",
      label: "&Reopen reading line by line",
      run: reopenLineByLine,
      enabled: () => !!tab?.info.path,
      checked: () => !!tab?.info.line_by_line,
    },
    { id: "file.close", label: "&Close", keys: ["Ctrl+W", "Ctrl+F4"], run: () => void closeTab(), enabled: hasTab },
    { id: "file.quit", label: "E&xit", run: quit },

    { id: "edit.undo", label: "&Undo", keys: ["Ctrl+Z"], run: () => undoRedo("undo"), enabled: () => ready() && !!tab?.info.can_undo },
    { id: "edit.redo", label: "&Redo", keys: ["Ctrl+Y", "Ctrl+Shift+Z"], run: () => undoRedo("redo"), enabled: () => ready() && !!tab?.info.can_redo },
    {
      id: "edit.repeat",
      label: "Repeat last action",
      keys: ["F4"],
      run: async () => {
        if (reg.lastRepeatable) await reg.run(reg.lastRepeatable);
      },
      enabled: () => ready() && reg.lastRepeatable !== null,
    },
    { id: "edit.cut", label: "Cu&t", keys: ["Ctrl+X"], run: () => copy(true), enabled: ready },
    { id: "edit.copy", label: "&Copy", keys: ["Ctrl+C"], run: () => copy(false), enabled: hasTab },
    { id: "edit.paste", label: "&Paste", run: pasteFromMenu, enabled: ready },
    { id: "edit.moveCut", label: "&Move what was cut here", run: () => void pasteCut(null), enabled: () => ready() && !!tab?.cut },
    { id: "edit.cancelCut", label: "Ca&ncel cut", run: cancelCut, enabled: () => !!tab?.cut },
    { id: "edit.pasteSpecial", label: "Paste &special…", keys: ["Ctrl+Alt+V"], run: () => openOp("paste_special"), enabled: ready },
    { id: "edit.clear", label: "Cle&ar contents", keys: ["Delete"], run: clearSelection, enabled: ready, repeatable: true },
    { id: "edit.fillDown", label: "Fill &down", keys: ["Ctrl+D"], run: () => fill("down"), enabled: ready, repeatable: true },
    { id: "edit.fillRight", label: "Fill r&ight", keys: ["Ctrl+R"], run: () => fill("right"), enabled: ready, repeatable: true },
    { id: "edit.fillUp", label: "Fill &up", run: () => fill("up"), enabled: ready, repeatable: true },
    { id: "edit.fillLeft", label: "Fill le&ft", run: () => fill("left"), enabled: ready, repeatable: true },
    { id: "edit.fillSeries", label: "Fill series…", run: () => openOp("series"), enabled: ready },
    { id: "edit.cell", label: "&Edit cell", keys: ["F2"], run: () => grid?.startEdit(), enabled: ready },
    {
      id: "edit.today",
      label: "Insert today's date",
      keys: ["Ctrl+;"],
      run: () => commitCell(tab!.sel.focus.row, tab!.sel.focus.col, grid?.today() ?? "", false),
      enabled: ready,
    },
    {
      id: "edit.pickDate",
      label: "Pick a date from a calendar…",
      keys: ["Ctrl+Shift+D"],
      run: () => {
        const t = tab!;
        const { row, col } = t.sel.focus;
        datePick = { row, col, text: t.cache.get(row, col) ?? "" };
      },
      enabled: ready,
    },
    {
      id: "edit.now",
      label: "Insert current time",
      keys: ["Ctrl+Shift+:", "Ctrl+Shift+;"],
      run: () => commitCell(tab!.sel.focus.row, tab!.sel.focus.col, grid?.nowTime() ?? "", false),
      enabled: ready,
    },
    {
      id: "edit.selectAll",
      label: "Select &all",
      keys: ["Ctrl+A", "Ctrl+Shift+Space"],
      run: () => grid?.select({ row: tab!.firstRow, col: 0 }, { row: tab!.info.rows - 1, col: tab!.info.cols - 1 }, false),
      enabled: hasTab,
    },
    {
      id: "edit.goto",
      label: "&Go to…",
      keys: ["Ctrl+G", "F5"],
      run: () => {
        gotoInput?.focus();
        gotoInput?.select();
      },
      enabled: hasTab,
    },
    { id: "edit.deleteSmart", label: "&Delete…", keys: ["Ctrl+-"], run: deleteSmart, enabled: ready, repeatable: true },
    { id: "edit.deleteRows", label: "Delete &rows", run: deleteRows, enabled: ready, repeatable: true },
    { id: "edit.deleteCols", label: "Delete co&lumns", run: deleteCols, enabled: ready, repeatable: true },
    { id: "edit.moveRowsUp", label: "Move rows up", keys: ["Alt+Shift+ArrowUp"], run: () => moveRows(-1), enabled: ready, repeatable: true },
    { id: "edit.moveRowsDown", label: "Move rows down", keys: ["Alt+Shift+ArrowDown"], run: () => moveRows(1), enabled: ready, repeatable: true },
    { id: "edit.moveColsLeft", label: "Move columns left", keys: ["Alt+Shift+ArrowLeft"], run: () => moveCols(-1), enabled: ready, repeatable: true },
    { id: "edit.moveColsRight", label: "Move columns right", keys: ["Alt+Shift+ArrowRight"], run: () => moveCols(1), enabled: ready, repeatable: true },

    { id: "insert.smart", label: "&Cells…", keys: ["Ctrl+Shift++", "Ctrl+Shift+=", "Ctrl++"], run: insertSmart, enabled: ready, repeatable: true },
    { id: "insert.rowsAbove", label: "&Rows above", run: () => insertRows(false), enabled: ready, repeatable: true },
    { id: "insert.rowsBelow", label: "Rows &below", run: () => insertRows(true), enabled: ready, repeatable: true },
    { id: "insert.colsLeft", label: "Columns &left", run: () => insertCols(false), enabled: ready, repeatable: true },
    { id: "insert.colsRight", label: "Columns r&ight", run: () => insertCols(true), enabled: ready, repeatable: true },
    { id: "insert.rowsMany", label: "Rows (choose how &many)…", run: () => { insertAsk = "rows"; }, enabled: ready },
    { id: "insert.colsMany", label: "Columns (choose how ma&ny)…", run: () => { insertAsk = "cols"; }, enabled: ready },
    { id: "insert.symbol", label: "&Symbol…", run: openSymbols, enabled: ready },

    { id: "format.autofit", label: "&AutoFit column width", run: () => { const r = tab!.rect; void grid?.fitCols(r.col, r.cols); }, enabled: hasTab },
    { id: "format.hideCols", label: "&Hide columns", keys: ["Ctrl+0"], run: hideCols, enabled: hasTab },
    { id: "format.showCols", label: "&Unhide all columns", keys: ["Ctrl+Shift+0", "Ctrl+Shift+)"], run: () => { tab!.columns.showAll(); tab!.bump(); }, enabled: () => !!tab && tab.columns.hiddenCount() > 0 },
    { id: "format.header", label: "First row is &header", run: toggleHeader, enabled: hasTab, checked: () => !!tab?.header },
    { id: "format.makeHeader", label: "&Make this row the header", run: makeHeader, enabled: ready },

    { id: "view.freeze", label: "&Freeze panes at cursor", run: freezeAtCursor, enabled: hasTab },
    { id: "view.freezeRow", label: "Freeze top &row", run: () => { tab!.frozenRows = 1; tab!.frozenCols = 0; }, enabled: hasTab },
    { id: "view.freezeCol", label: "Freeze first &column", run: () => { tab!.frozenCols = 1; tab!.frozenRows = 0; }, enabled: hasTab },
    { id: "view.unfreeze", label: "&Unfreeze panes", run: () => { tab!.frozenRows = 0; tab!.frozenCols = 0; }, enabled: () => !!tab && (tab.frozenRows > 0 || tab.frozenCols > 0) },
    { id: "view.zoomIn", label: "Zoom &in", keys: ["Ctrl+Alt+=", "Ctrl+Alt++"], run: () => prefs.setZoom(prefs.data.zoom + 0.1) },
    { id: "view.zoomOut", label: "Zoom &out", keys: ["Ctrl+Alt+-"], run: () => prefs.setZoom(prefs.data.zoom - 0.1) },
    { id: "view.zoomReset", label: "&Zoom 100%", keys: ["Ctrl+Alt+0"], run: () => prefs.setZoom(1) },
    { id: "view.cellBar", label: "&Expand cell bar", keys: ["Ctrl+Shift+U"], run: () => { cellBarExpanded = !cellBarExpanded; }, checked: () => cellBarExpanded },
    { id: "view.gridLines", label: "&Gridlines", run: () => { prefs.data.gridLines = !prefs.data.gridLines; prefs.save(); }, checked: () => prefs.data.gridLines },
    { id: "view.fontBigger", label: "Larger te&xt", run: () => { prefs.data.fontSize = Math.min(28, prefs.data.fontSize + 1); prefs.save(); } },
    { id: "view.fontSmaller", label: "Smaller tex&t", run: () => { prefs.data.fontSize = Math.max(9, prefs.data.fontSize - 1); prefs.save(); } },
    { id: "view.themeSystem", label: "Theme: &system", run: setTheme("system"), checked: () => prefs.data.theme === "system" },
    { id: "view.themeLight", label: "Theme: &light", run: setTheme("light"), checked: () => prefs.data.theme === "light" },
    { id: "view.themeDark", label: "Theme: &dark", run: setTheme("dark"), checked: () => prefs.data.theme === "dark" },
    ...ACCENTS.map((a) => ({
      id: `view.accent.${a.name.toLowerCase()}`,
      label: `Accent: ${a.name}`,
      run: () => {
        prefs.data.accent = a.value;
        prefs.save();
      },
      checked: () => prefs.data.accent === a.value,
    })),
    ...FONTS.map((f) => ({
      id: `view.font.${f.name.toLowerCase()}`,
      label: `Font: ${f.name}`,
      run: () => {
        prefs.data.font = f.value;
        prefs.save();
      },
      checked: () => prefs.data.font === f.value,
    })),
    { id: "window.next", label: "Next file", keys: ["Ctrl+PageDown", "Ctrl+Tab"], run: () => switchTab(1), enabled: () => tabs.length > 1 },
    { id: "window.prev", label: "Previous file", keys: ["Ctrl+PageUp", "Ctrl+Shift+Tab"], run: () => switchTab(-1), enabled: () => tabs.length > 1 },
    { id: "data.sortAsc", label: "Sort &A → Z", run: () => sortByColumn(false), enabled: ready, repeatable: true },
    { id: "data.sortDesc", label: "Sort &Z → A", run: () => sortByColumn(true), enabled: ready, repeatable: true },
    { id: "data.sort", label: "&Custom sort…", run: openSortDialog, enabled: ready },
    { id: "data.filter", label: "&Filter", keys: ["Ctrl+Shift+L"], run: toggleFilterButtons, enabled: ready, checked: () => !!tab && (tab.filterButtons || tab.info.has_filter) },
    { id: "data.filterMenu", label: "Filter &menu for this column", keys: ["Alt+ArrowDown"], run: () => void altDown(), enabled: ready },
    { id: "format.hideRows", label: "Hide &rows", keys: ["Ctrl+9"], run: () => void hideRows(), enabled: ready, repeatable: true },
    { id: "format.showRows", label: "Unhide rows in the selection", keys: ["Ctrl+Shift+9", "Ctrl+Shift+("], run: () => void showRows(false), enabled: () => ready() && !!tab?.info.hidden_rows },
    { id: "format.showAllRows", label: "Unhide all ro&ws", run: () => void showRows(true), enabled: () => ready() && !!tab?.info.hidden_rows },
    { id: "format.wrap", label: "&Wrap text", run: toggleWrap, enabled: hasTab, checked: () => !!tab && selectedCols(tab).length > 0 && selectedCols(tab).every((c) => tab!.columns.wraps.has(c)) },
    { id: "format.rowHeight", label: "Row &height (lines)…", run: askRowLines, enabled: hasTab },
    { id: "format.fitRows", label: "Fit row height to wrapped text", run: () => grid?.fitRowLines(), enabled: () => !!tab && tab.columns.wraps.size > 0 },
    { id: "format.display", label: "&Display format (numbers, dates, tick boxes)…", run: openFormat, enabled: hasTab },
    { id: "format.bakeDisplay", label: "Write the display format into the cells", run: () => void bakeFormats(), enabled: () => !!tab && !tab.busy && selectedCols(tab).some((c) => tab!.columns.formats.has(c)) },
    { id: "format.clearDisplay", label: "Remove display format", run: () => { if (tab) setFormat(selectedCols(tab), null); }, enabled: () => !!tab && selectedCols(tab).some((c) => tab!.columns.formats.has(c)) },
    { id: "view.banded", label: "Alternating row &colours", run: toggleBanded, enabled: hasTab, checked: () => !!tab?.banded },
    { id: "data.shuffle", label: "Shu&ffle rows (random order)", run: () => void shuffleRows(), enabled: ready, repeatable: true },
    { id: "tools.fillBlanks", label: "&Fill blank cells…", run: () => openTools("fill_blanks"), enabled: ready },
    { id: "tools.countRepeats", label: "&Count repeats in a new column…", run: () => openTools("count_repeats"), enabled: ready },
    { id: "tools.blankRepeats", label: "&Blank out repeats (keep the first)…", run: () => openTools("blank_repeats"), enabled: ready },
    { id: "tools.breaks", label: "Blank &rows between groups…", run: () => openTools("breaks"), enabled: ready },
    { id: "tools.flip", label: "F&lip row order", run: () => void flipRows(), enabled: ready, repeatable: true },
    { id: "tools.deleteHidden", label: "&Delete hidden rows", run: () => void deleteHiddenRows(), enabled: () => ready() && !!tab?.info.hidden_rows },
    { id: "tools.deleteHiddenCols", label: "Delete hidden col&umns", run: () => void deleteHiddenCols(), enabled: ready },
    { id: "tools.reshape", label: "Column to a &table…", run: () => openTools("reshape"), enabled: ready },
    { id: "tools.stack", label: "Columns to &one column…", run: () => openTools("stack"), enabled: ready },
    { id: "tools.sample", label: "Random s&ample…", run: () => openTools("sample"), enabled: ready },
    { id: "tools.selectionTab", label: "Selection to a &new tab", run: () => void selectionToTab(), enabled: ready },
    { id: "tools.watch", label: "&Watch a folder (run a recipe on new files)…", run: () => { watchOpen = true; } },
    { id: "tools.updates", label: "Take &changes from a newer file…", run: () => { updatesOpen = true; }, enabled: ready },
    { id: "tools.compare", label: "Co&mpare with another open file…", run: () => openTools("compare"), enabled: ready },
    { id: "tools.split", label: "S&plit into files…", run: () => openTools("split"), enabled: () => ready() },
    { id: "tools.combine", label: "Combine &files into one…", run: () => openTools("combine"), enabled: hasTab },
    { id: "tools.nextChange", label: "&Next change in this column", keys: ["Ctrl+Alt+PageDown"], run: () => void jumpChange(false), enabled: ready },
    { id: "tools.prevChange", label: "Pre&vious change in this column", keys: ["Ctrl+Alt+PageUp"], run: () => void jumpChange(true), enabled: ready },
    { id: "tools.largest", label: "Go to the lar&gest number", run: () => void jumpExtreme(true), enabled: ready },
    { id: "tools.smallest", label: "Go to the smallest num&ber", run: () => void jumpExtreme(false), enabled: ready },
    { id: "file.revert", label: "Re&vert to saved", run: () => void revertFile(), enabled: () => !!tab?.info.path },
    { id: "file.rename", label: "Rena&me file…", run: () => void renameFile(), enabled: () => !!tab?.info.path && !tab?.info.read_only },
    { id: "file.showFolder", label: "Show in File E&xplorer", run: () => void showInFolder(), enabled: () => !!tab?.info.path },
    { id: "file.copyPath", label: "Cop&y file path", run: () => void copyPath(), enabled: () => !!tab?.info.path },
    { id: "file.saveAll", label: "Save a&ll", keys: ["Ctrl+Alt+S"], run: () => void saveAll(), enabled: () => tabs.some((t) => t.info.modified) },
    { id: "file.closeUnchanged", label: "Close &unchanged tabs", run: () => void closeUnchanged(), enabled: () => tabs.length > 0 },
    { id: "view.valueTypes", label: "Show value t&ypes (numbers, dates)", keys: ["Ctrl+F8"], run: toggleValueTypes, enabled: hasTab, checked: () => !!tab?.valueTypes },
    { id: "view.divider", label: "&Line at each change in this column", run: toggleDivider, enabled: hasTab, checked: () => tab?.divider !== null && tab?.divider === tab?.sel.focus.col },
    { id: "view.scale3", label: "Colour scale (red, yellow, green)", run: () => toggleScale("scale3"), enabled: ready, checked: () => !!tab?.scales.some((s) => s.col === tab?.sel.focus.col && s.kind === "scale3") },
    { id: "view.scale2", label: "Colour scale (white to green)", run: () => toggleScale("scale2"), enabled: ready, checked: () => !!tab?.scales.some((s) => s.col === tab?.sel.focus.col && s.kind === "scale2") },
    { id: "view.bars", label: "Data &bars", run: () => toggleScale("bar"), enabled: ready, checked: () => !!tab?.scales.some((s) => s.col === tab?.sel.focus.col && s.kind === "bar") },
    { id: "view.clearHighlights", label: "Clear all highlight rules", run: clearHighlights, enabled: hasTab },
    { id: "data.clearChecks", label: "Clear all data checks", run: clearChecks, enabled: () => !!tab?.checks.length },
    { id: "view.typeahead", label: "Suggest values while typing", run: () => { prefs.data.typeahead = !prefs.data.typeahead; prefs.save(); say(prefs.data.typeahead ? "Values already in the column are suggested while you type." : "No suggestions while typing."); }, checked: () => prefs.data.typeahead },
    { id: "clean.suggest", label: "Clean-up &suggestions…", run: () => void openSuggestions(), enabled: ready },
    { id: "file.recover", label: "Recover unsaved &work…", run: () => void showRecovery(false) },
    { id: "file.versions", label: "Earlier sa&ves of this file…", run: () => void showVersions(), enabled: () => !!tab?.info.path },
    { id: "data.filterBySel", label: "Filter by &selected value", run: filterBySelection, enabled: ready },
    { id: "data.clearFilter", label: "C&lear filter", run: clearFilter, enabled: () => ready() && !!tab?.info.has_filter },
    { id: "data.reapply", label: "&Reapply filter", keys: ["Ctrl+Alt+L"], run: reapplyFilter, enabled: () => ready() && !!tab?.filter },
    { id: "data.removeDuplicates", label: "Remove &duplicates…", run: () => openOp("remove_duplicates"), enabled: ready },
    { id: "data.showDuplicates", label: "S&how duplicates…", run: () => openOp("show_duplicates"), enabled: ready },
    { id: "data.removeBlankRows", label: "Remove &blank rows", run: () => removeBlanks(false), enabled: ready, repeatable: true },
    { id: "data.removeBlankCols", label: "Remove blan&k columns", run: () => removeBlanks(true), enabled: ready, repeatable: true },
    { id: "data.splitColumn", label: "Split column (t&ext to columns)…", run: () => openOp("split_column"), enabled: ready },
    { id: "data.mergeColumns", label: "Mer&ge columns…", run: () => openOp("merge_columns"), enabled: ready },
    { id: "data.splitRows", label: "Split into ro&ws…", run: () => openOp("split_rows"), enabled: ready },
    { id: "data.mergeRows", label: "Merge r&ows…", run: () => openOp("merge_rows"), enabled: ready },
    { id: "data.sortColsAsc", label: "Sort columns by title A → Z", run: () => sortColumns(false), enabled: ready, repeatable: true },
    { id: "data.sortColsDesc", label: "Sort columns by title Z → A", run: () => sortColumns(true), enabled: ready, repeatable: true },
    { id: "data.pivot", label: "&Pivot table…", run: openPivot, enabled: ready },
    { id: "data.unpivot", label: "U&npivot…", run: () => openOp("unpivot"), enabled: ready },
    { id: "data.join", label: "&Join with another table…", run: () => { joinKind = "join"; }, enabled: ready },
    { id: "data.street", label: "Look up by &street range (precinct, district)…", run: () => { streetOpen = true; }, enabled: ready },
    { id: "data.match", label: "&Match people (spelled differently)…", run: () => { matchOpen = true; }, enabled: ready },
    { id: "data.values", label: "Edit &values (rename, merge, translate)…", run: () => { valuesOpen = true; }, enabled: ready },
    { id: "data.group", label: "&Group figures (count, total, rank)…", run: () => { editKind = "group"; }, enabled: ready },
    { id: "data.arrange", label: "Choose and &order columns…", run: () => { editKind = "arrange"; }, enabled: ready },
    { id: "data.extract", label: "E&xtract into a column…", run: () => { editKind = "extract"; }, enabled: ready },
    { id: "data.build", label: "&Build a column (template or rules)…", run: () => { editKind = "build"; }, enabled: ready },
    { id: "data.append", label: "Appen&d another table…", run: () => { joinKind = "append"; }, enabled: ready },
    { id: "data.form", label: "Form v&iew (one row per page)…", run: () => { if (tab) formRow = tab.sel.focus.row; }, enabled: ready },
    { id: "data.fillExample", label: "Fill by e&xample", keys: ["Ctrl+E"], run: () => void fillByExample(), enabled: ready, repeatable: true },
    { id: "data.splitName", label: "Split names into columns…", run: () => { splitNameOpen = true; }, enabled: ready },
    { id: "clean.tools", label: "&Cleaning tools…", keys: ["Ctrl+Shift+K"], run: () => { cleanTool = "spaces"; }, enabled: ready },
    ...TOOLS.map((tl) => ({ id: `clean.tool.${tl.id}`, label: `${tl.label}…`, run: () => { cleanTool = tl.id; }, enabled: ready })),
    { id: "find.find", label: "&Find…", keys: ["Ctrl+F", "Shift+F5"], run: () => openFind(false), enabled: hasTab },
    { id: "find.replace", label: "&Replace…", keys: ["Ctrl+H"], run: () => openFind(true), enabled: ready },
    { id: "find.next", label: "Find &next", keys: ["F3"], run: () => findAgain(false), enabled: hasTab },
    { id: "find.prev", label: "Find &previous", keys: ["Shift+F3"], run: () => findAgain(true), enabled: hasTab },
    { id: "view.panel", label: "Side &panel", keys: ["Ctrl+F1"], run: () => showPanel(prefs.data.panel ? null : panelTab), checked: () => !!prefs.data.panel },
    { id: "view.panelSummary", label: "Column summary", run: () => showPanel("summary"), checked: () => prefs.data.panel === "summary" },
    { id: "view.panelChart", label: "Chart", run: () => showPanel("chart"), checked: () => prefs.data.panel === "chart" },
    { id: "view.panelFiles", label: "Files in this folder", run: () => showPanel("files"), checked: () => prefs.data.panel === "files" },
    { id: "view.panelActivity", label: "Activity", run: () => showPanel("activity"), checked: () => prefs.data.panel === "activity" },
    { id: "view.fontOther", label: "Font: Other…", run: askFont, checked: () => !FONTS.some((f) => f.value === prefs.data.font) },
    { id: "view.spellcheck", label: "Check &spelling while typing", run: () => { prefs.data.spellcheck = !prefs.data.spellcheck; prefs.save(); }, checked: () => prefs.data.spellcheck },
    { id: "view.highlights", label: "&Highlight rules…", run: () => { highlightOpen = true; }, enabled: ready },
    { id: "view.saveView", label: "Save &view…", run: saveView, enabled: ready },
    { id: "view.applyView", label: "&Apply a saved view…", run: () => { search = "View: "; }, enabled: () => ready() && prefs.data.views.length > 0 },
    { id: "view.saveViewFile", label: "Save view to a file…", run: () => void saveViewFile(), enabled: ready },
    { id: "view.openViewFile", label: "Apply a view from a file…", run: () => void openViewFile(), enabled: ready },
    { id: "file.readOnly", label: "&Lock (read-only)", run: () => void toggleReadOnly(), enabled: () => !!tab && tab.info.indexed, checked: () => !!tab?.info.read_only },
    { id: "data.subtotals", label: "S&ubtotals…", run: () => { subtotalOpen = true; }, enabled: ready },
    { id: "help.search", label: "&Search commands…", keys: ["Alt+Q"], run: () => { search = ""; } },
    { id: "help.shortcuts", label: "&Customize shortcuts…", run: () => { shortcutsOpen = true; } },
    { id: "help.keys", label: "&Keyboard shortcuts", keys: ["Ctrl+/", "F1"], run: () => { keysMouse = false; keysOpen = true; } },
    { id: "help.mouse", label: "&Mouse and drag moves", run: () => { keysMouse = true; keysOpen = true; } },
    { id: "help.about", label: "&About Natendo Data Edit", run: showAbout },
  ];
  reg.add(cmds);
  // Custom keys (taken from the commands' defaults, which are Excel's).
  reg.setOverrides(cleanShortcuts(prefs.data.shortcuts, (id) => !!reg.get(id)));

  const menus: Menu[] = [
    { label: "&File", items: ["file.new", "file.open", "file.recent", "-", "file.save", "file.saveAs", "file.saveAll", "-", "file.import", "file.importClipboard", "file.export", "file.safeCopy", "-", "file.openEncrypted", "file.saveEncrypted", "file.openSqlite", "file.saveSqlite", "-", "file.print", "file.pdf", "file.mail", "-", "file.layout", "file.reopenLines", "file.readOnly", "file.private", "file.privateFolder", "file.revert", "-", "file.rename", "file.showFolder", "file.copyPath", "-", "file.recover", "file.versions", "-", "file.options", "-", "file.close", "file.closeUnchanged", "file.quit"] },
    {
      label: "&Edit",
      items: [
        "edit.undo", "edit.redo", "edit.repeat", "-", "edit.cut", "edit.copy", "edit.paste", "edit.moveCut", "edit.cancelCut", "edit.pasteSpecial", "edit.clear", "-",
        "edit.fillDown", "edit.fillRight", "edit.fillUp", "edit.fillLeft", "edit.fillSeries", "-", "edit.cell", "edit.today", "edit.pickDate", "edit.now", "-", "edit.selectAll", "edit.goto", "-", "edit.note", "edit.changes", "-",
        "edit.deleteRows", "edit.deleteCols", "edit.moveRowsUp", "edit.moveRowsDown", "edit.moveColsLeft", "edit.moveColsRight",
      ],
    },
    {
      label: "&View",
      items: [
        "view.freeze", "view.freezeRow", "view.freezeCol", "view.unfreeze", "-", "view.zoomIn", "view.zoomOut", "view.zoomReset", "-",
        "view.panel", "view.panelSummary", "view.panelChart", "view.panelFiles", "view.panelActivity", "-",
        "view.showEdits", "view.highlights", "view.scale3", "view.scale2", "view.bars", "view.clearHighlights", "-",
        "view.saveView", "view.applyView", "view.saveViewFile", "view.openViewFile", "-",
        "view.cellBar", "view.gridLines", "view.banded", "view.valueTypes", "view.divider", "view.typeahead", "view.spellcheck", "view.fontBigger", "view.fontSmaller", "-", "view.themeSystem", "view.themeLight", "view.themeDark", "-",
        ...ACCENTS.map((a) => `view.accent.${a.name.toLowerCase()}`), "-", ...FONTS.map((f) => `view.font.${f.name.toLowerCase()}`), "view.fontOther",
      ],
    },
    { label: "&Insert", items: ["insert.rowsAbove", "insert.rowsBelow", "insert.rowsMany", "-", "insert.colsLeft", "insert.colsRight", "insert.colsMany", "-", "insert.symbol"] },
    { label: "F&ormat", items: ["format.header", "format.makeHeader", "-", "format.display", "format.bakeDisplay", "format.clearDisplay", "format.wrap", "format.rowHeight", "format.fitRows", "-", "format.autofit", "format.hideCols", "format.showCols", "-", "format.hideRows", "format.showRows", "format.showAllRows"] },
    {
      label: "&Data",
      items: [
        "data.sortAsc", "data.sortDesc", "data.sort", "data.sortColsAsc", "data.sortColsDesc", "data.shuffle", "-",
        "data.filter", "data.filterMenu", "data.filterBySel", "data.clearFilter", "data.reapply", "-",
        "data.removeDuplicates", "data.showDuplicates", "data.removeBlankRows", "data.removeBlankCols", "-",
        "data.splitColumn", "data.mergeColumns", "data.splitRows", "data.mergeRows", "-",
        "data.pivot", "data.unpivot", "data.subtotals", "data.form", "-",
        "data.compute", "data.checks", "data.fieldCheck", "data.unmarkChecks", "data.clearChecks", "-",
        "data.fillExample", "data.splitName", "-",
        "data.join", "data.match", "data.street", "data.append", "-",
        "data.values", "data.group", "data.arrange", "data.extract", "data.build", "-",
        "find.find", "find.replace", "find.next", "find.prev",
      ],
    },
    {
      label: "Clea&n",
      items: [
        "clean.suggest", "clean.tools", "-",
        ...TOOLS.flatMap((tl, i) => (i > 0 && TOOLS[i - 1]!.group !== tl.group ? ["-", `clean.tool.${tl.id}`] : [`clean.tool.${tl.id}`])), "-",
        "data.fillExample", "data.splitName",
      ],
    },
    {
      label: "&Tools",
      items: [
        "tools.fillBlanks", "tools.countRepeats", "tools.blankRepeats", "tools.breaks", "-",
        "tools.flip", "tools.deleteHidden", "tools.deleteHiddenCols", "-",
        "tools.reshape", "tools.stack", "tools.sample", "tools.selectionTab", "-",
        "tools.compare", "tools.updates", "tools.split", "tools.combine", "tools.searchFolder", "tools.recipe", "tools.watch", "-",
        "tools.nextChange", "tools.prevChange", "tools.largest", "tools.smallest", "-",
        "tools.findPrivate",
      ],
    },
    { label: "&Help", items: ["help.search", "help.keys", "help.mouse", "help.shortcuts", "-", "help.logs", "help.about"] },
  ];

  // Command search shows each command's menu and its README description.
  const menuOfId = new Map<string, string>();
  for (const m of menus) for (const it of m.items) if (it !== "-" && !menuOfId.has(it)) menuOfId.set(it, m.label.replace(/&/g, ""));
  const searchDocs = featureDocs(readmeBullets(readme), FEATURE_LINKS, (id) => (reg.get(id)?.label ?? id).replace(/&/g, ""));

  const contextItems: Record<string, string[]> = {
    cell: ["edit.cut", "edit.copy", "edit.paste", "edit.moveCut", "edit.cancelCut", "edit.pasteSpecial", "edit.clear", "-", "data.filterBySel", "data.sortAsc", "data.sortDesc", "-", "insert.rowsAbove", "insert.rowsBelow", "edit.deleteRows", "-", "insert.colsLeft", "insert.colsRight", "edit.deleteCols", "-", "edit.fillDown", "edit.fillRight"],
    row: ["edit.cut", "edit.copy", "edit.paste", "edit.moveCut", "edit.cancelCut", "-", "insert.rowsAbove", "insert.rowsBelow", "insert.rowsMany", "edit.deleteRows", "-", "edit.moveRowsUp", "edit.moveRowsDown", "-", "format.hideRows", "format.showRows", "-", "data.shuffle", "-", "format.makeHeader", "-", "edit.clear"],
    col: ["edit.cut", "edit.copy", "edit.paste", "edit.moveCut", "edit.cancelCut", "-", "data.sortAsc", "data.sortDesc", "data.filterMenu", "-", "insert.colsLeft", "insert.colsRight", "insert.colsMany", "edit.deleteCols", "-", "edit.moveColsLeft", "edit.moveColsRight", "-", "format.display", "format.wrap", "format.autofit", "format.hideCols", "format.showCols", "-", "edit.clear"],
    corner: ["edit.selectAll", "edit.copy", "-", "format.header", "format.showCols"],
  };

  const toolbar: (string | "-")[] = [
    "file.new", "file.open", "file.save", "-", "edit.undo", "edit.redo", "-", "edit.cut", "edit.copy", "edit.paste", "-",
    "insert.rowsAbove", "insert.rowsBelow", "edit.deleteRows", "-", "insert.colsLeft", "insert.colsRight", "edit.deleteCols", "-",
    "format.header", "view.freeze", "-", "data.sortAsc", "data.sortDesc", "data.filter", "find.find", "-",
    "view.zoomOut", "view.zoomIn", "view.cellBar",
  ];
  const icons: Record<string, string> = {
    "file.new": "new", "file.open": "open", "file.save": "save", "edit.undo": "undo", "edit.redo": "redo",
    "edit.cut": "cut", "edit.copy": "copy", "edit.paste": "paste", "insert.rowsAbove": "rowAbove", "insert.rowsBelow": "rowBelow",
    "edit.deleteRows": "rowDelete", "insert.colsLeft": "colLeft", "insert.colsRight": "colRight", "edit.deleteCols": "colDelete",
    "format.header": "header", "view.freeze": "freeze", "data.sortAsc": "sortAsc", "data.sortDesc": "sortDesc",
    "data.filter": "filter", "find.find": "find", "view.zoomOut": "zoomOut", "view.zoomIn": "zoomIn", "view.cellBar": "panel",
  };

  function plain(label: string): string {
    return label.replace("&", "");
  }
  function accessKey(label: string): string | null {
    const i = label.indexOf("&");
    return i >= 0 && i + 1 < label.length ? label[i + 1]!.toLowerCase() : null;
  }
  function labelParts(label: string): [string, string, string] {
    const i = label.indexOf("&");
    if (i < 0) return [label, "", ""];
    return [label.slice(0, i), label[i + 1] ?? "", label.slice(i + 2)];
  }

  function switchTab(dir: number): void {
    if (tabs.length === 0) return;
    active = (active + dir + tabs.length) % tabs.length;
    void tick().then(() => grid?.focus());
  }

  async function runCmd(id: string): Promise<void> {
    openMenu = null;
    context = null;
    try {
      await reg.run(id);
    } catch (e) {
      say(errorText(e));
    }
    if (!dialog && document.activeElement === document.body) grid?.focus();
  }

  function showAbout(): void {
    void ask(
      "Natendo Data Edit",
      `Version ${info?.version ?? "?"} (engine ${info?.engine_version ?? "?"}). An editor for CSV, TSV and other delimited text files of any size.`,
      [{ id: "ok", label: "OK", primary: true }],
      "ok",
    );
  }

  // ---- keyboard ----------------------------------------------------------------------
  function inTextField(el: EventTarget | null): boolean {
    if (el instanceof HTMLElement && el.dataset.gridFocus !== undefined) return false;
    return el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement;
  }

  const SAFE_IN_FIELDS = /^(file\.|view\.zoom|window\.|help\.|find\.)/;

  function menuKey(e: KeyboardEvent): boolean {
    const list = openMenu !== null ? menus[openMenu]?.items : context?.items;
    if (!list) return false;
    const items = list.filter((x) => x !== "-");
    const idx = openMenu !== null ? menuIndex : (context?.index ?? 0);
    const setIdx = (i: number) => {
      const n = (i + items.length) % items.length;
      if (openMenu !== null) menuIndex = n;
      else if (context) context.index = n;
    };
    e.preventDefault();
    switch (e.key) {
      case "Escape":
      case "Alt":
        openMenu = null;
        context = null;
        grid?.focus();
        break;
      case "ArrowDown":
        setIdx(idx + 1);
        break;
      case "ArrowUp":
        setIdx(idx - 1);
        break;
      case "ArrowRight":
        if (openMenu !== null) {
          openMenu = (openMenu + 1) % menus.length;
          menuIndex = 0;
        }
        break;
      case "ArrowLeft":
        if (openMenu !== null) {
          openMenu = (openMenu - 1 + menus.length) % menus.length;
          menuIndex = 0;
        }
        break;
      case "Enter":
      case " ": {
        const id = items[idx];
        if (id && reg.enabled(id)) void runCmd(id);
        break;
      }
      default: {
        const k = e.key.toLowerCase();
        const hitId = items.find((id) => accessKey(reg.get(id)?.label ?? "") === k);
        if (hitId && reg.enabled(hitId)) void runCmd(hitId);
      }
    }
    return true;
  }

  let altAlone = false;

  function onKeyDown(e: KeyboardEvent): void {
    if (dialog || sortDialog || filterMenu || opDialog || formRow !== null || symbols || saveAsReq || exportOpen || importing || joinKind) return;
    if (cleanTool || splitNameOpen || insertAsk || search !== null || shortcutsOpen || highlightOpen || subtotalOpen || pivotDoc !== null || prompt) return;
    if ((openMenu !== null || context) && menuKey(e)) return;
    altAlone = e.key === "Alt";
    const field = inTextField(e.target);
    // Outside text boxes, an Alt the event forgot still counts (see
    // watchAlt); inside one only for the search shortcut, so typing is never
    // taken for a shortcut.
    const loose = keyOfLoose(e);
    const key = !field || reg.forKey(loose)?.id === "help.search" ? loose : keyOf(e);

    // Alt+letter opens a menu (Excel-style access keys).
    if ((field ? e.altKey : altOf(e)) && !e.ctrlKey && !e.shiftKey && e.key.length === 1) {
      const m = menus.findIndex((x) => accessKey(x.label) === e.key.toLowerCase());
      if (m >= 0) {
        e.preventDefault();
        openMenu = m;
        menuIndex = 0;
        menuByKeyboard = true;
        return;
      }
    }

    const gridFocused = (e.target as HTMLElement | null)?.dataset?.gridFocus !== undefined;

    // Ctrl+V: let the browser deliver a paste event (handled below).
    if (key === "Ctrl+V") return;

    const cmd = reg.forKey(key);
    if (cmd && (!field || SAFE_IN_FIELDS.test(cmd.id))) {
      e.preventDefault();
      void runCmd(cmd.id);
      return;
    }
    if (!gridFocused || !tab || !grid) return;
    const t = tab;

    const extend = e.shiftKey;
    const last = Math.max(t.firstRow, t.info.rows - 1);
    const lastCol = Math.max(0, t.info.cols - 1);
    const f = t.sel.focus;
    const to = (row: number, col: number) => {
      if (extend) grid!.select(t.sel.anchor, { row, col });
      else grid!.select({ row, col });
    };
    switch (e.key) {
      case "ArrowUp":
        e.preventDefault();
        if (e.ctrlKey) void jump("up", extend);
        else grid.move(-1, 0, extend);
        return;
      case "ArrowDown":
        e.preventDefault();
        if (e.ctrlKey) void jump("down", extend);
        else grid.move(1, 0, extend);
        return;
      case "ArrowLeft":
        e.preventDefault();
        if (e.ctrlKey) void jump("left", extend);
        else grid.move(0, -1, extend);
        return;
      case "ArrowRight":
        e.preventDefault();
        if (e.ctrlKey) void jump("right", extend);
        else grid.move(0, 1, extend);
        return;
      case "PageDown":
        e.preventDefault();
        if (altOf(e)) grid.move(0, 10, extend);
        else grid.move(grid.pageRows(), 0, extend);
        return;
      case "PageUp":
        e.preventDefault();
        if (altOf(e)) grid.move(0, -10, extend);
        else grid.move(-grid.pageRows(), 0, extend);
        return;
      case "Home":
        e.preventDefault();
        if (e.ctrlKey) to(t.firstRow, 0);
        else to(f.row, 0);
        return;
      case "End":
        e.preventDefault();
        if (e.ctrlKey) to(last, lastCol);
        else to(f.row, lastCol);
        return;
      case "Enter":
        e.preventDefault();
        grid.move(e.shiftKey ? -1 : 1, 0, false);
        return;
      case "Tab":
        e.preventDefault();
        grid.move(0, e.shiftKey ? -1 : 1, false);
        return;
      case "Backspace":
        e.preventDefault();
        void grid.startEdit("");
        return;
      case "Escape":
        if (t.cut) cancelCut();
        else cutEpoch++;
        grid.select(f);
        return;
    }
    if (key === "Shift+Space") {
      e.preventDefault();
      grid.select({ row: t.sel.anchor.row, col: 0 }, { row: f.row, col: lastCol }, false);
      return;
    }
    if (key === "Ctrl+Space") {
      e.preventDefault();
      grid.select({ row: t.firstRow, col: t.sel.anchor.col }, { row: last, col: f.col }, false);
      return;
    }
    // Typed characters reach the grid's hidden text box and start editing.
  }

  function onKeyUp(e: KeyboardEvent): void {
    // Pressing and releasing Alt alone focuses the menu bar, as in Excel.
    if (e.key === "Alt" && altAlone && !dialog && openMenu === null) {
      e.preventDefault();
      openMenu = 0;
      menuIndex = 0;
      menuByKeyboard = true;
    }
    altAlone = false;
  }

  function onPaste(e: ClipboardEvent): void {
    if (dialog || opDialog || formRow !== null || symbols || saveAsReq || exportOpen || importing || joinKind || inTextField(e.target) || !tab) return;
    const text = e.clipboardData?.getData("text/plain");
    if (text === undefined) return;
    e.preventDefault();
    void pasteText(text);
  }

  // Development builds only: let browser tests inspect state.
  if (import.meta.env.DEV) {
    (window as unknown as { __nde: unknown }).__nde = {
      get tab() {
        return tab;
      },
      get grid() {
        return grid;
      },
    };
  }

  // ---- computed columns, data checks --------------------------------------------
  async function runCompute(spec: ComputeSpec): Promise<void> {
    computeOpen = false;
    const t = tab;
    const rep = await runOp({ op: "compute", spec }, "Computing");
    if (!rep || !t) return;
    if (spec.target === null && spec.at !== null) t.columns.shift(spec.at, 1);
    const errs = rep.errors ? ` ${plural(rep.errors, "cell")} came out as errors (like #VALUE!).` : "";
    say(`Computed ${plural(rep.cells, "cell")}.${errs} The values are saved as text. Undo (Ctrl+Z) takes it back.`);
    void tick().then(() => grid?.focus());
  }

  function keepChecks(checks: Check[], mark: boolean): void {
    const t = tab;
    if (!t) return;
    t.checks = checks;
    t.checksMarked = mark && checks.length > 0;
    t.checkVersion++;
    t.bump();
  }

  async function showProblems(spec: CheckSpec, passing = false): Promise<void> {
    checksOpen = false;
    fieldCheckOpen = false;
    const t = tab;
    if (!t) return;
    if (!passing && spec.checks.length === t.checks.length) keepChecks(spec.checks, true);
    const rep = await runOp({ op: "show_problems", spec, passing }, "Filtering");
    if (!rep) return;
    t.filter = null;
    const what = passing ? "that pass" : "with problems";
    if (rep.rows) say(`Showing the ${plural(rep.rows, "row")} ${what}. Click “Filtered ✕” to show every row again.`);
    else {
      say(passing ? "No row passes every check." : "No problems found.");
      void clearFilter();
    }
    void tick().then(() => grid?.focus());
  }

  async function sortFailing(spec: CheckSpec): Promise<void> {
    fieldCheckOpen = false;
    const rep = await runOp({ op: "sort_problems", spec }, "Checking");
    if (!rep) return;
    say(rep.groups ? `Put the ${plural(rep.groups, "row")} that fail first (in their order). Undo (Ctrl+Z) takes it back.` : "No row fails, so the order stays.");
    void tick().then(() => grid?.focus());
  }

  async function addCheckColumn(spec: CheckSpec): Promise<void> {
    fieldCheckOpen = false;
    const rep = await runOp({ op: "check_column", spec, title: "Check result" }, "Checking");
    if (!rep) return;
    say(`Added a “Check result” column: OK, or the checks the row fails (${plural(rep.groups, "row")} fail). Sort or filter it like any column; Undo (Ctrl+Z) takes it back.`);
    void tick().then(() => grid?.focus());
  }

  // ---- printing and PDF -------------------------------------------------------------
  /** Columns shown (hidden ones are not printed). */
  function shownCols(t: Tab): number[] {
    const out: number[] = [];
    for (let c = 0; c < t.info.cols && out.length < 16_384; c++) if (!t.columns.isHidden(c)) out.push(c);
    return out;
  }

  /** Rows of text for these columns (viewport reads, 1,024 characters a cell). */
  async function fetchRows(t: Tab, first: number, count: number, cols: number[]): Promise<string[][]> {
    if (!cols.length || count <= 0) return [];
    const c0 = cols[0]!;
    const c1 = cols[cols.length - 1]! + 1;
    const width = c1 - c0;
    const per = Math.max(1, Math.floor(20_000 / width));
    const out: string[][] = [];
    for (let r = first; r < first + count; r += per) {
      const n = Math.min(per, first + count - r);
      const v = await backend().viewport(t.id, r, n, c0, width, 1024);
      for (let i = 0; i < v.rows; i++) out.push(cols.map((c) => v.cells[i * v.cols + (c - c0)] ?? ""));
      if (v.rows < n) break;
    }
    return out;
  }

  /** Open a folder search result: its file (or its tab) at the cell. */
  async function openFolderHit(h: FolderHit): Promise<void> {
    try {
      const o = await backend().openFolderHit(h.path);
      const existing = o.doc ? tabs.findIndex((t) => t.id === o.doc!.id) : -1;
      if (existing >= 0) active = existing;
      else opened(o);
      if (!o.doc) return;
      folderSearchOpen = false;
      await tick();
      const t = tab;
      if (!t || t.id !== o.doc.id) return;
      if (t.info.filtered) {
        say(`Found at row ${(h.row + 1).toLocaleString()}; a filter is on, so clear it to see that row.`);
        return;
      }
      grid?.select({ row: h.row, col: h.col });
      grid?.focus();
    } catch (e) {
      say(errorText(e));
    }
  }

  async function openChanges(): Promise<void> {
    const t = tab;
    if (!t) return;
    changes = null;
    changesOpen = true;
    try {
      changes = await runJob("Listing your changes", () => backend().changeList(t.id));
      // Stopped: runJob said so.
      if (!changes) changesOpen = false;
    } catch (e) {
      changesOpen = false;
      say(errorText(e));
    }
  }

  async function openRecipes(): Promise<void> {
    const t = tab;
    if (!t) return;
    recipeCurrent = null;
    recipeFile = null;
    recipeOpen = true;
    try {
      recipeCurrent = await backend().recipeGet(t.id, t.header);
    } catch (e) {
      recipeOpen = false;
      say(errorText(e));
    }
  }

  async function saveRecipe(name: string): Promise<void> {
    const t = tab;
    if (!t) return;
    if (recipeCurrent?.holds_values.some(Boolean) && !(await privacyGate("The recipe file keeps values from it (filter ticks, texts, examples) and its column titles."))) return;
    try {
      const path = await backend().recipeSave(t.id, t.header, name);
      if (path !== null) say(`Saved the recipe as ${path}. Open it from Tools on another file, or run it with natendo-cli run.`);
    } catch (e) {
      say(errorText(e));
    }
  }

  async function openRecipeFile(): Promise<void> {
    const t = tab;
    if (!t) return;
    try {
      const r = await backend().recipeOpen(t.id);
      if (r) recipeFile = r;
    } catch (e) {
      say(errorText(e));
    }
  }

  async function runRecipe(force: boolean, phrase: string): Promise<void> {
    const t = tab;
    const r = recipeFile;
    if (!t || !r) return;
    recipeOpen = false;
    try {
      const info = await runJob("Running the recipe", () => backend().recipeRun(t.id, withPhrase(r.recipe, phrase), localNow(), force));
      if (info) t.update(info, true);
      say(`Ran the recipe: ${plural(r.recipe.steps.length, "step")}. Undo each with Ctrl+Z.`);
    } catch (e) {
      say(`${errorText(e)} Steps before it stay done; undo them with Ctrl+Z.`);
    }
    recipeFile = null;
    grid?.focus();
  }

  async function saveChangeReport(): Promise<void> {
    const t = tab;
    if (!t) return;
    if (!(await privacyGate("The report writes the old and new value of every changed cell to a file."))) return;
    try {
      const n = await runJob("Writing the list of changes", () => backend().saveChangeReport(t.id));
      if (n !== null) say(`Saved the list of changes (${plural(n, "change")}) as a CSV report.`);
    } catch (e) {
      say(errorText(e));
    }
  }

  function setShowEdits(on: boolean): void {
    const t = tab;
    if (!t) return;
    t.showEdits = on;
    t.bump();
    say(on ? "Cells that differ from the file now have an orange corner mark." : "Edit marks are off.");
  }

  function goToChange(row: number, col: number): void {
    const t = tab;
    if (!t) return;
    if (t.info.filtered) {
      say("A filter is on: clear it to go to this cell.");
      return;
    }
    changesOpen = false;
    grid?.select({ row, col });
    grid?.focus();
  }

  async function openMail(): Promise<void> {
    if (!tab) return;
    mailProblem = "";
    try {
      mailSheets = await backend().labelSheets();
    } catch (e) {
      say(errorText(e));
    }
  }

  async function saveMail(spec: import("./lib/mail").MailSpec): Promise<void> {
    const t = tab;
    if (!t) return;
    mailProblem = "";
    if (!(await privacyGate("Printed labels and letters can be read by anyone who picks them up."))) return;
    try {
      const rep = await runJob(spec.kind === "labels" ? "Making labels" : "Making letters", () => backend().saveMail(t.id, spec));
      if (!rep) return;
      mailSheets = null;
      say(
        `Saved ${plural(rep.rows, spec.kind === "labels" ? "label" : "letter")} on ${plural(rep.pages, "page")} as a PDF.` +
          (rep.cut ? ` ${plural(rep.cut, "line")} did not fit and were cut short; try a smaller text size or fewer lines.` : ""),
      );
    } catch (e) {
      mailProblem = errorText(e);
    }
  }

  async function openPrint(pdf: boolean): Promise<void> {
    const t = tab;
    if (!t) return;
    if (!(await privacyGate(pdf ? "A PDF is a copy that is easy to send on." : "Printed pages can be read by anyone who picks them up."))) return;
    const cols = shownCols(t);
    const all = shownTitles(t);
    const titles = t.header ? cols.map((c) => all[c] ?? "") : null;
    const first = t.firstRow;
    const r = t.rect;
    const selFirst = Math.max(first, r.row);
    const selRows = Math.max(0, Math.min(t.info.rows, r.row + r.rows) - selFirst);
    let sample: string[][] = [];
    try {
      sample = await fetchRows(t, first, Math.min(60, t.info.rows - first), cols);
    } catch (e) {
      say(errorText(e));
      return;
    }
    pageSetup ??= defaultPage(t.info.name);
    printReq = { pdf, titles, sample, cols, allRows: Math.max(0, t.info.rows - first), selRows, selFirst };
  }

  async function doPrint(setup: PageSetup, selection: boolean): Promise<void> {
    const req = printReq;
    const t = tab;
    printReq = null;
    if (!req || !t) return;
    pageSetup = setup;
    const first = selection ? req.selFirst : t.firstRow;
    const count = selection ? req.selRows : req.allRows;
    if (count > PRINT_MAX_ROWS || count * req.cols.length > PRINT_MAX_CELLS) {
      say("That is too much to print at once; use Save as PDF instead.");
      return;
    }
    say("Getting the pages ready…");
    let rows: string[][];
    try {
      rows = await fetchRows(t, first, count, req.cols);
    } catch (e) {
      say(errorText(e));
      return;
    }
    const html = printHtml(req.titles, rows, setup, getComputedStyle(document.body).fontFamily || "sans-serif");
    const frame = document.createElement("iframe");
    frame.setAttribute("aria-hidden", "true");
    frame.style.cssText = "position:fixed;right:0;bottom:0;width:0;height:0;border:0;visibility:hidden";
    frame.srcdoc = html;
    frame.onload = () => {
      const w = frame.contentWindow;
      if (!w) return;
      w.addEventListener("afterprint", () => setTimeout(() => frame.remove(), 1000));
      setTimeout(() => frame.remove(), 10 * 60_000);
      say("");
      w.focus();
      w.print();
    };
    document.body.appendChild(frame);
  }

  async function doPdf(setup: PageSetup, selection: boolean): Promise<void> {
    const req = printReq;
    const t = tab;
    printReq = null;
    if (!req || !t) return;
    pageSetup = setup;
    const hidden = req.cols.length < t.info.cols;
    const page: PageSetup = {
      ...setup,
      rows: selection ? [req.selFirst, req.selRows] : null,
      cols: hidden ? req.cols : null,
    };
    const o = { ...exportDefaults(t.header), format: "pdf" as const, page };
    try {
      const r = await runJob("Writing the PDF", () => backend().exportFile(t.id, o));
      if (r) say(`Saved a PDF of ${plural(r.sheets, "page")} (${plural(r.rows, "row")}).`);
    } catch (e) {
      say(errorText(e));
    } finally {
      void tick().then(() => grid?.focus());
    }
  }

  // ---- files from Windows, a second launch, drops and the agent link ------------
  async function takeOpened(): Promise<void> {
    let list;
    try {
      list = await backend().takeOpened();
    } catch {
      return;
    }
    for (const o of list) {
      if (o.existing !== null) {
        const i = tabs.findIndex((t) => t.id === o.existing);
        if (i >= 0) active = i;
      } else if (o.error) say(`${o.name}: ${o.error}`);
      else if (o.opened) opened(o.opened);
    }
  }

  // ---- loading progress -------------------------------------------------------------
  onMount(() => {
    const unwatchAlt = watchAlt(window);
    backend().onOpened(() => void takeOpened());
    void takeOpened();
    backend().onAgentGoto((g) => {
      const i = tabs.findIndex((t) => t.id === g.doc);
      if (i < 0) return;
      active = i;
      void tick().then(() => grid?.select({ row: Math.min(g.row, tabs[i]!.info.rows - 1), col: Math.min(g.col, tabs[i]!.info.cols - 1) }));
    });
    if (prefs.data.agentLink) void backend().setAgentLink(true).catch(() => {});
    void backend()
      .appInfo()
      .then((i) => (info = i));
    void loadRecent();
    backend().onCloseRequested(() => void quit());
    // Work left unsaved by a run that stopped is offered back once, at start.
    void showRecovery(true);
    void backend().versionsKeep(prefs.data.keepEarlierSaves).catch(() => {});
    const autosaver = window.setInterval(() => void autosaveTick(), 15_000);
    const timer = window.setInterval(async () => {
      for (const t of tabs) {
        if (t.info.indexed && !t.info.error) continue;
        if (t.info.error) continue;
        try {
          const d = await backend().docInfo(t.id);
          t.update(d, false);
        } catch {
          // Closed meanwhile.
        }
      }
    }, 200);
    const close = (e: MouseEvent) => {
      if (!(e.target as HTMLElement).closest(".menu, .menubar, .context")) {
        openMenu = null;
        context = null;
      }
    };
    window.addEventListener("mousedown", close);
    return () => {
      clearInterval(timer);
      clearInterval(autosaver);
      window.removeEventListener("mousedown", close);
      unwatchAlt();
    };
  });

  $effect(() => {
    const root = document.documentElement;
    if (prefs.data.theme === "system") root.removeAttribute("data-theme");
    else root.dataset.theme = prefs.data.theme;
    root.style.setProperty("--accent", prefs.data.accent);
    root.style.setProperty("--accent-ink", "#ffffff");
    root.style.setProperty("--ui-font", prefs.data.font);
  });

  function openContext(x: number, y: number, area: "cell" | "row" | "col" | "corner"): void {
    openMenu = null;
    // The cut items show only while something is cut.
    const cutItems = new Set(["edit.moveCut", "edit.cancelCut"]);
    const items = (contextItems[area] ?? []).filter((i) => tab?.cut || !cutItems.has(i));
    context = { x: Math.min(x, window.innerWidth - 260), y: Math.min(y, window.innerHeight - 420), items, index: 0 };
  }

  const notices = $derived.by(() => {
    const t = tab;
    type Notice = { text: string; action?: string; run?: () => void };
    if (!t || t.dismissed) return [] as Notice[];
    const d = t.info;
    const out: Notice[] = [];
    if (d.layout_confidence !== null && d.layout_confidence < 0.6 && d.path)
      out.push({ text: "Not sure how this file's columns are separated. Please check the layout.", action: "Choose layout…", run: chooseLayout });
    if (d.preamble_lines > 0)
      out.push({
        text: `${d.preamble_lines} line${d.preamble_lines === 1 ? "" : "s"} above the table (a title or notes) ${d.preamble_lines === 1 ? "is" : "are"} set aside and kept as ${d.preamble_lines === 1 ? "it is" : "they are"} when saving.`,
        action: d.path ? "Change…" : undefined,
        run: chooseLayout,
      });
    if (d.error) out.push({ text: `Reading stopped: ${d.error}` });
    if (d.line_by_line) out.push({ text: "Opened line by line: a cell that spans several lines shows as separate rows, and saving over this file keeps them split.", action: "Read normally", run: () => void reopenLineByLine() });
    else if (d.unclosed_quotes || d.long_quoted_cells)
      out.push({
        text: d.line_bound_from !== null
          ? `A quote mark that never closes may have swallowed rows (from row ${d.line_bound_from + 1}).`
          : "A quoted cell spans many lines; a stray quote mark may have joined rows together.",
        action: "Reopen reading line by line",
        run: () => void reopenLineByLine(),
      });
    if (d.lossy) out.push({ text: "Some characters could not be read in this file's encoding and show as �. Saving will ask before replacing them." });
    if (d.invalid_bytes) out.push({ text: "Some bytes are not valid text and show as �. They are kept unchanged unless you edit their row." });
    if (d.repair_count > 0 && !d.unclosed_quotes && !d.long_quoted_cells)
      out.push({ text: `${d.repair_count.toLocaleString()} row${d.repair_count === 1 ? " has" : "s have"} quote marks in unexpected places; they were read as plain text.` });
    return out;
  });

  const status = $derived.by(() => {
    const t = tab;
    if (!t) return null;
    const d = t.info;
    const r = t.rect;
    const delim = separatorName(d);
    return {
      size: d.filtered
        ? `${Math.max(0, d.rows - t.firstRow).toLocaleString()} of ${Math.max(0, d.total_rows - t.firstRow).toLocaleString()} rows shown × ${d.cols.toLocaleString()} columns`
        : `${Math.max(0, d.rows - t.firstRow).toLocaleString()} ${d.rows - t.firstRow === 1 ? "row" : "rows"} × ${d.cols.toLocaleString()} ${d.cols === 1 ? "column" : "columns"}`,
      hidden: d.hidden_rows,
      filter: d.has_filter || d.filter_on_deleted
        ? [
            ...(t.filter?.columns ?? []).map((c) => `${columnTitle(t, c.col)} ${describeRule(c.rule)}`),
            ...(d.filter_on_deleted ? ["a rule on a deleted column (Undo brings the column back)"] : []),
            ...(!t.filter?.columns.length && !d.filter_on_deleted ? ["rows with duplicates"] : []),
          ].join("; ")
        : "",
      sel: cellCount(r) > 1 ? `${r.rows.toLocaleString()} × ${r.cols.toLocaleString()} selected` : "",
      format: `${d.encoding}${d.bom ? " (BOM)" : ""} · ${delim} separated · ${d.line_break}${d.mixed_line_breaks ? " (mixed)" : ""}`,
    };
  });
</script>

<svelte:window onkeydown={onKeyDown} onkeyup={onKeyUp} onpaste={onPaste} />

<div class="shell">
  <nav class="menubar" aria-label="Menu">
    <img class="logo" src="/favicon.svg" alt="" width="18" height="18" />
    {#each menus as m, i (m.label)}
      {@const [a, k, b] = labelParts(m.label)}
      <div class="mwrap">
      <button
        type="button"
        class="menu-title"
        class:open={openMenu === i}
        onmousedown={(e) => {
          e.preventDefault();
          openMenu = openMenu === i ? null : i;
          menuIndex = -1;
          menuByKeyboard = false;
          context = null;
        }}
        onmouseenter={() => {
          if (openMenu !== null && openMenu !== i) {
            openMenu = i;
            menuIndex = -1;
          }
        }}>{a}<u>{k}</u>{b}</button
      >
      {#if openMenu === i}
        <div class="menu" role="menu">
          {#each m.items as id, j (j)}
            {#if id === "-"}
              <div class="sep"></div>
            {:else}
              {@const c = reg.get(id)}
              {@const pos = m.items.filter((x) => x !== "-").indexOf(id)}
              {#if c}
                {@const [p, q, s] = labelParts(c.label)}
                <button
                  type="button"
                  role="menuitem"
                  class="item"
                  class:hot={menuIndex === pos}
                  disabled={!reg.enabled(id)}
                  onmouseenter={() => (menuIndex = pos)}
                  onclick={() => void runCmd(id)}
                >
                  <span class="check">{c.checked?.() ? "✓" : ""}</span>
                  <span class="label">{p}<u class:show={menuByKeyboard}>{q}</u>{s}</span>
                  <span class="keys">{c.keys?.[0] ?? ""}</span>
                </button>
              {/if}
            {/if}
          {/each}
        </div>
      {/if}
      </div>
    {/each}
  </nav>

  <div class="toolbar" role="toolbar" aria-label="Tools">
    {#each toolbar as id, i (i)}
      {#if id === "-"}
        <span class="tsep"></span>
      {:else}
        {@const c = reg.get(id)}
        <button
          type="button"
          class="tool"
          class:on={c?.checked?.()}
          title={`${plain(c?.label ?? "")}${c?.keys?.[0] ? ` (${c.keys[0]})` : ""}`}
          aria-label={plain(c?.label ?? id)}
          disabled={!reg.enabled(id)}
          onmousedown={(e) => e.preventDefault()}
          onclick={() => void runCmd(id)}><Icon name={icons[id] ?? ""} /></button
        >
      {/if}
    {/each}
    <span class="spacer"></span>
    <span class="zoom">{Math.round(prefs.data.zoom * 100)}%</span>
  </div>

  {#if tab}
    <div class="cellbar" class:expanded={cellBarExpanded}>
      <input
        bind:this={gotoInput}
        class="goto"
        aria-label="Go to row and column"
        title="Go to: type a row, or row and column (Ctrl+G)"
        bind:value={gotoText}
        onkeydown={(e) => {
          e.stopPropagation();
          if (e.key === "Enter") goTo(gotoText);
          if (e.key === "Escape") grid?.focus();
        }}
      />
      <textarea
        bind:this={cellBarInput}
        class="cellvalue"
        aria-label="Cell contents"
        spellcheck={prefs.data.spellcheck}
        placeholder={cellBarTruncated ? "" : ""}
        readonly={cellBarTruncated || !tab.info.indexed}
        bind:value={cellBarText}
        oninput={() => (cellBarDirty = true)}
        onkeydown={cellBarKey}
        onblur={() => {
          if (!cellBarDirty) return;
        }}
      ></textarea>
      {#if cellBarDirty}
        <button type="button" class="small primary" onclick={() => void applyCellBar()} title="Apply (Enter{cellBarExpanded ? ', or Ctrl+Enter' : ''})"
          >{cellCount(tab.rect) > 1 ? `Apply to ${cellCount(tab.rect).toLocaleString()} cells` : "Apply"}</button
        >
        <button type="button" class="small" onclick={() => { cellBarDirty = false; cellBarFor = ""; }}>Cancel</button>
      {/if}
      <button type="button" class="small expand" title="Expand cell bar (Ctrl+Shift+U)" aria-label="Expand cell bar" onclick={() => (cellBarExpanded = !cellBarExpanded)}
        >{cellBarExpanded ? "▴" : "▾"}</button
      >
    </div>
    {#if cellFind && cellBarExpanded}
      <div class="cellfind" role="search" aria-label="Find in this cell">
        <input
          aria-label="Find in this cell"
          placeholder="Find in this cell"
          bind:value={cellFind.text}
          onkeydown={(e) => {
            e.stopPropagation();
            if (e.key === "Enter") { e.preventDefault(); void cellFindRun(e.shiftKey ? -1 : 1); }
            if (e.key === "Escape") { cellFind = null; cellBarInput?.focus(); }
          }}
        />
        <button type="button" class="small" aria-label="Previous match" onclick={() => void cellFindRun(-1)}>↑</button>
        <button type="button" class="small" aria-label="Next match" onclick={() => void cellFindRun(1)}>↓</button>
        <span class="muted">{cellFind.at >= 0 ? `${cellFind.at + 1} of ${cellFind.hits.length}` : cellFind.hits.length === 0 && cellFind.text ? "" : ""}</span>
        <span class="muted">({findOptions.kind === "regex" ? "regular expression" : findOptions.kind === "wildcard" ? "wildcards" : "plain text"}{findOptions.matchCase ? ", match case" : ""})</span>
        <button type="button" class="small" aria-label="Close" onclick={() => { cellFind = null; cellBarInput?.focus(); }}>✕</button>
      </div>
    {/if}
    {#if findOpen}
      <FindPanel
        bind:this={findPanel}
        {tab}
        bind:replace={findReplace}
        bind:options={findOptions}
        run={runJob}
        onSelect={(row, col) => { grid?.select({ row, col }); }}
        onChanged={(d) => tab?.update(d, true)}
        onClose={() => { findOpen = false; grid?.focus(); }}
      />
    {/if}
  {/if}

  <div class="tabs" role="tablist" aria-label="Open files">
    {#each tabs as t, i (t.id)}
      <div class="tab" class:active={i === active} class:dragging={tabDrag?.moved && tabs[tabDrag.from] === t} role="tab" aria-selected={i === active} tabindex="-1" title={t.info.path ?? t.info.name}
        data-tab-index={i}
        onmousedown={(e) => { if (e.button === 1) { e.preventDefault(); void closeTab(i); } }}
        onpointerdown={(e) => tabPointerDown(e, i)}
        onpointermove={tabPointerMove}
        onpointerup={tabPointerUp}
        onpointercancel={tabPointerUp}
        onclick={() => { active = i; void tick().then(() => grid?.focus()); }}
        onkeydown={() => {}}
      >
        <span>{t.info.modified ? "● " : ""}{t.info.name}</span>
        {#if tabFolder[t.id]}<span class="tab-folder">{tabFolder[t.id]}</span>{/if}
        {#if !t.info.indexed}<span class="progress">{Math.round(t.info.progress * 100)}%</span>{/if}
        <button type="button" class="x" aria-label="Close {t.info.name}" onclick={(e) => { e.stopPropagation(); void closeTab(i); }}><Icon name="close" size={12} /></button>
      </div>
    {/each}
  </div>

  {#if notices.length}
    <div class="notices" role="status">
      {#each notices as n, i (i)}
        <div class="notice">
          <span>{n.text}</span>
          {#if n.action && n.run}<button type="button" class="small" onclick={n.run}>{n.action}</button>{/if}
        </div>
      {/each}
      <button type="button" class="small dismiss" aria-label="Dismiss notices" onclick={() => { if (tab) tab.dismissed = true; }}><Icon name="close" size={12} /></button>
    </div>
  {/if}

  <main class="body" class:with-panel={!!prefs.data.panel}>
    {#if tab}
      {#key tab.id}
        <Grid
          bind:this={grid}
          {tab}
          readOnly={!tab.info.indexed}
          onCommit={commitCell}
          onContext={openContext}
          onMessage={say}
          onFilterButton={(col, x, y) => (filterMenu = { col, x, y })}
          onFill={(dir, count, swap) => void fillHandle(dir, count, swap)}
          onFillAuto={() => void fillHandleAuto()}
          onOpenLink={(url) => void openLink(url)}
          onMoveCols={(at, count, to) => void dragCols(at, count, to)}
          onMoveRows={(at, count, to) => void dragRows(at, count, to)}
          onMoveCells={(from, row, col, copy) => void dragCells(from, row, col, copy)}
        />
      {/key}
    {:else}
      <div class="empty">
        <div class="card">
          <h1>No file open</h1>
          <p>Open a CSV, TSV or any delimited text file. Files of any size open; there is no row limit.</p>
          <div class="actions">
            <button type="button" class="primary" onclick={openFile}>Open… (Ctrl+O)</button>
            <button type="button" onclick={newFile}>New (Ctrl+N)</button>
            <button type="button" onclick={importFile}>Import…</button>
          </div>
          {#if recent.length}
            <h2>Recent files</h2>
            <ul class="recent">
              {#each recent as r (r.id)}
                <!-- Middle-click: mousedown is stopped so Windows does not start its scroll mode. -->
                <li><button type="button" title={r.folder ? `${r.folder}\\${r.name}` : r.name} onclick={() => void openRecent(r)} onmousedown={(e) => { if (e.button === 1) e.preventDefault(); }} onauxclick={(e) => void startMiddle(e, r)}><span class="name">{r.name}</span> <span class="muted">{r.folder}</span></button></li>
              {/each}
            </ul>
          {/if}
          <p class="hint">Alt+Q searches every command.</p>
        </div>
      </div>
    {/if}
    {#if prefs.data.panel}
      <SidePanel
        {tab}
        bind:which={panelTab}
        titles={tab ? shownTitles(tab) : []}
        {activity}
        read={READ}
        onOpened={opened}
        onFilterValue={(col, value, cut) => void setRule(col, cut ? { type: "conditions", first: { type: "text", op: "begins_with", text: value.replace(/…$/, ""), match_case: true } } : value === "" ? { type: "values", values: [], blanks: true } : { type: "values", values: [value], blanks: false })}
        onClose={() => showPanel(null)}
      />
    {/if}
  </main>

  <footer class="status">
    {#if message}
      <span class="msg">{message}</span>
    {:else if tab?.busy}
      <span>{tab.busy}{tab.progress !== null ? ` ${Math.round(tab.progress * 100)}%` : ""}</span>
      {#if tab.busy !== "Saving…"}<button type="button" class="small" onclick={cancelJob}>Cancel</button>{/if}
    {:else if tab && !tab.info.indexed}
      <span>Reading… {Math.round(tab.info.progress * 100)}% ({tab.info.rows.toLocaleString()} rows so far)</span>
    {:else if tab?.cache.error}
      <span class="msg">{tab.cache.error}</span>
    {:else}
      <span>Ready</span>
    {/if}
    <span class="spacer"></span>
    {#if status}
      {#if tab}<QuickTotals {tab} />{/if}
      {#if status.sel}<span>{status.sel}</span>{/if}
      {#if status.hidden}
        <button type="button" class="small filtered" title="Hidden rows are still saved. Click to show them all again." onclick={() => void showRows(true)}>{plural(status.hidden, "hidden row")} ✕</button>
      {/if}
      {#if status.filter}
        <button type="button" class="small filtered" title={`Filter: ${status.filter}. Click to show all rows.`} onclick={() => void clearFilter()}>Filtered ✕</button>
      {/if}
      {#if tab?.info.read_only}<button type="button" class="small" title="Locked: cells cannot change. Click to unlock." onclick={() => void toggleReadOnly()}>Locked</button>{/if}
      <span>{status.size}</span>
      <span class="muted">{status.format}</span>
      {#if tab?.info.modified && autosaveNote && prefs.data.autosave}<span class="muted" title="Unsaved changes are copied to a private folder every few minutes, so they can be recovered if the app stops. Set this in Options.">{autosaveNote}</span>{/if}
      {#if tab?.info.column_changes}<span class="muted" title="Columns were inserted, deleted or moved, so saving rewrites every row">columns changed</span>{/if}
    {:else if info}
      <span class="muted">v{info.version} · engine {info.engine_version}</span>
    {/if}
  </footer>
</div>

{#if context}
  <div class="menu context" role="menu" style:left="{context.x}px" style:top="{context.y}px">
    {#each context.items as id, j (j)}
      {#if id === "-"}
        <div class="sep"></div>
      {:else}
        {@const c = reg.get(id)}
        {@const pos = context.items.filter((x) => x !== "-").indexOf(id)}
        {#if c}
          <button type="button" role="menuitem" class="item" class:hot={context.index === pos} disabled={!reg.enabled(id)} onclick={() => void runCmd(id)}>
            <span class="check">{c.checked?.() ? "✓" : ""}</span>
            <span class="label">{plain(c.label)}</span>
            <span class="keys">{c.keys?.[0] ?? ""}</span>
          </button>
        {/if}
      {/if}
    {/each}
  </div>
{/if}

{#if layoutFor}
  <LayoutDialog
    doc={layoutFor.doc}
    modified={layoutFor.modified}
    onChoose={(l) => void reopenWithLayout(l)}
    onClose={() => { layoutFor = null; void tick().then(() => grid?.focus()); }}
  />
{/if}

{#if filterMenu && tab}
  {@const fm = filterMenu}
  <FilterMenu
    {tab}
    col={fm.col}
    title={columnTitle(tab, fm.col)}
    x={fm.x}
    y={fm.y}
    onApply={(rule) => { const col = fm.col; filterMenu = null; void setRule(col, rule); grid?.focus(); }}
    onSort={(desc) => { const col = fm.col; filterMenu = null; sortByColumn(desc, col); grid?.focus(); }}
    onClose={() => { filterMenu = null; grid?.focus(); }}
  />
{/if}

{#if sortDialog && tab}
  <SortDialog
    titles={Array.from({ length: Math.min(tab.info.cols, 5000) }, (_, c) => columnTitle(tab!, c))}
    header={tab.header}
    start={sortDialog}
    onSort={(spec, header) => {
      sortDialog = null;
      if (tab && header !== tab.header) { tab.header = header; tab.headerTouched = true; }
      void sortRows(spec);
      grid?.focus();
    }}
    onClose={() => { sortDialog = null; grid?.focus(); }}
  />
{/if}

{#if opDialog && tab}
  {@const od = opDialog}
  <OpDialog
    kind={od.kind}
    titles={Array.from({ length: Math.min(tab.info.cols, 5000) }, (_, c) => columnTitle(tab!, c))}
    cols={tab.info.cols}
    header={tab.header}
    sel={tab.rect}
    clip={od.clip}
    onRun={(r, header) => { opDialog = null; void runDialog(r, header); grid?.focus(); }}
    onClose={() => { opDialog = null; grid?.focus(); }}
  />
{/if}

{#if formRow !== null && tab}
  <FormView
    doc={tab.id}
    titles={Array.from({ length: Math.min(tab.info.cols, 500) }, (_, c) => columnTitle(tab!, c))}
    cols={tab.info.cols}
    firstRow={tab.firstRow}
    rows={tab.info.rows}
    start={formRow}
    onChanged={(d) => tab?.update(d, true)}
    onClose={(row) => { formRow = null; grid?.select({ row, col: tab?.sel.focus.col ?? 0 }); grid?.focus(); }}
  />
{/if}

{#if symbols}
  <SymbolPicker
    onPick={(ch) => { const at = symbols?.caret ?? null; symbols = null; void grid?.insertText(ch, at); }}
    onClose={() => { symbols = null; grid?.focus(); }}
  />
{/if}

{#if saveAsReq && tab}
  {@const req = saveAsReq}
  <SaveAsDialog info={tab.info} sample={req.sample} onSave={(o) => req.done(o)} onClose={() => { req.done(null); grid?.focus(); }} />
{/if}

{#if exportOpen && tab}
  <ExportDialog
    doc={tab.id}
    name={tab.info.name}
    header={tab.header}
    filtered={tab.info.filtered}
    titles={Array.from({ length: Math.min(tab.info.cols, 200) }, (_, c) => columnTitle(tab!, c))}
    onExport={(o, to) => void exportRun(o, to)}
    onClose={() => { exportOpen = false; grid?.focus(); }}
  />
{/if}

{#if importing}
  {@const start = importing}
  <ImportDialog {start} onImport={(o, header) => void finishImport(start, o, header)} onClose={closeImport} />
{/if}

{#if joinKind && tab}
  {@const kind = joinKind}
  <JoinDialog
    {kind}
    doc={tab.id}
    titles={Array.from({ length: Math.min(tab.info.cols, 2000) }, (_, c) => columnTitle(tab!, c))}
    header={tab.header}
    rows={tab.info.rows}
    others={otherTables()}
    {titlesOf}
    onOpen={openOther}
    onRun={(op, header) => { joinKind = null; void runDialog({ data: op }, header); grid?.focus(); }}
    onClose={() => { joinKind = null; grid?.focus(); }}
  />
{/if}

{#if cleanTool && tab}
  {@const r = tab.sel}
  <CleanDialog
    doc={tab.id}
    titles={Array.from({ length: Math.min(tab.info.cols, 2000) }, (_, c) => columnTitle(tab!, c))}
    header={tab.header}
    sel={{ row: Math.min(r.anchor.row, r.focus.row), rows: Math.abs(r.anchor.row - r.focus.row) + 1, col: Math.min(r.anchor.col, r.focus.col), cols: Math.abs(r.anchor.col - r.focus.col) + 1 }}
    bind:cleaner
    tool={cleanTool}
    onRun={(spec, header, what) => { cleanTool = null; void runClean(spec, header, what); grid?.focus(); }}
    onSay={say}
    onClose={() => { cleanTool = null; grid?.focus(); }}
  />
{/if}

{#if search !== null}
  <CommandSearch
    commands={[...reg.all(), ...viewCommands]}
    initial={search}
    docs={searchDocs}
    menuOf={(id) => menuOfId.get(id) ?? (id.startsWith("view.") ? "View" : undefined)}
    hotkey={reg.get("help.search")?.keys?.[0] ?? "Alt+Q"}
    onUnavailable={(name) => say(ready() ? `“${name}” isn't available here right now.` : `“${name}” needs a file: open one first.`)}
    keysOf={(id) => reg.get(id)?.keys ?? []}
    enabled={(id) => (id.startsWith("view.saved.") ? ready() : id.startsWith("view.forget.") ? true : reg.enabled(id))}
    onRun={runSearched}
    onClose={() => { search = null; grid?.focus(); }}
  />
{/if}

{#if recovery}
  <RecoveryDialog
    mode={recovery.mode}
    name={recovery.name}
    items={recovery.items}
    onOpen={(i, label) => void openRecovered(i, label)}
    onDiscard={recovery.mode === "recover" ? (i) => void discardRecovered(i) : undefined}
    onClose={() => { recovery = null; grid?.focus(); }}
  />
{/if}

{#if suggest && tab}
  {@const t = tab}
  <SuggestDialog
    found={suggest.found}
    busy={suggest.busy}
    title={(c) => columnTitle(t, c)}
    onFix={(s) => void fixSuggestions([s])}
    onFixAll={() => void fixSuggestions(suggest?.found?.items ?? [])}
    onClose={() => { suggest = null; grid?.focus(); }}
  />
{/if}

{#if recentOpen}
  <RecentDialog items={recent} openDocs={openDocs()} onOpen={(list) => void openRecentMany(list)} onOpenKeep={(r) => void openRecentMany([r], true)} onClose={() => { recentOpen = false; grid?.focus(); }} />
{/if}

{#if keysOpen}
  <KeysDialog
    groups={keyGroups(menus, (id) => { const c = reg.get(id); return c ? { label: c.label, keys: c.keys ?? [] } : null; })}
    basics={BASIC_KEYS}
    mouse={MOUSE_MOVES}
    showMouse={keysMouse}
    onCustomize={() => { keysOpen = false; shortcutsOpen = true; }}
    onClose={() => { keysOpen = false; keysMouse = false; grid?.focus(); }}
  />
{/if}

{#if privacyOpen && tab}
  <PrivacyDialog
    titles={shownTitles(tab)}
    cols={tab.info.cols}
    header={tab.header}
    findings={privacyFindings}
    problem={privacyProblem}
    onRun={(spec) => void saveSafeCopy(spec)}
    onClose={() => { privacyOpen = false; grid?.focus(); }}
  />
{/if}

{#if folderSearchOpen}
  <FolderSearchDialog doc={tab?.id ?? null} onOpen={(h) => void openFolderHit(h)} onClose={() => { folderSearchOpen = false; grid?.focus(); }} />
{/if}

{#if recipeOpen && tab}
  <RecipeDialog
    current={recipeCurrent}
    opened={recipeFile}
    onSave={(n) => void saveRecipe(n)}
    onOpen={() => void openRecipeFile()}
    onRun={(f, p) => void runRecipe(f, p)}
    onClose={() => { recipeOpen = false; recipeFile = null; grid?.focus(); }}
  />
{/if}

{#if changesOpen && tab}
  <ChangesDialog
    list={changes}
    titles={shownTitles(tab)}
    showEdits={tab.showEdits}
    onGo={goToChange}
    onSave={() => void saveChangeReport()}
    onShowEdits={setShowEdits}
    onStop={cancelJob}
    onClose={() => { if (!changes) cancelJob(); changesOpen = false; grid?.focus(); }}
  />
{/if}

{#if mailSheets && tab}
  <MailDialog
    titles={shownTitles(tab)}
    header={tab.header}
    rows={Math.max(0, tab.info.rows - (tab.header ? 1 : 0))}
    sheets={mailSheets}
    preview={(spec) => backend().mailPreview(tab!.id, spec)}
    problem={mailProblem}
    onSave={(spec) => void saveMail(spec)}
    onClose={() => { mailSheets = null; grid?.focus(); }}
  />
{/if}

{#if datePick && tab}
  {@const at = datePick}
  <DatePicker
    current={at.text}
    onPick={(text) => { datePick = null; void commitCell(at.row, at.col, text, false); grid?.focus(); }}
    onClose={() => { datePick = null; grid?.focus(); }}
  />
{/if}

{#if formatFor && tab}
  {@const f = formatFor}
  <FormatDialog
    what={f.what}
    current={f.current}
    samples={f.samples}
    onDone={(fm) => { const cols = f.cols; formatFor = null; setFormat(cols, fm); grid?.focus(); }}
    onClose={() => { formatFor = null; grid?.focus(); }}
  />
{/if}

{#if shortcutsOpen}
  <ShortcutsDialog commands={reg.all()} custom={prefs.data.shortcuts} onSave={saveShortcuts} onClose={() => { shortcutsOpen = false; grid?.focus(); }} />
{/if}

{#if highlightOpen && tab}
  <HighlightDialog
    titles={shownTitles(tab)}
    col={tab.sel.focus.col}
    rules={tab.highlights}
    onSave={(rules) => { highlightOpen = false; setHighlights(rules); grid?.focus(); }}
    onClose={() => { highlightOpen = false; grid?.focus(); }}
  />
{/if}

{#if valuesOpen && tab}
  <ValueEditor
    doc={tab.id}
    titles={shownTitles(tab)}
    col={tab.sel.focus.col}
    header={tab.header}
    others={otherTables()}
    {titlesOf}
    onRun={(op, header) => { valuesOpen = false; void runDialog({ data: op }, header); grid?.focus(); }}
    onClose={() => { valuesOpen = false; grid?.focus(); }}
  />
{/if}

{#if editKind && tab}
  {@const kind = editKind}
  <EditTools
    {kind}
    doc={tab.id}
    titles={shownTitles(tab)}
    cols={tab.info.cols}
    col={tab.sel.focus.col}
    header={tab.header}
    onRun={(op, header) => { editKind = null; void runDialog({ data: op }, header); grid?.focus(); }}
    onClose={() => { editKind = null; grid?.focus(); }}
  />
{/if}

{#if computeOpen && tab}
  <ComputeDialog
    doc={tab.id}
    titles={shownTitles(tab)}
    col={tab.sel.focus.col}
    header={tab.header}
    onRun={(s) => void runCompute(s)}
    onClose={() => { computeOpen = false; grid?.focus(); }}
  />
{/if}

{#if watchOpen}
  <WatchDialog onClose={() => { watchOpen = false; grid?.focus(); }} />
{/if}

{#if notesRow !== null && tab}
  <NotesDialog
    doc={tab.id}
    row={notesRow}
    titles={Array.from({ length: Math.min(tab.info.cols, 2000) }, (_, c) => columnTitle(tab!, c))}
    header={tab.header}
    current={tab.rowNotes.get(notesRow) ?? null}
    onChanged={() => { const t = tab; if (t) { t.notesVersion++; t.bump(); } }}
    onGo={(row) => { notesRow = null; grid?.select({ row, col: tab?.sel.focus.col ?? 0 }); grid?.focus(); }}
    onShow={(flag) => {
      const t = tab;
      notesRow = null;
      if (!t) return;
      void runJob("Filtering", () => backend().notesShow(t.id, flag))
        .then((info) => {
          if (!info) return;
          t.update(info, true);
          t.filter = null;
          say(`Showing the ${plural(Math.max(0, info.rows - (t.header ? 1 : 0)), "row")} with ${flag ? "that flag" : "a note or flag"}. Click “Filtered ✕” to show every row again.`);
        })
        .catch((e) => say(errorText(e)))
        .finally(() => grid?.focus());
    }}
    onClose={() => { notesRow = null; grid?.focus(); }}
  />
{/if}

{#if sqliteMode}
  <SqliteDialog
    mode={sqliteMode}
    doc={tab?.id ?? null}
    docName={tab?.info.name ?? ""}
    header={tab?.header ?? true}
    filtered={!!tab && tab.info.rows < tab.info.total_rows}
    onSaved={(n) => { sqliteMode = null; say(`Saved ${plural(n, "row")} to the database.`); grid?.focus(); }}
    onOpened={(info) => { sqliteMode = null; addTab(info, true); say("Opened the table in a new tab. Save it as a file with File ▸ Save as, or back to a database with File ▸ Save as a table in a database."); }}
    onClose={() => { sqliteMode = null; grid?.focus(); }}
  />
{/if}

{#if encryptMode}
  <EncryptDialog
    mode={encryptMode}
    same={encryptSame}
    doc={tab?.id ?? null}
    filtered={!!tab && tab.info.rows < tab.info.total_rows}
    onSaved={(n) => { encryptMode = null; encryptSame = null; say(`Saved an encrypted copy (${plural(n, "row")}). Keep the password safe: without it the file cannot be opened.`); grid?.focus(); }}
    onOpened={(info) => { encryptMode = null; encryptSame = null; addTab(info, false); say("Opened the encrypted file. This tab is private: no recovery copies, not in the recent files."); }}
    onClose={() => { encryptMode = null; encryptSame = null; grid?.focus(); }}
  />
{/if}

{#if updatesOpen && tab}
  <UpdatesDialog
    doc={tab.id}
    titles={Array.from({ length: Math.min(tab.info.cols, 2000) }, (_, c) => columnTitle(tab!, c))}
    header={tab.header}
    col={tab.sel.focus.col}
    others={otherTables()}
    onOpen={openOther}
    onDone={(info, summary) => {
      const t = tab;
      updatesOpen = false;
      if (t) t.update(info, true);
      say(`${summary} Undo (Ctrl+Z) takes it all back.`);
      grid?.focus();
    }}
    onClose={() => { const t = tab; updatesOpen = false; if (t) void backend().updatesClose(t.id); grid?.focus(); }}
  />
{/if}

{#if streetOpen && tab}
  <StreetDialog
    doc={tab.id}
    titles={Array.from({ length: Math.min(tab.info.cols, 2000) }, (_, c) => columnTitle(tab!, c))}
    header={tab.header}
    col={tab.sel.focus.col}
    others={otherTables()}
    {titlesOf}
    onOpen={openOther}
    onRun={(other, cols, spec) => {
      const t = tab;
      streetOpen = false;
      if (!t) return;
      void runJob("Looking up streets", () => backend().streetLookup(t.id, other, cols, spec))
        .then((r) => {
          if (!r) return;
          t.update(r.info, true);
          say(`${streetSummary(r.report)} Undo (Ctrl+Z) takes the new columns back.`);
        })
        .catch((e) => say(errorText(e)))
        .finally(() => grid?.focus());
    }}
    onClose={() => { streetOpen = false; grid?.focus(); }}
  />
{/if}

{#if matchOpen && tab}
  <MatchDialog
    doc={tab.id}
    titles={Array.from({ length: Math.min(tab.info.cols, 2000) }, (_, c) => columnTitle(tab!, c))}
    header={tab.header}
    others={otherTables()}
    {titlesOf}
    onOpen={openOther}
    onDone={(info, n) => {
      const t = tab;
      matchOpen = false;
      if (t) {
        t.update(info, true);
        void backend().matchClose(t.id);
      }
      say(`Added match columns for ${plural(n, "accepted pair")}. Undo (Ctrl+Z) takes them back.`);
      grid?.focus();
    }}
    onClose={() => { const t = tab; matchOpen = false; if (t) void backend().matchClose(t.id); grid?.focus(); }}
  />
{/if}

{#if fieldCheckOpen && tab}
  <FieldCheckDialog
    doc={tab.id}
    titles={shownTitles(tab)}
    header={tab.header}
    checks={tab.checks}
    onApply={(c) => keepChecks(c, true)}
    onShow={(s, passing) => void showProblems(s, passing)}
    onSortFailing={(s) => void sortFailing(s)}
    onResultColumn={(s) => void addCheckColumn(s)}
    onClose={() => { fieldCheckOpen = false; grid?.focus(); }}
  />
{/if}

{#if checksOpen && tab}
  <ChecksDialog
    doc={tab.id}
    titles={shownTitles(tab)}
    col={tab.sel.focus.col}
    header={tab.header}
    checks={tab.checks}
    marked={tab.checksMarked}
    onApply={keepChecks}
    onGoto={(row, col) => { const t = tab; if (t) grid?.select({ row, col: col ?? t.sel.focus.col }); }}
    onShowProblems={(s) => void showProblems(s)}
    onClose={() => { checksOpen = false; grid?.focus(); }}
  />
{/if}

{#if printReq && tab}
  <PrintDialog
    setup={pageSetup ?? defaultPage(tab.info.name)}
    titles={printReq.titles}
    sample={printReq.sample}
    allRows={printReq.allRows}
    selRows={printReq.selRows}
    cols={printReq.cols.length}
    fontFamily={getComputedStyle(document.body).fontFamily || "sans-serif"}
    pdfFirst={printReq.pdf}
    onPrint={(s, sel) => void doPrint(s, sel)}
    onPdf={(s, sel) => void doPdf(s, sel)}
    onClose={() => { printReq = null; grid?.focus(); }}
  />
{/if}

{#if optionsOpen}
  <OptionsDialog
    onAgent={(on) => { prefs.data.agentLink = on; prefs.save(); }}
    onClose={() => { optionsOpen = false; grid?.focus(); }}
  />
{/if}

{#if toolsKind && tab}
  {@const t = tab}
  <ToolsDialog
    kind={toolsKind}
    titles={shownTitles(t)}
    cols={selectedCols(t)}
    col={t.sel.focus.col}
    header={t.header}
    others={tabs.filter((x) => x !== t).map((x) => ({ id: x.id, name: x.info.name }))}
    onRun={(r, header) => { toolsKind = null; void runTools(r, header); }}
    onClose={() => { toolsKind = null; grid?.focus(); }}
  />
{/if}

{#if pivotDoc !== null && pivotTab()}
  {@const pt = pivotTab()!}
  <PivotDialog
    doc={pt.id}
    titles={shownTitles(pt)}
    header={pt.header}
    rows={Math.max(0, pt.info.rows - (pt.header ? 1 : 0))}
    initial={pivotSpecs.get(pt.id) ?? null}
    sample={(c) => pivotSample(pt, c)}
    onTable={(s) => void pivotToTab(s)}
    onCopy={(s) => void pivotCopy(s)}
    onDrill={(s, r, c) => void pivotDrill(s, r, c)}
    onSay={say}
    onConfirmSave={(n) => pivotSaveAsk(pt, n)}
    onClose={pivotDone}
  />
{/if}

{#if subtotalOpen && tab}
  <SubtotalDialog
    titles={shownTitles(tab)}
    col={tab.sel.focus.col}
    header={tab.header}
    onRun={(op, header) => { subtotalOpen = false; void runDialog({ table: op }, header); }}
    onClose={() => { subtotalOpen = false; grid?.focus(); }}
  />
{/if}

{#if prompt}
  {@const p = prompt}
  <PromptDialog
    title={p.title}
    label={p.label}
    value={p.value}
    note={p.note}
    check={p.check}
    onDone={(v) => { const done = p.done; prompt = null; done(v); void tick().then(() => grid?.focus()); }}
    onClose={() => { prompt = null; grid?.focus(); }}
  />
{/if}

{#if splitNameOpen && tab}
  <SplitNameDialog
    col={tab.sel.focus.col}
    titles={Array.from({ length: Math.min(tab.info.cols, 2000) }, (_, c) => columnTitle(tab!, c))}
    header={tab.header}
    onRun={(op, header) => { splitNameOpen = false; void runSplitName(op, header); grid?.focus(); }}
    onClose={() => { splitNameOpen = false; grid?.focus(); }}
  />
{/if}

{#if insertAsk && tab}
  {@const kind = insertAsk}
  <InsertDialog
    {kind}
    count={kind === "rows" ? tab.rect.rows : tab.rect.cols}
    onRun={(n, after) => { const rows = kind === "rows"; insertAsk = null; void (rows ? insertRows(after, n) : insertCols(after, n)).then(() => grid?.focus()); }}
    onClose={() => { insertAsk = null; void tick().then(() => grid?.focus()); }}
  />
{/if}

{#if dialog}
  <Dialog request={dialog} onDone={() => { dialog = null; void tick().then(() => grid?.focus()); }} />
{/if}

<style>
  .shell {
    display: flex;
    flex-direction: column;
    height: 100%;
    font-family: var(--ui-font);
  }
  .menubar {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 2px 8px;
    background: var(--surface);
    position: relative;
    z-index: 20;
  }
  .logo {
    margin-right: 6px;
  }
  .menu-title {
    font: inherit;
    background: none;
    border: none;
    color: var(--text);
    padding: 4px 10px;
    border-radius: 5px;
    cursor: default;
  }
  .menu-title:hover,
  .menu-title.open {
    background: var(--bg);
  }
  .menu-title u {
    text-decoration: none;
  }
  .menubar:focus-within .menu-title u,
  .menu-title.open u {
    text-decoration: underline;
  }
  .mwrap {
    position: relative;
  }
  .menu {
    position: absolute;
    top: 100%;
    left: 0;
    min-width: 250px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 10px 30px rgb(0 0 0 / 0.18);
    padding: 4px;
    z-index: 30;
    max-height: calc(100vh - 60px);
    overflow-y: auto;
  }
  .context {
    position: fixed;
    top: 0;
  }
  .item {
    display: grid;
    grid-template-columns: 18px 1fr auto;
    gap: 8px;
    width: 100%;
    font: inherit;
    text-align: left;
    background: none;
    border: none;
    color: var(--text);
    padding: 5px 10px 5px 6px;
    border-radius: 5px;
    cursor: default;
  }
  .item.hot:not(:disabled) {
    background: var(--accent);
    color: var(--accent-ink);
  }
  .item:disabled {
    opacity: 0.45;
  }
  .item u {
    text-decoration: none;
  }
  .item u.show {
    text-decoration: underline;
  }
  .keys {
    color: inherit;
    opacity: 0.7;
    font-size: 12px;
  }
  .sep {
    height: 1px;
    background: var(--border);
    margin: 4px 6px;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 3px 8px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
  }
  .tool {
    display: grid;
    place-items: center;
    width: 30px;
    height: 28px;
    border: 1px solid transparent;
    border-radius: 6px;
    background: none;
    color: var(--text);
    cursor: default;
    flex: none;
  }
  .tool:hover:not(:disabled) {
    background: var(--bg);
    border-color: var(--border);
  }
  .tool.on {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
  }
  .tool:disabled {
    opacity: 0.35;
  }
  .tsep {
    width: 1px;
    height: 20px;
    background: var(--border);
    margin: 0 4px;
    flex: none;
  }
  .spacer {
    flex: 1;
  }
  .zoom {
    font-size: 12px;
    color: var(--muted);
  }
  .cellbar {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    padding: 4px 8px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }
  .goto {
    width: 120px;
    font: inherit;
    font-size: 13px;
    padding: 3px 6px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--bg);
    color: var(--text);
  }
  .cellvalue {
    flex: 1;
    font: inherit;
    font-size: 13px;
    height: 26px;
    padding: 3px 6px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--surface);
    color: var(--text);
    resize: none;
    white-space: pre;
    overflow: hidden;
  }
  .expanded .cellvalue {
    height: 180px;
    white-space: pre-wrap;
    overflow: auto;
    tab-size: 4;
  }
  .goto:focus,
  .cellvalue:focus {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
  }
  .small {
    font: inherit;
    font-size: 12px;
    padding: 3px 9px;
    border-radius: 5px;
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--text);
    cursor: pointer;
    white-space: nowrap;
  }
  .small.primary,
  .primary {
    background: var(--accent);
    color: var(--accent-ink);
    border-color: var(--accent);
  }
  .expand {
    width: 28px;
    padding: 3px 0;
  }
  .tabs {
    display: flex;
    gap: 2px;
    padding: 4px 8px 0;
    background: var(--bg);
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 6px 5px 12px;
    border: 1px solid var(--border);
    border-bottom: none;
    border-radius: 7px 7px 0 0;
    background: var(--bg);
    color: var(--muted);
    cursor: default;
    white-space: nowrap;
    font-size: 13px;
  }
  .tab.active {
    background: var(--surface);
    color: var(--text);
    box-shadow: inset 0 2px 0 var(--accent);
  }
  .tab.dragging {
    opacity: 0.7;
    cursor: grabbing;
  }
  .tab-folder {
    color: var(--muted);
    font-size: 0.85em;
  }
  .tabs:empty {
    display: none;
  }
  .progress {
    font-size: 11px;
    color: var(--accent);
  }
  .x {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border: none;
    background: none;
    color: inherit;
    border-radius: 4px;
    cursor: pointer;
  }
  .x:hover {
    background: var(--border);
  }
  .notices {
    position: relative;
    padding: 6px 40px 6px 12px;
    background: color-mix(in srgb, #e3a008 16%, var(--surface));
    border-bottom: 1px solid var(--border);
    font-size: 13px;
    display: grid;
    gap: 4px;
  }
  .notice {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .dismiss {
    position: absolute;
    right: 8px;
    top: 6px;
    padding: 3px 5px;
  }
  .body {
    flex: 1;
    display: grid;
    min-height: 0;
  }
  .body.with-panel {
    grid-template-columns: minmax(0, 1fr) 320px;
  }
  .card h2 {
    font-size: 14px;
    margin: 20px 0 6px;
    text-align: left;
  }
  .recent {
    list-style: none;
    margin: 0;
    padding: 0;
    text-align: left;
    max-height: 40vh;
    overflow: auto;
  }
  .recent button {
    width: 100%;
    text-align: left;
    font: inherit;
    padding: 4px 6px;
    border: 0;
    border-radius: 4px;
    background: none;
    color: var(--text);
    cursor: pointer;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .recent button:hover {
    background: var(--bg);
  }
  .recent .muted,
  .hint {
    color: var(--muted);
    font-size: 12px;
  }
  .empty {
    display: grid;
    place-items: center;
    padding: 16px;
  }
  .card {
    max-width: 440px;
    text-align: center;
    padding: 32px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
  }
  .card h1 {
    font-size: 20px;
    margin: 0 0 8px;
  }
  .card p {
    margin: 4px 0 16px;
    color: var(--muted);
  }
  .actions {
    display: flex;
    gap: 8px;
    justify-content: center;
  }
  .actions button {
    font: inherit;
    padding: 6px 14px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--text);
    cursor: pointer;
  }
  .actions button.primary {
    background: var(--accent);
    color: var(--accent-ink);
    border-color: var(--accent);
  }
  .status {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 4px 12px;
    background: var(--surface);
    border-top: 1px solid var(--border);
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
  }
  .muted {
    color: var(--muted);
  }
  .msg {
    color: var(--accent);
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
