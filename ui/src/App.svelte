<script lang="ts">
  import { actingWords, hiddenRows, hiddenWords, kindOfOp } from "./lib/shown.svelte";
  import { forgetHeaders, recallColumnTypes, recallHeader, rememberColumnTypes, rememberedHeaders, rememberHeader, type TypeMemory } from "./lib/headerMemory";
import { readDateFor, readNumberFor, textCols, typeOf as typeAt, typeName, type ColumnType } from "./lib/readers";
  import { sessionToOffer } from "./lib/sessionOffer";
  import { namesFromText } from "./lib/columnNames";
  import { onMount, tick, untrack } from "svelte";
  import { fade } from "svelte/transition";
  import { backend, writeGuard, errorText, isCmdError, type HandOver, type ModifiedReport, type AppInfo, type Change, type DocInfo, type Layout, type Checkpoint, type EncodingChoice, type DateScan } from "./lib/backend";
  import { Tab, exportHandTab, importHandTab, type HandTab, type RowMap } from "./lib/tab.svelte";
import { MAX_CUSTOM_ROWS, MAX_ROW_PX, MIN_ROW_PX, RowHeights } from "./lib/rowheights";
  import { SECOND_WINDOW } from "./lib/windowRole";
  import { invisibleNote, xrayNote } from "./lib/invisible";
  import { GUARD_LABEL, guardNote } from "./lib/csvguard";
  import { COLUMNS_CHANGED, raggedNotice, raggedResult, repairWhy, type RaggedPick } from "./lib/ragged";
  import RepairBar from "./lib/RepairBar.svelte";
  import { DATE_SAMPLE, datePill, dateSampled, dateSummary, dateWhy } from "./lib/dates";
  import DateReviewDialog, { type DateReviewDone } from "./lib/DateReviewDialog.svelte";
  import { prefs, customFont, enterStep, ACCENTS, FONTS, type Theme } from "./lib/prefs.svelte";
  import { Registry, altOf, keyOf, keyOfLoose, watchAlt, type Command, type Menu, SELECTION_FREE } from "./lib/commands";
  import { parseTsv, toTsv } from "./lib/tsv";
  import { colLetters, parseGoto } from "./lib/goto";
  import { cellCount, type Rect, cellCountAreas, colsOfAreas, disjoint, MAX_AREAS } from "./lib/selection";
  import { dropIndex, MOUSE_MOVES } from "./lib/dragmove";
  import { headerCertainty, looksLikeHeader, titleLines } from "./lib/header";
  import type { NotePlace } from "./lib/backend";
  import { separatorName } from "./lib/layout";
  import Grid from "./lib/Grid.svelte";
  import LayoutDialog from "./lib/LayoutDialog.svelte";
  import Icon from "./lib/Icon.svelte";
  import Dialog, { type DialogRequest, type DialogButton } from "./lib/Dialog.svelte";
  import FilterMenu from "./lib/FilterMenu.svelte";
  import SortDialog from "./lib/SortDialog.svelte";
  import FindPanel from "./lib/FindPanel.svelte";
  import { FIND_DEFAULTS, describeRule, ruleFor, withRule, type FindOptions } from "./lib/filters";
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
  import SinceDialog from "./lib/SinceDialog.svelte";
  import RecipeDialog from "./lib/RecipeDialog.svelte";
  import { answerPairs, keepAnswers, withPhrase, type RecipeOpened, type RecipeView } from "./lib/recipe";
  import BatchDialog from "./lib/BatchDialog.svelte";
  import FolderSearchDialog, { type FolderHit } from "./lib/FolderSearchDialog.svelte";
  import { copyTouches, findingsText, LARGE_COPY } from "./lib/privacy";
  import EditTools, { type ToolKind as EditKind } from "./lib/EditTools.svelte";
  import CleanDialog from "./lib/CleanDialog.svelte";
  import XrayDialog from "./lib/XrayDialog.svelte";
  import EncodingPicker from "./lib/EncodingPicker.svelte";
  import SplitNameDialog from "./lib/SplitNameDialog.svelte";
  import InsertDialog from "./lib/InsertDialog.svelte";
  import RenameColumn from "./lib/RenameColumn.svelte";
  import ShiftCellsDialog, { type ShiftChoice } from "./lib/ShiftCellsDialog.svelte";
  import GoToSpecialDialog, { type SpecialChoice } from "./lib/GoToSpecialDialog.svelte";
  import ZipPickDialog from "./lib/ZipPickDialog.svelte";
  import TidyTitlesDialog from "./lib/TidyTitlesDialog.svelte";
  import CompareColsDialog, { type CompareAction } from "./lib/CompareColsDialog.svelte";
  import GapsDialog from "./lib/GapsDialog.svelte";
  import RelationsDialog from "./lib/RelationsDialog.svelte";
  import type { Finding, RelationsReport, RelationsSpec } from "./lib/relations";
  import SetsDialog, { type SetsTab } from "./lib/SetsDialog.svelte";
  import PatchDialog from "./lib/PatchDialog.svelte";
  import { appliedText } from "./lib/patch";
  import type { SetsRun, SetsSpec } from "./lib/sets";
  import CleanMatrixDialog from "./lib/CleanMatrixDialog.svelte";
  import type { CleanMatrix, MatrixPreview } from "./lib/matrix";
  import type { GapReport, GapsSpec } from "./lib/gaps";
  import type { CompareCols } from "./lib/backend";
  import { typedCell } from "./lib/cellformula";
  import type { TitleSpec } from "./lib/backend";
  import type { ZipList } from "./lib/backend";
  import type { SpecialKind, SpecialSpec } from "./lib/backend";
  import { columnRef, localDays, placeCheck } from "./lib/m8";
  import { remapChecks, renameInChecks } from "./lib/rename";
  import { TOOLS, localNow, type Cleaner, type CleanSpec, type ToolId } from "./lib/clean";
  import { joinSummary, type OtherTable } from "./lib/transfer";
  import type { ExportOptions, ImportNotes, ImportOptions, ImportStart, Opened, ReceiptAsk, SaveAsOptions } from "./lib/backend";
  import SidePanel, { type PanelTab } from "./lib/SidePanel.svelte";
  import CommandSearch from "./lib/CommandSearch.svelte";
  import readme from "../../README.md?raw";
  import { FEATURE_LINKS, featureDocs, readmeBullets } from "./lib/featureIndex";
  import ShortcutsDialog from "./lib/ShortcutsDialog.svelte";
  import HighlightDialog from "./lib/HighlightDialog.svelte";
  import SubtotalDialog from "./lib/SubtotalDialog.svelte";
  import BatchReplaceDialog from "./lib/BatchReplaceDialog.svelte";
  import PivotDialog from "./lib/PivotDialog.svelte";
  import { followColumns, type PivotSpec } from "./lib/pivot";
  import PromptDialog from "./lib/PromptDialog.svelte";
  import ComputeDialog from "./lib/ComputeDialog.svelte";
  import ProfileDialog from "./lib/ProfileDialog.svelte";
  import { FULL_SAMPLE, MAX_INSIGHT_COLS, QUICK_SAMPLE } from "./lib/insights";
  import ChecksDialog from "./lib/ChecksDialog.svelte";
  import { describeCheck, jumpText, shapeRegex } from "./lib/lint";
  import FieldCheckDialog from "./lib/FieldCheckDialog.svelte";
  import MatchDialog from "./lib/MatchDialog.svelte";
  import { madeText } from "./lib/golden";
  import StreetDialog from "./lib/StreetDialog.svelte";
  import UpdatesDialog from "./lib/UpdatesDialog.svelte";
  import EncryptDialog from "./lib/EncryptDialog.svelte";
  import { MAX_ASKED, TOTAL_CHOICES, totalCopy, type TotalChoice } from "./lib/totalsRow";
  import DiskBanner from "./lib/DiskBanner.svelte";
  import { DISK_AGAIN, DISK_EVERY, diskBanner, takeDisk } from "./lib/disk";
  import SqliteDialog from "./lib/SqliteDialog.svelte";
  import NotesDialog from "./lib/NotesDialog.svelte";
  import WatchDialog from "./lib/WatchDialog.svelte";
  import ReceiptDialog from "./lib/ReceiptDialog.svelte";
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
  import { BASIC_KEYS, cleanFormat, defaultFormat, describeFormat, formatSteps, keyGroups, MAX_ROW_LINES, type DisplayFormat } from "./lib/m9";
  import { defaultPage, newCheck, printHtml, PRINT_MAX_CELLS, PRINT_MAX_ROWS } from "./lib/m8";
  import type { Check, CheckSpec, ComputeSpec, PageSetup, TableCheck } from "./lib/backend";
  import { exportDefaults } from "./lib/transfer";
  import { cleanShortcuts, matchColumns, safeFont, MAX_VIEWS, sessionView, type RecentItem, type Shortcuts, type View } from "./lib/views";

  const DEFAULT_COL_WIDTH = 130;
  const MAX_CELLS_PER_CHANGE = 100_000;

  let info = $state<AppInfo | null>(null);
  let tabs = $state<Tab[]>([]);
  let active = $state(0);
  let grid = $state<ReturnType<typeof Grid>>();
  let dialog = $state<DialogRequest | null>(null);
  let message = $state<string>("");
  /** A button after the status message (cleared by the next message). */
  let messageAction = $state<{ label: string; run: () => void } | null>(null);
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
  let saveAsReq = $state<{ sample: string[][]; done: (o: SaveAsOptions | null, receipt?: ReceiptAsk | null) => void } | null>(null);
  /** Tools > Verify a receipt is open. */
  let receiptOpen = $state(false);
  let exportOpen = $state(false);
  let importing = $state<ImportStart | null>(null);
  /** What the last import kept exactly (counts and column names only), for the command of that name. */
  let lastImportNote = $state("");
  let joinKind = $state<"join" | "append" | null>(null);
  let valuesOpen = $state(false);
  /** How the value editor groups look-alikes when it opens. */
  let valuesMethod = $state<"letters" | "sounds" | "spelling" | "both">("letters");
  let datePick = $state<{ row: number; col: number; text: string } | null>(null);
  let privacyOpen = $state(false);
  /** Label sheets, once the mail screen is open. */
  let mailSheets = $state<[string, import("./lib/mail").LabelSheet][] | null>(null);
  let mailProblem = $state("");
  /** List of my changes: open, and what it found (null while it looks). */
  let changesOpen = $state(false);
  /** The "what changed since a checkpoint" window, and the checkpoint the menu entry would compare with. */
  let sinceFor = $state<{ doc: number; checkpoint: Checkpoint } | null>(null);
  let sinceNewest = $state<{ doc: number; ready: boolean; why: string }>({ doc: -1, ready: false, why: "" });
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
  /** Set while a command picked from a heading's menu runs. */
  let menuCol: number | null = null;
  let renameAsk = $state<{ id: number; col: number; name: string; header: boolean; others: Set<string>; titles: string[] } | null>(null);
  /** Insert > Symbol is open; `caret` is where the cell editor's cursor was. */
  let symbols = $state<{ caret: number | null } | null>(null);
  let findOpen = $state(false);
  let findReplace = $state(false);
  let findPanel = $state<ReturnType<typeof FindPanel>>();
  /** The block Find and Replace keep to (see FindPanel's `block`). */
  let findBlock = $state<Rect | null>(null);
  /** The blocks of a Ctrl+click selection that Find's "Selection" searches. */
  let findBlocks = $state<Rect[]>([]);
  let findOptions = $state<FindOptions>({ ...FIND_DEFAULTS });
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
  let batchOpen = $state(false);
  /** The tab the pivot builder works on (null: closed). */
  let pivotDoc = $state<number | null>(null);
  /** Each tab's last pivot layout, and which tab each pivot tab came from. */
  const pivotSpecs = new Map<number, PivotSpec>();
  /** Each pivot tab's source, layout and the source's column titles then
   *  (Refresh follows the columns by title). */
  const pivotOf = new Map<number, { source: number; spec: PivotSpec; titles: string[] }>();
  /** A one-line question (a view's name, a font name). */
  let prompt = $state<{ title: string; label: string; value: string; note?: string; check?: (v: string) => string; done: (v: string) => void } | null>(null);
  let recent = $state<RecentItem[]>([]);
  /** File > Open recent is showing. */
  let recentOpen = $state(false);
  let panelTab = $state<PanelTab>(prefs.data.panel ?? "summary");
  let computeOpen = $state(false);
  /** The column whose profile is open. */
  let profileCol = $state<number | null>(null);

  // The column insight strip (View › Column insights): a quick sample, then
  // a bigger one, whenever the tab's data or filter changes. Display only:
  // a busy or failed read leaves the strip as it was.
  let insightSeq = 0;
  let insightKey = "";
  $effect(() => {
    const t = tab;
    if (!t || !prefs.data.insights) return;
    void t.version;
    void t.header;
    const cols = Array.from({ length: Math.min(t.info.cols, MAX_INSIGHT_COLS) }, (_, c) => c);
    if (!t.info.indexed || !cols.length) return;
    // After an edit alone (same tab, rows, columns and filter) the strip
    // waits a second, so typing never waits for it.
    const key = JSON.stringify([t.id, t.header, t.info.rows, t.info.cols, t.info.filtered, t.info.filter_cols ?? []]);
    const editOnly = key === insightKey;
    insightKey = key;
    const mine = ++insightSeq;
    const ask = (sample: number) =>
      backend()
        .columnInsights(t.id, { cols, keep_first: t.header, sample, read: READ })
        .then((r) => {
          if (mine !== insightSeq) return false;
          t.insights = { byCol: new Map(r.cols.map((i) => [i.col, i])), sampled: r.sampled, rows: r.rows, revision: r.revision };
          return r.sampled < r.rows;
        });
    const timer = setTimeout(() => {
      void ask(QUICK_SAMPLE)
        .then((more) => (more && mine === insightSeq ? ask(FULL_SAMPLE) : false))
        .catch(() => {});
    }, editOnly ? 1000 : 150);
    return () => clearTimeout(timer);
  });
  /** What the expression screen is for while open. */
  let computeMode = $state<"compute" | "change" | "filter">("compute");
  let checksOpen = $state(false);
  /** The checks dialog opens with its table checks in view. */
  let checksAtTable = $state(false);
  /** X-ray: what this screen shows (cells and columns with a gremlin), and the report dialog. */
  let xrayNow = $state<{ cells: number; cols: number } | null>(null);
  let xrayOpen = $state(false);
  /** The Change encoding picker. */
  let encodingOpen = $state(false);
  let fieldCheckOpen = $state(false);
  let matchOpen = $state(false);
  let streetOpen = $state(false);
  let updatesOpen = $state(false);
  let encryptMode = $state<"save" | "open" | null>(null);
  /** Save encrypted over this file (Save on a tab from an encrypted file). */
  let encryptSame = $state<string | null>(null);
  let sqliteMode = $state<"save" | "open" | null>(null);
  /** The database an Open, drop or double-click chose, for the table list. */
  let sqliteStart = $state<Opened["sqlite"] | null>(null);
  let notesRow = $state<number | null>(null);
  let watchOpen = $state(false);
  let recipeFilesOpen = $state(false);
  let optionsOpen = $state(false);
  /** The print screen: what would print, and whether PDF is the main button. */
  let printReq = $state<{ pdf: boolean; titles: string[] | null; sample: string[][]; cols: number[]; allRows: number; selRows: number; selFirst: number } | null>(null);
  /** Page setup used last (kept while the app runs). */
  let pageSetup = $state<PageSetup | null>(null);
  const READ = { decimal_comma: false, date_order: "mdy" as const };

  const tab = $derived<Tab | undefined>(tabs[active]);

  // ---- helpers ------------------------------------------------------------------
  /** Cells the last change left alone in protected columns, said with the
   *  change's own message (or on its own, just after). */
  let protectNote = 0;
  /** Said with the next message (or alone a moment later): custom row heights
   *  were cleared because rows changed in a way they cannot follow. */
  let rowNote = "";
  // A change on any path (a tool, a job, a recipe, a formula fill, a refresh) that
  // clears the row heights says so at once, not at the next unrelated message.
  // Undo, Redo and an applied view put the column types back in the engine.
  Tab.typesSink = (t, types) => {
    void backend()
      .setColumnTypes(t.id, types)
      .then((info) => {
        t.update(info, true);
        void rememberTypes(t);
      })
      .catch(() => {});
  };
  Tab.rowNoteSink = (note) => {
    rowNote = note;
    window.setTimeout(() => {
      if (rowNote) say("");
    }, 60);
  };
  function noteProtected(info: DocInfo): void {
    const n = info.protected_skipped ?? 0;
    if (!n) return;
    protectNote = n;
    // The change's own message usually follows at once and carries the
    // note; if none comes, the note shows on its own.
    setTimeout(() => {
      if (protectNote) say("");
    }, 500);
  }

  /** What is said when work that takes one block meets a selection of several. */
  const ONE_BLOCK = "That works on one block. Click once to select a single block.";

  /** The one selected block of a tab, or null (the message above said) when
   *  several blocks are selected. */
  /** For dialogs that only open after a command that took one block: the
   *  block, or an empty one in the moment a dialog is closing. */
  const NO_BLOCK: Rect = { row: 0, rows: 0, col: 0, cols: 0 };
  function blockOr(t: Tab): Rect {
    return t.singleRect() ?? NO_BLOCK;
  }

  /** The second guard (the registry is the first): work that changes a file
   *  does not start while several blocks are selected, unless a command that
   *  handles several blocks is running. The backend itself refuses too. */
  function writeBlocked(t: Tab): boolean {
    if (t.extra.length > 0 && !reg.multiOk) {
      say(ONE_BLOCK);
      return true;
    }
    return false;
  }

  function oneBlock(t: Tab): Rect | null {
    const r = t.singleRect();
    if (!r) say(ONE_BLOCK);
    return r;
  }

  /** What to add to the next status line about hidden rows (set when an operation ran with some hidden). */
  let hiddenAfter = "";
  function say(text: string): void {
    if (hiddenAfter) {
      text = text ? `${text} ${hiddenAfter}` : hiddenAfter;
      hiddenAfter = "";
    }
    if (rowNote) {
      text = text ? `${text} ${rowNote}` : rowNote;
      rowNote = "";
    }
    if (protectNote) {
      const n = protectNote;
      protectNote = 0;
      text = `${text ? `${text} ` : ""}${n.toLocaleString()} ${n === 1 ? "cell" : "cells"} in protected columns ${n === 1 ? "was" : "were"} left as ${n === 1 ? "it was" : "they were"}.`;
    }
    message = text;
    messageAction = null;
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

  // Looking at an earlier recipe step: nothing may change or save (that
  // would drop the later steps); the steps panel goes back to the latest.
  const ready = () => !!tab && tab.info.indexed && !tab.busy && !tab.peeking && !tab.info.peeking;
  const hasTab = () => !!tab;

  /** Run an engine call for the active tab and take the new document state. */
  async function apply(f: (t: Tab) => Promise<DocInfo>, dataChanged = true, of?: Tab): Promise<boolean> {
    const t = of ?? tab;
    if (!t) return false;
    if (writeBlocked(t)) return false;
    try {
      const info = await f(t);
      t.update(info, dataChanged);
      if (t.rowHeightsNote) {
        rowNote = t.rowHeightsNote;
        t.rowHeightsNote = "";
        // The caller's own message takes it along; otherwise it is said alone.
        window.setTimeout(() => {
          if (rowNote) say("");
        }, 60);
      }
      noteProtected(info);
      // Any other change after undoing an added header row ends its redo,
      // so redo can no longer switch the header back on.
      if (t.headerOffByUndo && !undoBusy.has(t)) {
        t.headerOffByUndo = false;
        t.headerAddedAt = null;
      }
      // A change after undoing a rename ends its redo.
      if (!undoBusy.has(t) && t.refEdits.some((e) => !e.applied)) t.refEdits = t.refEdits.filter((e) => e.applied);
      if (!undoBusy.has(t) && t.headerEdits.some((e) => !e.applied)) t.headerEdits = t.headerEdits.filter((e) => e.applied);
      return true;
    } catch (e) {
      say(errorText(e));
      return false;
    } finally {
      // A failed change must not lend its name to the next one.
      t.whatNext = null;
      t.wideNext = false;
    }
  }

  function wholeRows(r: Rect, t: Tab): boolean {
    return r.col === 0 && r.cols >= t.info.cols;
  }
  function wholeCols(r: Rect, t: Tab): boolean {
    return r.row <= t.firstRow && r.row + r.rows >= t.info.rows;
  }

  // ---- documents -----------------------------------------------------------------
  function addTab(d: DocInfo, guess = true, fromOtherWindow = false): void {
    const t = new Tab(d, DEFAULT_COL_WIDTH);
    tabs.push(t);
    active = tabs.length - 1;
    void tick().then(() => grid?.focus());
    if (guess) {
      void guessHeader(t);
      void applyRememberedTypes(t, false);
    }
    void loadSidecar(t);
    // A document the second window makes itself (a result tab) is its own.
    if (SECOND_WINDOW && !fromOtherWindow) void backend().windowClaim(handOverOf(t)).catch(() => {});
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
  async function privacyGate(what: string, cols?: [number, number], of?: Tab): Promise<boolean> {
    const t = of ?? tab;
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
          (rep.blanked ? ` ${blanks} could not be read and were left blank, not copied as they were.` : "") +
          guardNote(rep.guarded),
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
      if (on && t.info.path) void rememberHeader(t.info.path, false, true);
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

  /** Remember this file's "first row holds the column names" for the next
   *  time it opens: from the switch or a close or quit with nothing unsaved,
   *  or from a save
   *  (a title row added and then not saved is not kept); never for
   *  a file in private mode (whose earlier choice is forgotten instead). */
  async function rememberTab(t: Tab, saved = false): Promise<void> {
    const path = t.info.path;
    if (!path) return;
    try {
      const p = await backend().privateState(t.id);
      if (p.file || p.folder) await rememberHeader(path, false, true);
      // A file that asked keeps the choice applied, answered or not, so
      // it asks only once.
      else if (t.headerTouched || saved || t.headerAsk !== null) await rememberHeader(path, t.header);
    } catch {
      // Not remembered this time.
    }
  }

  /** Format › Forget this file's title-row choice: the next open guesses. */
  async function forgetHeaderOf(): Promise<void> {
    const t = tab;
    const path = t?.info.path;
    if (!t || !path) return;
    await rememberHeader(path, false, true);
    // Closing it now does not remember it again (nor does the question,
    // which goes: the next open guesses and asks).
    t.headerTouched = false;
    t.headerAsk = null;
    say("Forgot this file's title-row choice; the next time it opens, the app guesses again. Using the switch or saving the file remembers it again.");
  }
  /** Format › Forget remembered title rows: every file's choice. */
  async function forgetAllHeaders(): Promise<void> {
    const n = rememberedHeaders();
    if (!n) {
      say("No title-row choices are remembered.");
      return;
    }
    const a = await ask("Forget remembered title rows?", `The app remembers whether the first row holds the column names for ${plural(n, "file")}. Forget them all? Files opened later are guessed again.`, [
      { id: "forget", label: "Forget them", primary: true },
      { id: "cancel", label: "Cancel" },
    ], "cancel");
    if (a !== "forget") return;
    forgetHeaders();
    for (const t of tabs) {
      t.headerTouched = false;
      t.headerAsk = null;
    }
    say(`Forgot the title-row choices of ${plural(n, "file")}.`);
  }

  /** The title-row bar answered: set it, remember it for this file. */
  async function answerHeader(yes: boolean): Promise<void> {
    const t = tab;
    if (!t) return;
    // The bar stays, now saying what was kept, so the table doesn't move.
    if (t.header !== yes) {
      t.header = yes;
      t.headerAddedAt = null;
      t.headerOffByUndo = false;
      grid?.select(yes && t.cell.row === 0 ? { row: 1, col: t.cell.col } : t.sel.anchor);
      t.bump();
    }
    t.headerTouched = true;
    if (!t.info.modified) await rememberTab(t);
    say(`${yes ? "The first row holds the column names" : "The first row is data"}; ${t.info.modified ? "remembered for this file when you save" : "remembered for this file"}.`);
    void tick().then(() => grid?.focus());
  }

  /** The title-row question is showing on this tab. */
  function headerAsking(): boolean {
    return !!tab?.headerAsk && !tab.headerTouched && !tab.headerAskClosed;
  }

  /** Format › Ask about the title row again: the bar comes back. */
  function askHeaderAgain(): void {
    const t = tab;
    if (!t) return;
    t.dismissed = false;
    t.headerTouched = false;
    t.headerAskClosed = false;
    t.headerAsk = t.headerSure ?? "unsure";
  }

  /** Format › Ask about the title row when a file opens (as in Options). */
  function toggleAskHeader(): void {
    prefs.data.askHeader = !prefs.data.askHeader;
    prefs.save();
    say(prefs.data.askHeader ? "Files with no remembered choice ask about their first row." : "Files no longer ask about their first row; the guess is applied.");
  }

  /** Set "First row is header" from a look at the first rows (the file is
   *  not changed; the user can flip the switch any time). */
  async function guessHeader(t: Tab, fresh = false): Promise<void> {
    // The choice made for this file before wins over a guess (not when it
    // was opened again with another layout: other columns).
    let priv = false;
    if (t.info.path && !t.headerTouched) {
      try {
        const p = await backend().privateState(t.id);
        priv = p.file || p.folder;
        // A private file keeps no choice: forget one made before.
        if (p.file || p.folder) await rememberHeader(t.info.path, false, true);
        const kept = p.file || p.folder || fresh ? null : await recallHeader(t.info.path);
        if (kept !== null && !t.headerTouched) {
          t.header = kept;
          t.headerTouched = true;
          if (kept && t.cell.row === 0) t.sel = { anchor: { row: 1, col: 0 }, focus: { row: 1, col: 0 } };
          t.bump();
          return;
        }
      } catch {
        // Guess instead.
      }
    }
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
      if (t.headerTouched) return;
      // Report title lines above the column titles: the bar offers
      // "Headers start at row N" (C4).
      t.titleLines = t.info.filtered ? 0 : titleLines(sample);
      const sure = headerCertainty(sample);
      t.headerSure = sure;
      const guess = sure === "unsure" ? looksLikeHeader(sample) : sure === "yes";
      if (guess) {
        t.header = true;
        if (t.cell.row === 0) t.sel = { anchor: { row: 1, col: 0 }, focus: { row: 1, col: 0 } };
        t.bump();
      }
      // A file with no remembered choice asks, in the bar (the guess is
      // already applied, so it can be ignored).
      // Private files remember nothing, so they are not asked either.
      if ((prefs.data.askHeader || t.titleLines > 0) && t.info.path && !priv && sample.length >= 2) {
        t.headerAsk = sure;
        t.headerAskClosed = false;
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
    else if (o?.zip) zipPick = o.zip;
    else if (o?.sqlite) {
      // A database or GeoPackage: the table list, with this file already chosen.
      sqliteStart = o.sqlite;
      sqliteMode = "open";
    }
    if (o) void loadRecent();
  }

  /** A zip file with several tables, waiting for a pick. */
  let zipPick = $state<ZipList | null>(null);
  async function openFromZip(list: ZipList, names: string[]): Promise<void> {
    zipPick = null;
    try {
      const docs = await runJob("Opening", () => backend().zipOpen(list.token, names));
      for (const d of docs ?? []) addTab(d);
      if (docs?.length) say(`Opened ${plural(docs.length, "table")} from ${list.name}. To keep changes, save each as a file of its own (File › Save as); the zip is never changed.`);
    } catch (e) {
      say(errorText(e));
    }
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
      return o.import || o.zip || o.sqlite ? "import" : o.doc ? "opened" : "failed";
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

  /** Record panel: the next or previous row, the same column. */
  function recordFlip(d: number): void {
    const t = tab;
    if (!t) return;
    const to = Math.max(t.firstRow, Math.min(t.info.rows - 1, t.cell.row + d));
    grid?.select({ row: to, col: t.cell.col });
  }

  /** Name this moment: a checkpoint in the undo history (a few bytes; the Steps panel lists it). */
  function nameMoment(): void {
    const t = tab;
    if (!t) return;
    prompt = {
      title: "Name this moment",
      label: "Name",
      value: "",
      note: "A checkpoint remembers exactly how the table looks now, so you can come back to it from the Steps panel. It costs almost nothing. It is lost when the undo history is trimmed past it, when a new change replaces what you undid, when you save the file, and when the tab closes.",
      check: (v) => (!v.trim() ? "Give the checkpoint a name." : v.trim().length > 100 ? "Use 100 characters or fewer." : ""),
      done: (name) => {
        t.recordView();
        const looks = t.lookSteps.length;
        void backend()
          .checkpointList(t.id)
          .then(async (before) => {
            const known = new Set(before.map((c) => c.id));
            for (const c of await backend().checkpointAdd(t.id, name)) if (!known.has(c.id)) t.checkpointLooks.set(c.id, looks);
            t.checkpointTick++;
            showPanel("steps");
            say(`Named this moment “${name.trim()}”. It is at the top of the Steps panel.`);
          })
          .catch((e: unknown) => say(errorText(e)));
      },
    };
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
        align: t.columns.aligns.get(c),
        clip: t.columns.clips.has(c) || undefined,
        format: t.columns.formats.get(c) ?? null,
        protect: (t.info.protected ?? []).includes(c) || undefined,
        col_type: typeAt(t.info.typed, c) === "auto" ? undefined : typeAt(t.info.typed, c),
        total: t.columns.totals.get(c),
      })),
      banded: t.banded,
      totals_row: t.totalsOn || undefined,
      row_lines: t.rowLines,
      row_heights: t.rowHeights.empty ? undefined : t.rowHeights.toView(),
      facets: t.facetCards.map((c) => ({ col: c.col, kind: c.kind })),
      record_pins: t.recordPins.length ? [...t.recordPins] : undefined,
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
    // The view's look and its sort and filter undo (and redo) as one step.
    t.recordView();
    const start = t.info.history_pos;
    t.grouping = true;
    try {
      await applyViewSteps(v, t);
    } finally {
      t.endGroup(start);
    }
  }

  /** `strict`: columns not found by title are left out without asking (a reload never guesses). */
  async function applyViewSteps(v: View, t: Tab, strict = false): Promise<void> {
    const headerWas = t.header;
    if (v.header !== t.header) {
      t.header = v.header;
      t.headerTouched = true;
      t.bump();
    }
    // The real titles (the grid only knows those on screen), as views
    // keep them: 60 characters, one line.
    let titles = shownTitles(t);
    // Full names (up to 200 characters) for the view's data-check rules.
    let ruleTitles: string[] = Array.from({ length: t.info.cols }, () => "");
    if (t.header && t.info.rows > 0) {
      try {
        const first = await backend().firstRowNames(t.id);
        ruleTitles = first;
        titles = titles.map((x, c) => {
          const v1 = (first[c] ?? "").replace(/[\r\n\t]+/g, " ").slice(0, 60);
          return v1 || x;
        });
      } catch {
        // Keep what the grid knows.
      }
    }
    const map = matchColumns(v.columns, titles);
    // Columns not found by title take the one in the same place (renamed,
    // say): ask first, as a filter or sort on the wrong column misleads.
    const norm = (s: string) => s.trim().toLowerCase();
    const byPlace = v.columns.flatMap((vc, k) => {
      const c = map[k];
      return c !== null && c !== undefined && vc.title.trim() && norm(titles[c] ?? "") !== norm(vc.title) ? [{ k, c, was: vc.title }] : [];
    });
    if (strict) for (const x of byPlace) map[x.k] = null;
    else if (byPlace.length) {
      const list = byPlace.slice(0, 8).map((x) => `“${x.was}” → column ${x.c + 1}, “${titles[x.c]}”`).join("; ");
      const a = await ask(
        "Use columns in the same place?",
        `The view “${v.name}” names ${byPlace.length === 1 ? "a column" : `${byPlace.length} columns`} this file has under another name (renamed, perhaps): ${list}${byPlace.length > 8 ? `, and ${byPlace.length - 8} more` : ""}.${(v.checks ?? []).some((c) => c.kind.type === "expression") ? " Data check rules in the view that name them follow too." : ""}`,
        [
          { id: "use", label: byPlace.length === 1 ? "Use that column" : "Use those columns", primary: true },
          { id: "skip", label: "Leave them out" },
          { id: "cancel", label: "Don't apply the view" },
        ],
        "cancel",
      );
      if (a === "cancel" || t !== tab) {
        if (t.header !== headerWas) {
          t.header = headerWas;
          t.bump();
        }
        return;
      }
      if (a === "skip") for (const x of byPlace) map[x.k] = null;
    }
    const to = (c: number): number | null => map[c] ?? null;
    let missing = 0;
    const wantTypes: [number, ColumnType][] = [];
    v.columns.forEach((vc, k) => {
      const c = map[k];
      if (c === null || c === undefined) {
        missing++;
        return;
      }
      t.columns.setWidth(c, vc.hidden ? 0 : (vc.width ?? DEFAULT_COL_WIDTH));
      if (vc.wrap) t.columns.wraps.add(c);
      else t.columns.wraps.delete(c);
      if (vc.align === "left" || vc.align === "center" || vc.align === "right") t.columns.aligns.set(c, vc.align);
      else t.columns.aligns.delete(c);
      if (vc.clip) t.columns.clips.add(c);
      else t.columns.clips.delete(c);
      if (vc.total && TOTAL_CHOICES.some(([k]) => k === vc.total)) t.columns.totals.set(c, vc.total);
      else t.columns.totals.delete(c);
      const f = cleanFormat(vc.format);
      if (f) t.columns.formats.set(c, f);
      else t.columns.formats.delete(c);
      if (vc.col_type) wantTypes.push([c, vc.col_type]);
    });
    // What the columns hold, as the view has them (a view without types leaves them alone).
    for (const [c, ty] of wantTypes) {
      try {
        t.update(await backend().setColumnType(t.id, c, ty), false);
      } catch {
        // Leave this column as it was.
      }
    }
    // Protected columns: as the view has them (by title).
    const prot = v.columns.flatMap((vc, k) => {
      const c = map[k];
      return vc.protect === true && c !== null && c !== undefined ? [c] : [];
    });
    if (prot.length || (t.info.protected ?? []).length) {
      try {
        t.update(await backend().setProtected(t.id, prot), false);
      } catch {
        // Kept as it was.
      }
    }
    t.banded = v.banded === true;
    t.totalsOn = v.totals_row === true;
    t.facetCards = (v.facets ?? []).slice(0, 32).flatMap((f) => {
      const c = to(f.col);
      return c === null || c === undefined || c < 0 ? [] : [{ col: c, kind: f.kind === "numbers" || f.kind === "dates" ? f.kind : "values", range: null }];
    });
    t.recordPins = (v.record_pins ?? []).filter((p): p is string => typeof p === "string").slice(0, 200);
    t.rowLines = Math.min(MAX_ROW_LINES, Math.max(1, Math.round(v.row_lines ?? 1) || 1));
    {
      // Rows past this file's end (a view from another file) are dropped, not an error.
      const { heights, dropped } = RowHeights.fromView(v.row_heights ?? [], t.info.total_rows);
      t.rowHeights.restore(heights.snapshot());
      // (Said with the message that follows: "Applied the view".)
      if (dropped) rowNote = `${plural(dropped, "row height")} in that view did not fit this file and ${dropped === 1 ? "was" : "were"} left out.`;
    }
    t.frozenRows = Math.min(v.freeze_rows, Math.max(0, t.info.rows - 1));
    t.frozenCols = Math.min(v.freeze_cols, Math.max(0, t.info.cols - 1));
    t.highlights = v.highlights.flatMap((h) => {
      const c = to(h.col);
      return c === null ? [] : [{ ...h, col: c }];
    });
    t.highlightVersion++;
    // Row rules name columns by the view's titles: rewrite them for the
    // columns this file uses (those taken by place have other names).
    const viewTitles = v.columns.map((c) => c.title);
    const ruled = remapChecks(v.checks ?? [], viewTitles, ruleTitles, (k) => to(k));
    t.checks = ruled.checks.flatMap((c) => {
      const k = placeCheck(c, to);
      return k === null ? [] : [k];
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
    say(`Applied the view “${v.name}”.${missing ? ` ${plural(missing, "column")} in the view had no match in this file.` : ""}${ruled.changed ? ` ${ruled.changed === 1 ? "1 data check rule names" : `${ruled.changed} data check rules name`} this file's columns now.` : ""}`);
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
  async function finishImport(start: ImportStart, o: ImportOptions, header: boolean, notes: ImportNotes | null = null): Promise<void> {
    importing = null;
    say(`Importing ${start.name}…`);
    const poll = window.setInterval(async () => {
      const f = await backend().importProgress(start.id).catch(() => null);
      if (f !== null) say(`Importing ${start.name}… ${Math.round(f * 100)}%`);
    }, 300);
    try {
      const d = await backend().importFinish(start.id, o);
      lastImportNote = notes?.summary ?? "";
      addTab(d, false);
      const nt = tabs[tabs.length - 1];
      if (nt) {
        nt.header = header;
        nt.headerTouched = true;
        nt.bump();
      }
      say(`Imported ${plural(Math.max(0, d.rows - (header ? 1 : 0)), "row")} and ${plural(d.cols, "column")}. It is not saved yet: Save As (F12) writes it as delimited text.${notes?.summary ? ` ${notes.summary}` : ""}`);
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
  async function askSaveOptions(t: Tab): Promise<{ options: SaveAsOptions; receipt: ReceiptAsk | null } | null> {
    const sample = await sampleRows(t);
    return new Promise((resolve) => {
      saveAsReq = {
        sample,
        done: (o, receipt) => {
          saveAsReq = null;
          resolve(o ? { options: o, receipt: receipt ?? null } : null);
        },
      };
    });
  }

  let exportTabs = $state<{ id: number; name: string; why: string }[]>([]);
  /** The open tabs for a workbook of several, with why a tab is left out
   *  unless ticked (private, encrypted, an audit record). */
  async function exportTabList(): Promise<{ id: number; name: string; why: string }[]> {
    return Promise.all(
      tabs.map(async (x) => {
        const enc = await backend().encryptedOrigin(x.id).catch(() => null);
        const why = enc ? "from an encrypted file" : x.private.file || x.private.folder ? "private" : /^Sample record/.test(x.info.name) ? "an audit record" : "";
        return { id: x.id, name: x.info.name, why };
      }),
    );
  }

  async function exportRun(o: ExportOptions, to: "file" | "clipboard", tabIds?: number[]): Promise<void> {
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
        if (tabIds && !tabIds.length) {
          say("Tick at least one tab for the workbook.");
          return;
        }
        // Each sheet gets its own tab's on-screen widths.
        const widthsOf = (id: number) => {
          const x = tabs.find((y) => y.id === id);
          return x ? Array.from({ length: Math.min(x.info.cols, 16_384) }, (_, c) => x.columns.customWidth(c) ?? 0) : [];
        };
        const r = await runJob("Exporting", () => (tabIds ? backend().exportBook(tabIds, o, tabIds.map(widthsOf)) : backend().exportFile(t.id, o)));
        if (!r) return;
        if (r.files?.length) say(`Exported ${plural(r.rows, "row")} to ${plural(r.files.length, "file")}: ${r.files[0]} to ${r.files[r.files.length - 1]}, each with the title row.`);
        else say(`Exported ${plural(r.rows, "row")}${r.sheets > 1 ? ` on ${plural(r.sheets, "sheet")}` : ""}.${noteLines(t) > 0 ? " The report notes (lines above and below the table) are not in the export: only Save writes them." : ""}`);
      }
    } catch (e) {
      say(errorText(e));
    } finally {
      void tick().then(() => grid?.focus());
    }
  }

  /** Column titles of an open document (first row, or "Column N"); the
   *  first 2,000, or with `all` every one (read 2,000 at a time). */
  async function titlesOf(id: number, header: boolean, all = false): Promise<string[]> {
    const t = tabs.find((x) => x.id === id);
    const cols = all ? (t?.info.cols ?? 1) : Math.min(t?.info.cols ?? 1, 2000);
    const first: string[] = [];
    if (header) {
      try {
        for (let c0 = 0; c0 < cols; c0 += 2000) {
          const n = Math.min(2000, cols - c0);
          const v = await backend().viewport(id, 0, 1, c0, n, 60);
          first.push(...v.cells.slice(0, v.cols));
        }
      } catch {
        // Numbers only.
      }
    }
    return Array.from({ length: cols }, (_, c) => (first[c] || `Column ${c + 1}`).replace(/[\r\n\t]+/g, " "));
  }

  function otherTables(): OtherTable[] {
    return tabs.map((t) => ({ id: t.id, name: t.info.name, header: t.header, cols: t.info.cols, hidden: t.info.total_rows - t.info.rows }));
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
      if (!d && o?.zip) say(`${o.zip.name} holds ${o.zip.entries.length} tables: open the one you want with File › Open first, then pick it here.`);
      if (!d && o?.sqlite) say("That is a database: open the table you want with File › Open first, then pick it here.");
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
    let receipt: ReceiptAsk | null = null;
    t.busy = "Saving…";
    try {
      for (;;) {
        try {
          if ((saveAs || !t.info.path) && !options) {
            t.busy = null;
            const picked = await askSaveOptions(t);
            if (!picked) return false;
            options = picked.options;
            receipt = picked.receipt;
            t.busy = "Saving…";
            saveAs = true;
          }
          const d = await backend().save(t.id, { saveAs, overwriteChanged, allowLossy, options, receipt });
          if (!d) return false; // dialog cancelled
          t.update(d, true);
          if (!d.can_undo) t.afterSave();
          void rememberTab(t, true);
          const note = receipt ? await backend().receiptNote(t.id).catch(() => null) : null;
          say(`Saved ${d.name}.${note ? ` ${note}.` : ""}`);
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
    if (!t.info.modified) await rememberTab(t);
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

  // ---- a tab in its own window ----
  /** A tab's look and place, to show it in the other window (the file itself
   *  is not read again: documents and undo history are shared). */
  function handOverOf(t: Tab): HandOver {
    return {
      doc: t.id,
      state: exportHandTab(t),
      // The same layout-only view the remembered session keeps.
      view: sessionView(currentView(t, t.info.name)),
      row: t.cell.row,
      col: t.cell.col,
    };
  }

  /** The tab leaves this window; its file stays open (it is shown elsewhere). */
  function detachTab(i: number): void {
    const t = tabs[i];
    if (!t) return;
    if (copiedFrom === t.id) copiedFrom = null;
    tabs.splice(i, 1);
    active = Math.min(active, Math.max(0, tabs.length - 1));
    void tick().then(() => grid?.focus());
  }

  async function moveTabToWindow(): Promise<void> {
    const t = tab;
    if (!t || SECOND_WINDOW) return;
    if (!t.info.indexed) {
      say("Wait until the file has been read, then move it.");
      return;
    }
    if (t.busy) {
      say("This file is busy: wait until it has finished, then move it.");
      return;
    }
    try {
      await backend().windowMove(handOverOf(t));
      sessionKey = ""; // the next save (10 s) adds the file window 2 now holds
    } catch (e) {
      say(errorText(e));
      return;
    }
    detachTab(tabs.indexOf(t));
    say(`Moved ${t.info.name} to a new window. Edit > ... or the tab's right-click menu moves it back.`);
  }

  async function moveTabBack(): Promise<void> {
    const t = tab;
    if (!t || !SECOND_WINDOW) return;
    try {
      await backend().windowMoveBack(handOverOf(t));
    } catch (e) {
      say(errorText(e));
      return;
    }
    detachTab(tabs.indexOf(t));
  }

  /** A tab arrives from the other window (or comes back on its own). */
  async function adoptHandOver(h: HandOver): Promise<void> {
    const have = tabs.findIndex((x) => x.id === h.doc);
    if (have >= 0) {
      active = have;
      if (h.note) say(h.note);
      return;
    }
    try {
      const d = await backend().docInfo(h.doc);
      addTab(d, false, true);
      const t = tabs[tabs.length - 1]!;
      const st = h.state as HandTab | null;
      if (st && typeof st === "object" && "snap" in st) importHandTab(t, st);
      else void guessHeader(t);
      active = tabs.length - 1;
      await tick();
      grid?.select(t.sel.anchor, t.sel.focus, true, t.sel.active);
      if (h.note) say(h.note);
    } catch (e) {
      say(errorText(e));
    }
  }

  async function takeHandovers(): Promise<void> {
    let list: HandOver[];
    try {
      list = await backend().windowTake();
    } catch {
      return;
    }
    for (const h of list) await adoptHandOver(h);
  }

  /** The second window tells the app what it holds, and that it is alive. */
  function syncSecond(): void {
    void backend().windowSync(tabs.map(handOverOf)).catch(() => {});
  }

  /** The second window closes: its latest views first, then the app hands every
   *  tab back to the main window (it does so without us, if we cannot). */
  async function closeSecond(): Promise<void> {
    try {
      await backend().windowSync(tabs.map(handOverOf));
    } catch {
      // The app returns the files from what it last had.
    }
    await backend().windowCloseSelf().catch(() => {});
  }

  // ---- pick up where you left off ----
  /** Tabs are remembered only once the start-up offer is settled (so the
   *  empty start does not overwrite last time's list). */
  let sessionReady = false;
  /** The last tab a file from Windows (or a second launch) brought in while
   *  start-up runs: it stays in front after last time's files reopen. */
  let launchTab: Tab | null = null;
  let sessionKey = "";

  async function saveSession(force = false): Promise<void> {
    if (!sessionReady || prefs.data.reopenLast === "never") return;
    const list = tabs
      .filter((t) => t.info.path && !t.private?.file && !t.private?.folder)
      .map((t) => ({
        doc: t.id,
        // Layout only: never anything that can hold values from the file.
        view: sessionView(currentView(t, t.info.name)),
        row: t.cell.row,
        col: t.cell.col,
        active: t === tab,
      }));
    const key = JSON.stringify(list);
    // (forced when quitting: the files held by window 2 are added by the app, so this window's list can look unchanged)
    if (key === sessionKey && !force) return;
    sessionKey = key;
    try {
      await backend().sessionSave(list);
    } catch {
      sessionKey = "";
    }
  }

  /** At start: offer (or just do) reopening last time's tabs. */
  async function startSession(): Promise<void> {
    try {
      if (prefs.data.reopenLast === "never") {
        await backend().sessionClear();
        return;
      }
      // Files open already (one opened from Windows, which started the
      // app) are not offered back: only the others are.
      const items = sessionToOffer(await backend().sessionList());
      if (!items.length) return;
      if (prefs.data.reopenLast === "ask") {
        const names = items.slice(0, 6).map((i) => i.name).join(", ") + (items.length > 6 ? `, and ${items.length - 6} more` : "");
        const a = await ask(
          `Reopen ${items.length === 1 ? "the file" : `${items.length} files`} from last time?`,
          `${names}. Each opens at the same cell with its hidden columns and widths${items.some((i) => i.changed) ? "; a file changed since opens fresh" : ""}.`,
          [
            { id: "open", label: "Reopen", primary: true },
            { id: "no", label: "Not now" },
          ],
          "open",
        );
        if (a !== "open") return;
      }
      await reopenSession(items);
    } catch {
      // Nothing remembered (or unreadable): start empty.
    } finally {
      sessionReady = true;
    }
  }

  async function reopenSession(items: { index: number; name: string; active: boolean }[]): Promise<void> {
    let activeTab: Tab | null = null;
    const fresh: string[] = [];
    for (const it of items) {
      try {
        const { opened: o, place } = await backend().sessionOpen(it.index);
        if (!o.doc) {
          opened(o);
          // An import screen waits for the user: the rest stay unopened.
          break;
        }
        const known = tabs.find((x) => x.id === o.doc!.id);
        if (!known) opened(o);
        const t = tabs.find((x) => x.id === o.doc!.id);
        if (!t) continue;
        if (it.active) activeTab = t;
        if (place.changed) fresh.push(it.name);
        // The view and the cell need the file read first.
        for (let k = 0; k < 600 && !t.info.indexed && !t.info.error; k++) await new Promise((r) => setTimeout(r, 100));
        if (!t.info.indexed) continue;
        if (place.view && !known) {
          active = tabs.indexOf(t);
          await tick();
          // As View › Apply a saved view does (its look, filter and sort
          // undo as one step).
          await applyView(place.view as View);
        }
        active = tabs.indexOf(t);
        await tick();
        const row = Math.min(place.row, Math.max(0, t.info.rows - 1));
        const col = Math.min(place.col, Math.max(0, t.info.cols - 1));
        grid?.select({ row, col }, { row, col });
      } catch (e) {
        say(`${it.name}: ${errorText(e)}`);
      }
    }
    // The file just double-clicked in Windows stays in front, whenever
    // its open finished; otherwise last time's front tab.
    const front = launchTab && tabs.includes(launchTab) ? launchTab : activeTab;
    if (front && tabs.includes(front)) active = tabs.indexOf(front);
    if (fresh.length) say(`${fresh.join(", ")} changed since last time, so ${fresh.length === 1 ? "it was" : "they were"} opened fresh (the view was not put back).`);
  }

  /** The quit question is open (a second close request meanwhile is ignored). */
  let quitAsking = false;

  /** Quit: ask about what is unsaved in EITHER window (the app knows, not this
   *  window's tab list), then hand the app the list the question showed. If
   *  something else was changed meanwhile, the app does not quit and the
   *  question comes round again (with the new list). */
  async function quit(): Promise<void> {
    if (SECOND_WINDOW) {
      await closeSecond();
      return;
    }
    if (quitAsking) return;
    quitAsking = true;
    try {
      // The app says "ask_again" when something changed after the question was
      // shown; the question then comes round again with the new list.
      for (;;) {
      let r: ModifiedReport;
      try {
        r = await backend().modifiedDocs();
      } catch (e) {
        say(errorText(e));
        return;
      }
      if (r.busy.length) {
        await ask(
          "A file is still being saved or changed",
          `${r.busy.join(", ")} ${r.busy.length === 1 ? "is" : "are"} still busy. Wait until it has finished, then quit.`,
          [{ id: "ok", label: "OK", primary: true }],
          "ok",
        );
        return;
      }
      if (r.docs.length || r.jobs.length) {
        const parts: string[] = [];
        if (r.docs.length) parts.push(`${r.docs.length === 1 ? "1 file has" : `${r.docs.length} files have`} unsaved changes: ${r.docs.map((d) => d.name).join(", ")}.`);
        if (r.jobs.length) parts.push(`Still running, and quitting stops it: ${r.jobs.join(", ")}.`);
        const a = await ask(
          r.docs.length ? "Quit without saving?" : "Quit now?",
          parts.join(" "),
          [
            { id: "quit", label: r.docs.length ? "Quit and lose changes" : "Quit", danger: true },
            { id: "cancel", label: "Cancel", primary: true },
          ],
          "cancel",
        );
        if (a !== "quit") return;
      }
      await Promise.all(tabs.filter((t) => !t.info.modified).map((t) => rememberTab(t)));
      await saveSession(true);
      const reply = await backend().quitApp(r.docs.map((d) => ({ id: d.id, generation: d.generation }))).catch(() => "exiting");
      if (reply !== "ask_again") return;
      }
    } finally {
      quitAsking = false;
    }
  }

  /** Document whose "File layout" dialog is open. */
  let layoutFor = $state<{ doc: number; modified: boolean } | null>(null);

  function chooseLayout(): void {
    const t = tab;
    if (t?.info.path) layoutFor = { doc: t.id, modified: t.info.modified };
  }

  /** Read the file as another encoding: an ASCII-compatible one in place (one undo step), any other by reopening the
   *  file from disk (a new tab replaces this one; Cancel or an error keeps it). Refused with unsaved changes. */
  async function changeEncoding(c: EncodingChoice, now: EncodingChoice | undefined): Promise<void> {
    const t = tab;
    if (!t) return;
    if (c.direct && now?.direct) {
      if (await apply((x) => backend().encodingApply(x.id, c.label), true, t)) {
        encodingOpen = false;
        say(`The file is read as ${c.name} now. Ctrl+Z reads it as before. Save keeps its bytes as they are; use Save As to write it in another encoding.`);
      }
      return;
    }
    if (t.info.modified) {
      say("Save or undo your changes first: reopening reads the file again from disk.");
      return;
    }
    encodingOpen = false;
    const keep: Layout | null =
      t.info.layout_confidence === null
        ? { delimiter: t.info.delimiter, quote: t.info.quote, escape: t.info.escape, merge: t.info.merge, skip_lines: t.info.preamble_lines }
        : null;
    await reopenTab(t, t.info.line_by_line, keep, false, c.label);
    say(`The file is open again, read as ${c.name}.`);
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
  async function reopenTab(t: Tab, lineByLine: boolean, layout: Layout | null, fresh = false, encoding?: string): Promise<void> {
    try {
      const d = encoding ? await backend().reopenEncoding(t.id, encoding, lineByLine, layout) : await backend().reopen(t.id, lineByLine, layout);
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
      if (fresh) void tick().then(() => guessHeader(nt, true));
      // The same columns: they stay protected.
      else if (t.info.protected?.length) {
        void backend()
          .setProtected(d.id, t.info.protected)
          .then((x) => nt.update(x, false))
          .catch(() => {});
      }
    } catch (e) {
      say(errorText(e));
    }
  }

  // ---- another program changed a file ------------------------------------------------

  let diskLooking = false;
  let diskLooked = 0;
  /** Look at every open file; `say`s the result when the person asked. */
  async function checkDisk(asked = false): Promise<void> {
    if (diskLooking || !tabs.length) return;
    diskLooking = true;
    diskLooked = Date.now();
    try {
      if (asked) {
        for (const t of tabs) {
          t.diskIgnored = "";
          t.diskMuted = false;
        }
      }
      const found = await backend().diskCheck();
      for (const e of found) {
        const t = tabs.find((x) => x.id === e.doc);
        if (t) takeDisk(t, e);
      }
      if (asked) {
        const n = tabs.filter((t) => t.diskNote).length;
        say(n ? `${plural(n, "open file")} changed or removed outside the app: see the banner on its tab.` : "No outside changes found in the open files.");
      }
    } catch (e) {
      if (asked) say(errorText(e));
    } finally {
      diskLooking = false;
    }
  }

  // A look on switching tabs, on coming back to the window, and every little while it has the focus.
  $effect(() => {
    void tab?.id;
    // Flipping through tabs asks once, not at every tab.
    untrack(() => {
      if (Date.now() - diskLooked >= DISK_AGAIN) void checkDisk();
    });
  });
  $effect(() => {
    const look = () => void checkDisk();
    const timer = setInterval(() => {
      if (document.hasFocus()) look();
    }, DISK_EVERY);
    window.addEventListener("focus", look);
    return () => {
      clearInterval(timer);
      window.removeEventListener("focus", look);
    };
  });

  async function reloadFromDisk(t: Tab): Promise<void> {
    if (t.info.modified) {
      const a = await ask(
        "Reload and lose your changes?",
        `${t.info.name} has changes you have not saved. Reloading reads the file on disk and drops them.`,
        [
          { id: "reload", label: "Reload and lose them", danger: true },
          { id: "compare", label: "Compare first" },
          { id: "keep", label: "Keep mine", primary: true },
        ],
        "keep",
      );
      if (a === "compare") return compareWithDisk(t);
      if (a !== "reload") {
        t.diskIgnored = t.diskNote?.token ?? "";
        return;
      }
    }
    const view = $state.snapshot(currentView(t, "before the reload")) as View;
    const before = view.columns.map((c) => c.title.trim().toLowerCase()).filter(Boolean);
    const i = tabs.indexOf(t);
    if (i < 0) return;
    await reopenTab(t, t.info.line_by_line, null);
    const nt = tabs[i];
    if (!nt || nt === t) return;
    for (let k = 0; k < 600 && !nt.info.indexed; k++) await new Promise((r) => setTimeout(r, 100));
    if (tab !== nt || !nt.info.indexed) return;
    await applyViewSteps(view, nt, true);
    const now = new Set(shownTitles(nt).map((x) => x.trim().toLowerCase()));
    const gone = nt.header ? before.filter((x) => !now.has(x)).length : 0;
    say(`Reloaded ${nt.info.name}.${gone ? ` ${plural(gone, "column")} from before ${gone === 1 ? "is" : "are"} not there any more; settings that pointed at ${gone === 1 ? "it" : "them"} were dropped.` : ""}`);
  }

  async function compareWithDisk(t: Tab): Promise<void> {
    try {
      if (tabs.indexOf(t) < 0) return;
      active = tabs.indexOf(t);
      const r = await runJob("Comparing with the file on disk", () => backend().diskCompare(t.id, t.header));
      if (!r) return;
      for (const d of r) addTab(d, false);
      say("Opened the file as it is on disk, and the differences from what you opened (your unsaved edits are not part of it). Tools > Take changes from a newer file brings chosen ones into your tab.");
    } catch (e) {
      say(errorText(e));
    }
  }

  // ---- editing ----------------------------------------------------------------------
  /** One value into every cell of all the selected blocks (Ctrl+Enter, a
   *  single value pasted, Delete with ""): one undo step, each cell once. */
  async function fillAreasValue(t: Tab, value: string, what: string): Promise<boolean> {
    const areas = t.areas;
    const n = cellCountAreas(areas);
    if (n > MAX_CELLS_PER_CHANGE) {
      say(`Filling several blocks works on up to ${MAX_CELLS_PER_CHANGE.toLocaleString()} cells at once. Select fewer cells, or use one block.`);
      return false;
    }
    t.whatNext = what;
    const ok = await reg.withMulti(() => apply((x) => backend().fillAreas(x.id, areas, value, hiddenColsOf(x)), true, t));
    if (ok) say(`${value === "" ? "Cleared" : "Filled"} ${plural(n, "cell")} in ${plural(areas.length, "block")}. Ctrl+Z undoes it.`);
    return ok;
  }

  async function commitCell(row: number, col: number, value: string, fill: boolean, what = "filling cells"): Promise<boolean> {
    const t = tab;
    if (!t) return false;
    // Typed (not pasted): "=…" is a formula, "'=…" text.
    if (what !== "pasting") {
      const typed = typedCell(value);
      if ("formula" in typed) return runCellFormula(t, row, col, typed.formula, fill);
      value = typed.text;
    }
    // Typing into one of several selected blocks writes the active cell only
    // (as in Excel); Ctrl+Enter writes it into every block.
    if (!fill && t.extra.length) t.extra = [];
    if (fill && t.extra.length) return fillAreasValue(t, value, what);
    if (fill) {
      const r = oneBlock(t);
      if (!r) return false;
      // A big block is one engine job (with progress and Cancel).
      if (cellCount(r) > MAX_CELLS_PER_CHANGE) return fillRect(r, value);
      t.whatNext = what;
      const ok = await apply((x) => backend().fillCells(x.id, r.row, r.rows, r.col, r.cols, value, hiddenColsOf(x)));
      if (ok) {
        const note = await hiddenRowsNote(t, r);
        if (note) say(`Filled ${plural(r.rows * shownColsIn(t, r), "cell")}.${note} Undo (Ctrl+Z) takes it back.`);
      }
      return ok;
    }
    if (value === t.cache.get(row, col) && !t.cache.isTruncated(row, col)) return true;
    // A protected column: the edit closes and nothing changes (its title
    // cell is renamed with Rename column instead).
    if (t.info.protected?.includes(col)) {
      say(`“${columnTitle(t, col)}” is protected, so nothing in it changes. Right-click its heading › Protect column to switch it off${t.header && row === 0 ? ", or use Rename column for its title" : ""}.`);
      return true;
    }
    const no = shownRowNo(t, row);
    t.whatNext = no === null ? "typing in a cell" : `typing in R${no} C${col + 1}`;
    return apply((x) => backend().setCells(x.id, [[row, col, value]]));
  }

  /** A formula typed in a cell (=B2*2, =SUM(C2:C9), =[Price]*[Qty]): it
   *  runs once and the cell keeps the result. With Ctrl+Enter it fills
   *  every selected cell, references moving with each one. */
  async function runCellFormula(t: Tab, row: number, col: number, formula: string, fill: boolean): Promise<boolean> {
    let targets: [number, number][] = [];
    if (fill) {
      const r = oneBlock(t);
      if (!r) return false;
      if (cellCount(r) > MAX_CELLS_PER_CHANGE) {
        say(`A formula fills at most ${MAX_CELLS_PER_CHANGE.toLocaleString()} cells at once. For a whole column use Data › Computed column.`);
        return false;
      }
      const skip = new Set([...hiddenColsOf(t), ...(t.info.protected ?? [])]);
      for (let y = r.row; y < r.row + r.rows; y++)
        for (let x = r.col; x < r.col + r.cols; x++) if (!skip.has(x)) targets.push([y, x]);
      if (!targets.length) {
        say("Nothing to fill: the selected columns are hidden or protected.");
        return true;
      }
    } else if (t.info.protected?.includes(col)) {
      say(`“${columnTitle(t, col)}” is protected, so nothing in it changes. Right-click its heading › Protect column to switch it off.`);
      return true;
    }
    t.whatNext = fill ? "filling a formula" : "a formula";
    let errors = 0;
    let firstError: string | null = null;
    let written = 0;
    try {
      // A job: a long fill shows progress and Cancel.
      const rep = await runJob(fill ? "Filling the formula" : "Running the formula", () =>
        backend().cellFormula(t.id, {
          formula,
          row,
          col,
          targets,
          keep_first: t.header,
          read: READ,
          now: localDays(),
        }),
      );
      if (!rep) return true;
      ({ errors, written } = rep);
      firstError = rep.first_error;
      if (written) {
        t.hasFormulas = true;
        t.update(rep.info, true);
        noteProtected(rep.info);
      }
    } catch (e) {
      say(`That formula doesn't work: ${errorText(e)}. To keep it as text, start it with an apostrophe ('=…).`);
      return false;
    }
    if (errors && !written) {
      say(`The formula gives ${firstError ?? "an error"}${targets.length > 1 ? " in every cell" : ""}, so nothing changed.`);
      return targets.length > 1;
    }
    if (errors) say(`Filled ${plural(written, "cell")}; ${plural(errors, "cell")} gave ${firstError ?? "an error"} and kept ${errors === 1 ? "its" : "their"} value. Undo (Ctrl+Z) takes it back.`);
    else if (targets.length > 1) say(`Filled ${plural(written, "cell")} with the formula. Undo (Ctrl+Z) takes it back.`);
    return true;
  }

  // ---- counts tab ---------------------------------------------------------------
  /** Each value of the column (or of two or three selected columns
   *  together) with its count, percent and a bar, in a tab of its own. */
  async function openCounts(): Promise<void> {
    const t = tab;
    if (!t) return;
    const r = oneBlock(t);
    if (!r) return;
    const hidden = new Set(hiddenColsOf(t));
    const picked = r.cols > 1 && r.cols <= 3 ? Array.from({ length: r.cols }, (_, i) => r.col + i).filter((c) => !hidden.has(c)) : [t.cell.col];
    const cols = picked.length ? picked : [t.cell.col];
    try {
      const d = await runJob("Counting values", () => backend().newTable(t.id, { op: "counts", spec: { cols, keep_first: t.header } }));
      if (!d) return;
      addTab(d, false);
      const nt = tabs[tabs.length - 1];
      if (!nt) return;
      nt.header = true;
      nt.headerTouched = true;
      nt.countsOf = { source: t.id, cols };
      // A bar in the Count column.
      nt.scales = [{ col: cols.length, kind: "bar" }];
      void loadRanges(nt);
      nt.bump();
      const lines = Math.max(0, d.rows - 1);
      if (lines >= 1_000_000) {
        say(`Over 1,000,000 different values in ${plural(Math.max(0, t.info.rows - (t.header ? 1 : 0)), "shown row")}: the 1,000,000 most common are listed. For one line per value, Data › Remove duplicates; for the column's shape, Data › Column profile.`);
        return;
      }
      say(`${plural(lines, "different value")} in ${plural(Math.max(0, t.info.rows - (t.header ? 1 : 0)), "shown row")}. Select lines and press Enter (or right-click › Show these rows) to see their rows in ${t.info.name}.`);
    } catch (e) {
      say(errorText(e));
    }
  }

  /** From a counts tab: show (or hide) the rows of the selected lines in
   *  the tab it counts. */
  async function countsShow(hide: boolean): Promise<void> {
    const ct = tab;
    const link = ct?.countsOf;
    if (!ct || !link) return;
    const src = tabs.find((x) => x.id === link.source);
    if (!src) {
      say("The tab these counts came from is closed.");
      return;
    }
    const r = oneBlock(ct);
    if (!r) return;
    const first = Math.max(1, r.row);
    const last = r.row + r.rows - 1;
    if (last < 1) {
      say("Pick one or more lines (not the titles).");
      return;
    }
    if (last - first + 1 > 200) {
      say("Pick at most 200 lines at once.");
      return;
    }
    const lines: string[][] = [];
    for (let row = first; row <= last; row++) {
      const vals: string[] = [];
      for (let k = 0; k < link.cols.length; k++) vals.push((await backend().cellText(ct.id, row, k)).text);
      lines.push(vals);
    }
    active = tabs.indexOf(src);
    await tick();
    if (link.cols.length === 1 && !hide) {
      const vals = lines.map((l) => l[0]!);
      const blanks = vals.includes("");
      await setRule(link.cols[0]!, { type: "values", values: vals.filter((v) => v !== ""), blanks });
      return;
    }
    // Several columns, or hiding: a formula over the shown rows.
    const ref = (c: number) => columnRef(src.header ? columnTitle(src, c) : "", c);
    const q = (s: string) => `"${s.replace(/"/g, '""')}"`;
    const one = (l: string[]) => {
      const parts = l.map((v, k) => `${ref(link.cols[k]!)} = ${q(v)}`);
      return parts.length === 1 ? parts[0]! : `AND(${parts.join(", ")})`;
    };
    const any = lines.length === 1 ? one(lines[0]!) : `OR(${lines.map(one).join(", ")})`;
    const expr = hide ? `NOT(${any})` : any;
    computeMode = "filter";
    await runCompute({ expr, title: "", target: null, at: null, keep_first: src.header, read: { decimal_comma: false, date_order: "mdy" }, now: localDays() });
  }

  /** Clear a block of cells (the fill handle dragged back over it). */
  async function clearRect(r: Rect): Promise<void> {
    const t = tab;
    if (!t || !ready()) return;
    if (cellCount(r) > MAX_CELLS_PER_CHANGE) {
      await fillRect(r, "");
      return;
    }
    t.whatNext = "clearing cells";
    if (await apply((x) => backend().fillCells(x.id, r.row, r.rows, r.col, r.cols, "", hiddenColsOf(x)))) {
      t.noteStepSel({ anchor: { row: r.row, col: r.col }, focus: { row: r.row + r.rows - 1, col: r.col + r.cols - 1 } });
      say(`Cleared ${plural(r.rows * shownColsIn(t, r), "cell")}.${await hiddenRowsNote(t, r)} Ctrl+Z undoes it.`);
    }
  }

  async function clearSelection(): Promise<void> {
    const t = tab;
    if (!t) return;
    if (t.extra.length) return void (await clearAreas(t));
    const r = oneBlock(t);
    if (!r) return;
    if (cellCount(r) > MAX_CELLS_PER_CHANGE) {
      await fillRect(r, "");
      return;
    }
    t.whatNext = "clearing cells";
    if (await apply((x) => backend().fillCells(x.id, r.row, r.rows, r.col, r.cols, "", hiddenColsOf(x)))) {
      const note = await hiddenRowsNote(t, r);
      if (note) say(`Cleared ${plural(r.rows * shownColsIn(t, r), "cell")}.${note} Ctrl+Z undoes it.`);
    }
  }

  /** Delete over several blocks: one undo step. Up to the per-change cap it is
   *  one engine edit; past it each block is cleared as an engine job and the
   *  steps are grouped so Ctrl+Z takes them back together. */
  async function clearAreas(t: Tab): Promise<void> {
    const areas = t.areas;
    const n = cellCountAreas(areas);
    if (n <= MAX_CELLS_PER_CHANGE) {
      await fillAreasValue(t, "", "clearing cells");
      return;
    }
    const parts = disjoint(areas);
    t.recordView();
    const start = t.info.history_pos;
    t.grouping = true;
    let done = 0;
    try {
      for (const a of parts) {
        const rep = await reg.withMulti(() => runOp({ op: "fill_rect", spec: { row: a.row, rows: a.rows, col: a.col, cols: a.cols, value: "" } }, "Clearing", t));
        if (!rep) break;
        done += rep.cells || cellCount(a);
      }
    } finally {
      t.endGroup(start);
    }
    if (done) say(`Cleared ${plural(done, "cell")} in ${plural(areas.length, "block")}. Ctrl+Z undoes it.`);
  }

  /** One value into every cell of a big block (clearing it when ""), as an
   *  engine job: whole columns clear at once, however many rows. */
  async function fillRect(r: Rect, value: string): Promise<boolean> {
    const n = cellCount(r);
    const rep = await runOp({ op: "fill_rect", spec: { row: r.row, rows: r.rows, col: r.col, cols: r.cols, value } }, value ? "Filling" : "Clearing");
    if (rep) {
      // Undo selects the block (not a tool over the whole table).
      if (tab) tab.noteStepSel({ anchor: { row: r.row, col: r.col }, focus: { row: r.row + r.rows - 1, col: r.col + r.cols - 1 } });
      say(`${value ? "Filled" : "Cleared"} ${plural(rep.cells || n, "cell")}.${hiddenNote(rep)} Undo (Ctrl+Z) takes it back.`);
    }
    return !!rep;
  }

  /** Copy over several blocks (Ctrl+click), as Excel allows: the blocks must
   *  share the same rows (they go side by side) or the same columns (they go
   *  one under the other). The text goes to the clipboard as one block. */
  async function copyAreas(t: Tab): Promise<void> {
    const areas = disjoint(t.areas);
    const sameRows = areas.every((a) => a.row === areas[0]!.row && a.rows === areas[0]!.rows);
    const sameCols = areas.every((a) => a.col === areas[0]!.col && a.cols === areas[0]!.cols);
    if (!sameRows && !sameCols) {
      say("Copy needs the blocks to line up: all with the same rows (side by side) or all with the same columns (one under the other). Select blocks that line up, or copy one block at a time.");
      return;
    }
    if (cellCountAreas(areas) > MAX_CELLS_PER_CHANGE) {
      say(`Copy works on up to ${MAX_CELLS_PER_CHANGE.toLocaleString()} cells over several blocks. Select fewer cells.`);
      return;
    }
    const ordered = [...areas].sort((a, b) => (sameRows ? a.col - b.col : a.row - b.row));
    try {
      if (cellCountAreas(areas) >= LARGE_COPY && !(await privacyGate("The copy goes to the clipboard, where other programs can read it.", [Math.min(...ordered.map((a) => a.col)), Math.max(...ordered.map((a) => a.col + a.cols)) - Math.min(...ordered.map((a) => a.col))]))) return;
      const texts = await Promise.all(ordered.map(async (a) => parseTsv(await backend().copyCells(t.id, a.row, a.rows, a.col, a.cols, hiddenColsOf(t)))));
      const rows: string[][] = sameRows
        ? texts[0]!.map((_, i) => texts.flatMap((g) => g[i] ?? []))
        : texts.flat();
      await navigator.clipboard.writeText(toTsv(rows));
      copiedFrom = t.id;
    } catch (e) {
      say(errorText(e));
      return;
    }
    say(`Copied ${plural(cellCountAreas(areas), "cell")} from ${plural(areas.length, "block")}.`);
  }

  async function copy(cut: boolean): Promise<void> {
    const t = tab;
    if (!t) return;
    if (t.extra.length) {
      if (cut) say("Cut works on one block. Click once to select a single block. (Copy works on blocks that line up.)");
      else await copyAreas(t);
      return;
    }
    const r = oneBlock(t);
    if (!r) return;
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
      if (x.cut || x.copied) {
        x.cut = null;
        x.copied = null;
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
        clip = await backend().copyCells(t.id, r.row, r.rows, r.col, r.cols, hiddenColsOf(t));
        await navigator.clipboard.writeText(clip);
        copiedFrom = t.id;
      } catch (e) {
        say(errorText(e));
        return;
      }
    }
    if (!kind) {
      if (epoch === cutEpoch && t.info.history_pos === pos) {
        t.copied = { ...r };
        t.bump();
      }
      say(`Copied ${what}. Esc takes the outline away.`);
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
    const r = oneBlock(t);
    if (!r) return false;
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
    // Names this app copied as a line of text (split by commas or the
    // file's delimiter) paste one name per column, as a tab row does; other
    // programs' comma text ("Dayton, OH") stays in one cell.
    // (A trailing line break, which some clipboard tools add, is ignored.)
    if (namesLine && namesLine.sep !== "\t" && text.replace(/\r?\n$/, "") === namesLine.text) {
      text = namesFromText(namesLine.text, namesLine.sep)
        .map((n) => (/[\t"\r\n]/.test(n) ? `"${n.replace(/"/g, '""')}"` : n))
        .join("\t");
    }
    // Several blocks selected: one value goes into every one of them; a bigger
    // block of text has no single place to land.
    if (t.extra.length) {
      const grid1 = parseTsv(text.replace(/\r?\n$/, ""));
      if (grid1.length === 1 && grid1[0]!.length === 1) await fillAreasValue(t, grid1[0]![0] ?? "", "pasting");
      else say("Paste of more than one value works on one block. Click once to select a single block. (One value pastes into every selected block.)");
      return;
    }
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
    let rows = parseTsv(text);
    if (rows.length === 0) return;
    const r = oneBlock(t);
    if (!r) return;
    let width = widest(rows);
    // A block pasted into a bigger selection that it fits a whole number of
    // times repeats to fill it (as in Excel and Google Sheets).
    // Hidden columns are passed over, so only the shown ones count.
    const shownCols = shownColsIn(t, r);
    const across = shownCols % width === 0 ? shownCols / width : 1;
    const down = r.rows % rows.length === 0 ? r.rows / rows.length : 1;
    if ((across > 1 || down > 1) && (rows.length > 1 || width > 1) && cellCount(r) <= MAX_CELLS_PER_CHANGE) {
      const line = (x: string[]): string[] => Array.from({ length: width }, (_, i) => x[i] ?? "");
      const block = rows.map((x) => Array.from({ length: across }, () => line(x)).flat());
      rows = Array.from({ length: down }, () => block).flat();
      width *= across;
      text = toTsv(rows);
    }
    if (rows.length === 1 && width === 1 && cellCount(r) > 1) {
      await commitCell(r.row, r.col, rows[0]![0] ?? "", true, "pasting");
      if (afterPaste) say(afterPaste);
      afterPaste = null;
      return;
    }
    // The engine parses the text again and writes it as one undo step,
    // adding rows past the end if needed (asked first while rows are hidden).
    const fit = await askOverflow(t, r.row, rows.length);
    if (!fit) {
      afterPaste = null;
      say("Nothing was pasted.");
      return;
    }
    const rowsBefore = t.info.rows;
    const done = await runOp({ op: "paste", spec: { row: r.row, col: r.col, op: "none", skip_blanks: false, transpose: false, stop_at_end: fit === "stop" }, text }, "Pasting");
    if (done) {
      const shown = Math.min(rows.length, rowsBefore - r.row);
      const added = fit === "add" ? rows.length - shown : 0;
      if (fit === "add" && added > 0) {
        // The new rows at the end, selected so they are never lost from view.
        const last = t.info.rows - 1;
        grid?.select({ row: last - added + 1, col: r.col }, { row: last, col: r.col + width - 1 });
        say(`Pasted ${plural(shown, "row")} into the shown rows and added ${plural(added, "new row")} at the end of the file.${hiddenNote(done)} Ctrl+Z undoes it.`);
      } else {
        grid?.select({ row: r.row, col: r.col }, { row: r.row + Math.min(rows.length, Math.max(1, shown)) - 1, col: r.col + width - 1 });
        if (fit === "stop") say(`Pasted ${plural(shown, "row")}; the last ${plural(rows.length - shown, "row")} of what you copied ${rows.length - shown === 1 ? "was" : "were"} left out.${hiddenNote(done)} Ctrl+Z undoes it.`);
        else if (rowsHidden(t) && (done.hidden_rows ?? 0) > 0) say(`Pasted ${plural(rows.length, "row")} into the shown rows only.${hiddenNote(done)} Ctrl+Z undoes it.`);
      }
      t.noteStepSel(t.sel);
    }
    if (done && afterPaste) say(afterPaste);
    afterPaste = null;
    // One line that went into one cell but holds the file's delimiter (or
    // commas): offer to split it across columns (as one more undo step).
    if (done && rows.length === 1 && width === 1) void offerSplit(t, r.row, r.col, text.replace(/\r?\n$/, ""));
  }

  /** After a one-cell paste of a delimited line: a "Split into columns"
   *  button in the status bar. Offered for 3 or more parts, or when the
   *  cells it would fill are blank, so an address ("Dayton, OH") pasted
   *  next to other data doesn't bring it up every time. */
  async function offerSplit(t: Tab, row: number, col: number, line: string): Promise<void> {
    const sep = t.info.delimiter && t.info.delimiter !== "\t" && line.includes(t.info.delimiter) ? t.info.delimiter : line.includes(",") ? "," : null;
    if (!sep) return;
    const parts = namesFromText(line, sep);
    if (parts.length < 2) return;
    /** The cells to the right that the parts would fill and are not blank. */
    const filled = async (): Promise<number> => {
      const n = Math.min(parts.length - 1, Math.max(0, t.info.cols - col - 1));
      if (!n) return 0;
      const v = await backend().viewport(t.id, row, 1, col + 1, n, 1);
      return v.cells.filter((c) => c !== "").length;
    };
    try {
      if (parts.length < 3 && (await filled()) > 0) return;
    } catch {
      return;
    }
    if (tab !== t) return;
    say(`Pasted into one cell (${parts.length} parts split by ${sep === "," ? "commas" : `"${sep}"`}).`);
    messageAction = {
      label: "Split into columns",
      run: () =>
        void (async () => {
          if (tab !== t) return;
          const at = col;
          let inserted = false;
          // Cells to the right with something in them: ask first.
          const busy = await filled().catch(() => 0);
          if (busy > 0) {
            const a = await ask(
              `Replace what is in ${busy === 1 ? "1 cell" : `${busy} cells`} to the right?`,
              `Splitting puts ${parts.length} parts in columns ${col + 1} to ${col + parts.length} of this row, over what is there now. Insert ${parts.length - 1 === 1 ? "a new column" : `${parts.length - 1} new columns`} instead to keep it (${parts.length - 1 === 1 ? "it is" : "they are"} added to every row, empty in the others).`,
              [
                { id: "insert", label: `Insert ${parts.length - 1 === 1 ? "1 column" : `${parts.length - 1} columns`}`, primary: true },
                { id: "replace", label: "Replace", danger: true },
                { id: "cancel", label: "Cancel" },
              ],
              "cancel",
            );
            if (a !== "insert" && a !== "replace") return;
            if (a === "insert" && !(await change({ op: "insert_cols", at: col + 1, count: parts.length - 1, unrecorded: true }))) return;
            inserted = a === "insert";
          }
          t.whatNext = "splitting the pasted line";
          if (await apply((x) => backend().setCells(x.id, parts.map((v, i) => [row, at + i, v] as [number, number, string])))) {
            grid?.select({ row, col: at }, { row, col: at + parts.length - 1 });
            say(`Split into ${parts.length} columns. ${inserted ? "Ctrl+Z takes the split back; press it again to remove the new columns." : "Ctrl+Z undoes it."}`);
          }
        })(),
    };
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
    const r0 = oneBlock(t);
    if (!r0) return;
    const r = { ...r0 };
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
    const r0 = oneBlock(t);
    if (!r0) return;
    const r = { ...r0 };
    const spec: FillSpec = { row: r.row, col: r.col, rows: r.rows, cols: r.cols, dir, count, mode: { kind: swap ? "auto_swap" : "auto" } };
    const rep = await runOp({ op: "fill", spec }, "Filling");
    if (!rep) return;
    const last = { row: r.row + r.rows - 1, col: r.col + r.cols - 1 };
    if (dir === "down") grid?.select({ row: r.row, col: r.col }, { row: last.row + count, col: last.col }, false);
    else if (dir === "up") grid?.select({ row: last.row, col: last.col }, { row: Math.max(t.firstRow, r.row - count), col: r.col }, false);
    else if (dir === "right") grid?.select({ row: r.row, col: r.col }, { row: last.row, col: last.col + count }, false);
    else grid?.select({ row: last.row, col: last.col }, { row: r.row, col: Math.max(0, r.col - count) }, false);
    // Undo selects what the fill made.
    t.noteStepSel(t.sel);
  }

  /** Double-click on the fill handle: fill down as far as the column next
   *  to the selection has data. */
  async function fillHandleAuto(): Promise<void> {
    const t = tab;
    if (!t || !ready()) return;
    const r = oneBlock(t);
    if (!r) return;
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
  /** The hidden columns of a tab (paste, fill, clear and copy pass over them). */
  function hiddenColsOf(t: Tab): number[] {
    return t.columns.hiddenList();
  }

  /** How many columns of a block are shown (not hidden). */
  function shownColsIn(t: Tab, r: Rect): number {
    const hidden = hiddenColsOf(t);
    if (!hidden.length) return r.cols;
    const h = new Set(hidden);
    let n = 0;
    for (let c = r.col; c < r.col + r.cols; c++) if (!h.has(c)) n++;
    return n;
  }

  /** The status note for a small fill or clear of a block that hidden rows
   *  run through ("" when none do). */
  async function hiddenRowsNote(t: Tab, r: Rect): Promise<string> {
    if (!rowsHidden(t) || r.rows < 2) return "";
    try {
      const n = await backend().hiddenBetween(t.id, r.row, r.rows);
      return hiddenNote({ hidden_rows: n } as OpReport);
    } catch {
      return "";
    }
  }

  /** Rows are hidden (a filter, or by hand). */
  const rowsHidden = (t: Tab): boolean => t.info.filtered || (t.info.hidden_rows ?? 0) > 0;

  /** A paste taller than the shown rows left while rows are hidden: ask
   *  first. "stop" fills the shown rows and leaves the rest out, "add" puts
   *  the rest in new rows at the end, null is Cancel. */
  async function askOverflow(t: Tab, row: number, height: number, into?: string): Promise<"stop" | "add" | "fits" | null> {
    const left = t.info.rows - row;
    if (!rowsHidden(t) || height <= left) return "fits";
    const extra = height - left;
    const hidden = Math.max(0, t.info.total_rows - t.info.rows);
    const a = await ask(
      `You're pasting ${plural(height, "row")}${into ? ` into ${into}` : ""}, but only ${plural(left, "row")} ${left === 1 ? "is" : "are"} showing from here down`,
      `${t.filter ? "A filter is on" : "Some rows are hidden"}, so Natendo pastes into the rows you can see and leaves the ${plural(hidden, "hidden row")} alone.`,
      [
        { id: "stop", label: `Fill the ${plural(left, "shown row")} and stop`, primary: true },
        { id: "add", label: `Fill them, then add the other ${plural(extra, "row")} at the end of the file` },
        { id: "cancel", label: "Cancel" },
      ],
      "cancel",
      `"Fill and stop" leaves out the last ${plural(extra, "row")} of what you copied (they stay on the clipboard). New rows at the end always show${t.filter ? ", even if they don't match the filter" : ""}.`,
    );
    return a === "stop" || a === "add" ? a : null;
  }

  /** The status line after a paste, fill or clear that passed over hidden rows. */
  function hiddenNote(rep: OpReport | null): string {
    const n = rep?.hidden_rows ?? 0;
    return n > 0 ? ` ${plural(n, "hidden row")} between them ${n === 1 ? "was" : "were"} left as ${n === 1 ? "it was" : "they were"}.` : "";
  }

  async function runOp(op: DataOp, label: string, of?: Tab): Promise<OpReport | null> {
    const t = of ?? tab;
    if (!t) return null;
    if (writeBlocked(t)) return null;
    // What can't be seen isn't touched: hidden columns are passed over.
    const hide = hiddenColsOf(t);
    if (hide.length && (op.op === "paste" || op.op === "fill" || op.op === "fill_rect")) op.spec.skip_cols = hide;
    // Counted before the run: what is said afterwards is what the dialog said.
    const about = hiddenNoteFor(op);
    try {
      t.whatNext = label.toLowerCase();
      t.wideNext = true;
      const rep = await runJob(label, () => backend().dataOp(t.id, op));
      if (!rep) return null;
      hiddenAfter = about;
      if (rep.info) {
        t.update(rep.info, true);
        noteProtected(rep.info);
      }
      return rep;
    } catch (e) {
      say(errorText(e));
      return null;
    } finally {
      t.whatNext = null;
      t.wideNext = false;
    }
  }

  /** The words about hidden rows for one operation ("" when none are hidden or it only acts on shown rows). */
  function hiddenNoteFor(op: DataOp): string {
    const { hidden, total } = hiddenRows;
    if (op.op === "compute" && op.spec.target === null) {
      if (!hidden) return "";
      return op.spec.hidden === false ? hiddenWords("empty", hidden, total) : "Including hidden rows: they were filled too.";
    }
    const kind = kindOfOp(op.op);
    return kind ? hiddenWords(kind, hidden, total) : "";
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
    const col = t.cell.col;
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
    if (op.op === "paste") {
      // Paste Special: the same question as a paste when it is taller than
      // the shown rows left.
      const g = parseTsv(op.text);
      const height = op.spec.transpose ? Math.max(0, ...g.map((x) => x.length)) : g.length;
      const fit = await askOverflow(t, op.spec.row, height);
      if (!fit) {
        say("Nothing was pasted.");
        return;
      }
      op.spec.stop_at_end = fit === "stop";
    }
    const rep = await runOp(op, op.op === "fill" ? "Filling" : op.op === "paste" ? "Pasting" : "Working");
    if (!rep) return;
    switch (op.op) {
      case "paste":
        say(`Changed ${plural(rep.cells, "cell")}.${rep.skipped ? ` ${plural(rep.skipped, "cell")} left as they were (not numbers, or division by zero).` : ""}${hiddenNote(rep)}`);
        break;
      case "fill":
        say(`Filled ${plural(rep.cells, "cell")}.${hiddenNote(rep)}`);
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
        if (rep.cols) t.shiftCols(op.spec.col + 1, rep.cols);
        say(rep.cols ? `Split into ${plural(rep.cols, "new column")}.` : "No cell contains that separator.");
        break;
      case "merge_columns":
        t.shiftCols(op.spec.col + 1, -(op.spec.count - 1));
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
        if (op.op === "join" && rep.join?.cols && op.spec.place === "after_key") t.shiftCols(Math.max(...op.spec.keys) + 1, rep.join.cols);
        if (rep.join) {
          const other = op.other === null ? undefined : tabs.find((x) => x.id === op.other);
          say(`${joinSummary(op.op, rep.join, op.op === "join" ? op.spec : undefined)}${other && other !== t && noteLines(other) > 0 ? " The report notes of the other file are not included." : ""} Undo (Ctrl+Z) takes it back.`);
        }
        break;
      case "map_values":
        say(rep.cells ? `Changed ${plural(rep.cells, "cell")}. Undo (Ctrl+Z) takes it back.` : "No cell held those values.");
        break;
      case "group_calc":
        t.shiftCols(op.spec.at ?? t.info.cols - 1, 1);
        say(`Added a column with figures for ${plural(rep.groups, "group")}. Undo (Ctrl+Z) takes it back.`);
        break;
      case "extract":
        t.shiftCols(op.spec.col + 1, 1);
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
    const r = oneBlock(t);
    if (!r) return;
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
      const titles = await titlesOf(id, spec.keep_first, true);
      const d = await runJob("Building the pivot table", () => backend().pivotTable(id, spec));
      if (!d) return;
      addTab(d, false);
      const nt = tabs[tabs.length - 1];
      if (nt) {
        nt.header = true;
        nt.headerTouched = true;
        nt.bump();
        pivotOf.set(nt.id, { source: id, spec, titles });
      }
      say(`The pivot table has ${plural(d.rows, "row")} and ${plural(d.cols, "column")}. It is not saved yet (Ctrl+S). Data › Pivot table on it changes the layout.`);
    } catch (e) {
      say(errorText(e));
    }
  }

  /** Data › Refresh pivot table: build this pivot again from its source
   *  tab (after edits there), in the same place. */
  async function pivotRefresh(): Promise<void> {
    const t = tab;
    const from = t ? pivotOf.get(t.id) : undefined;
    if (!t || !from) return;
    if (!tabs.some((x) => x.id === from.source)) {
      say("The tab this pivot table came from is closed; open it again, then make the pivot table again.");
      return;
    }
    if (t.info.can_undo) {
      const a = await ask("Refresh the pivot table?", "Refreshing builds it again from its source and replaces this tab, so changes made in it are lost.", [
        { id: "go", label: "Refresh", primary: true },
        { id: "cancel", label: "Cancel" },
      ], "cancel");
      if (a !== "go") return;
    }
    try {
      const now = await titlesOf(from.source, from.spec.keep_first, true);
      const spec = followColumns(from.spec, from.titles, now);
      if (typeof spec === "string") {
        say(spec);
        return;
      }
      const d = await runJob("Refreshing the pivot table", () => backend().pivotTable(from.source, spec));
      if (!d) return;
      const nt = new Tab(d, DEFAULT_COL_WIDTH);
      nt.header = true;
      nt.headerTouched = true;
      nt.private = t.private;
      const i = tabs.indexOf(t);
      if (i < 0) return;
      tabs[i] = nt;
      active = i;
      pivotOf.delete(t.id);
      pivotOf.set(nt.id, { source: from.source, spec, titles: now });
      await backend().closeDocument(t.id).catch(() => {});
      nt.bump();
      say(`Refreshed from ${tabs.find((x) => x.id === from.source)?.info.name ?? "its source"}: ${plural(d.rows, "row")} and ${plural(d.cols, "column")}.${t.info.path ? ` The refreshed table is not saved yet: Ctrl+S, then pick ${t.info.name} to save over the old one.` : ""}`);
      void tick().then(() => grid?.focus());
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
    // Ctrl+arrow from the active cell; Ctrl+Shift+arrow moves the far corner.
    const f = extend ? t.sel.focus : t.cell;
    let target: [number, number];
    try {
      target = await backend().dataEdge(t.id, f.row, f.col, dir);
    } catch {
      const last = Math.max(t.firstRow, t.info.rows - 1);
      target = dir === "up" ? [t.firstRow, f.col] : dir === "down" ? [last, f.col] : dir === "left" ? [f.row, 0] : [f.row, Math.max(0, t.info.cols - 1)];
    }
    if (t !== tab || !grid) return;
    // The engine does not know which columns are hidden: land on the
    // nearest shown one instead (as in Excel).
    let col = target[1];
    if (t.columns.width(col) === 0) {
      const back = dir === "left" ? 1 : -1;
      const near = t.columns.step(col, back);
      col = t.columns.width(near) > 0 ? near : t.columns.step(col, back === 1 ? -1 : 1);
    }
    const cell = { row: Math.max(t.firstRow, target[0]), col };
    if (extend) grid.select(t.sel.anchor, cell, true, t.sel.active);
    else grid.select(cell);
  }

  /** F8: arrow keys extend the selection until F8 or Esc. */
  let extendMode = $state(false);
  // Another tab, or a click anywhere, ends it (Excel ends it on a click too).
  $effect(() => {
    void active;
    extendMode = false;
  });
  function toggleExtend(): void {
    extendMode = !extendMode;
    say(extendMode ? "Extend mode: the arrow keys grow the selection. F8 or Esc ends it." : "Extend mode off.");
  }

  /** The block of data around the active cell, bounded by blank rows and
   *  columns (Excel's current region). */
  async function regionOf(t: Tab): Promise<Rect | null> {
    const c = t.cell;
    try {
      // One engine call: it looks at the file itself, all columns.
      const g = await backend().currentRegion(t.id, c.row, c.col, t.firstRow);
      if (!g) return null;
      const row = Math.max(t.firstRow, g[0]);
      return { row, rows: g[1] - row + 1, col: g[2], cols: g[3] - g[2] + 1 };
    } catch {
      return null;
    }
  }

  /** Ctrl+A: the block of data around the cell first, everything on the
   *  second press (as in Excel). */
  async function selectAll(): Promise<void> {
    const t = tab;
    if (!t || !grid) return;
    const all = () => grid?.select({ row: t.firstRow, col: 0 }, { row: t.info.rows - 1, col: t.info.cols - 1 }, false, t.cell);
    const r = await regionOf(t);
    const cur = t.singleRect();
    const whole = !r || (r.row <= t.firstRow && r.rows >= t.info.rows - t.firstRow && r.col === 0 && r.cols >= t.info.cols);
    const same = r && cur && cur.row === r.row && cur.rows === r.rows && cur.col === r.col && cur.cols === r.cols;
    if (whole || same || t !== tab) return void all();
    grid.select({ row: r.row, col: r.col }, { row: r.row + r.rows - 1, col: r.col + r.cols - 1 }, false, t.cell);
    say("Selected the block of data around the cell. Ctrl+A again selects everything.");
  }

  async function selectRegion(): Promise<void> {
    const t = tab;
    if (!t || !grid) return;
    const r = await regionOf(t);
    if (!r || t !== tab) return;
    grid.select({ row: r.row, col: r.col }, { row: r.row + r.rows - 1, col: r.col + r.cols - 1 }, false, t.cell);
  }

  /** Shift+F10 or the Menu key: the right-click menu at the active cell. */
  /** Until when a context menu event comes from Shift+F10 or the Menu key. */
  let ctxKeysUntil = 0;
  function contextFromKeys(): void {
    const t = tab;
    if (!t || !grid) return;
    ctxKeysUntil = performance.now() + 400;
    // With several blocks the menu opens on the active cell as for a cell.
    const r = t.singleRect();
    const area = r && wholeRows(r, t) && !wholeCols(r, t) ? "row" : r && wholeCols(r, t) && !wholeRows(r, t) ? "col" : "cell";
    grid.scrollIntoView(t.cell);
    const p = grid.cellPoint(t.cell);
    openContext(p.x, p.y, area, t.cell.col);
    // From the keyboard the first item is picked, ready for the arrows.
    if (context) context.index = 0;
  }

  /** Ctrl+': edit the cell, starting with the value of the cell above
   *  (read in full: the grid may hold only the start of a long value). */
  async function copyAbove(): Promise<void> {
    const t = tab;
    if (!t || !grid || lockedSay(t)) return;
    const { row, col } = t.cell;
    if (row <= t.firstRow) return;
    let v = t.cache.get(row - 1, col);
    if (v === undefined || t.cache.isTruncated(row - 1, col)) {
      try {
        const full = await backend().cellText(t.id, row - 1, col);
        if (full.truncated) {
          say("The cell above is too long to copy into an edit (over 1,048,576 characters).");
          return;
        }
        v = full.text;
      } catch (e) {
        say(errorText(e));
        return;
      }
    }
    if (t !== tab) return;
    void grid?.startEdit(v);
  }

  /** A locked file: say so (true) instead of starting a change. */
  function lockedSay(t: Tab): boolean {
    if (!t.info.read_only) return false;
    say("This file is locked, so cells can't change. Click Locked in the status bar to unlock it.");
    return true;
  }

  /** A row's number as shown on screen (the whole file's while filtered),
   *  or null when not known yet. */
  function shownRowNo(t: Tab, row: number): number | null {
    const n = t.info.filtered ? t.cache.rowNumber(row) : row;
    return n === undefined ? null : n + 1;
  }

  /** Ctrl+.: the active cell goes round the selection's corners clockwise. */
  function nextCorner(): void {
    const t = tab;
    if (!t || !grid) return;
    const r = oneBlock(t);
    if (!r) return;
    const r1 = r.row + r.rows - 1;
    const c1 = r.col + r.cols - 1;
    const corners = [
      { row: r.row, col: r.col },
      { row: r.row, col: c1 },
      { row: r1, col: c1 },
      { row: r1, col: r.col },
    ];
    // A one-row or one-column selection has two corners, not four.
    const ring = corners.filter((k, i) => corners.findIndex((x) => x.row === k.row && x.col === k.col) === i);
    const at = ring.findIndex((k) => k.row === t.cell.row && k.col === t.cell.col);
    const next = ring[(at + 1) % ring.length]!;
    grid.select(t.sel.anchor, t.sel.focus, false, next);
    grid.scrollIntoView(next);
  }

  /** Ctrl+Shift+1, 3, 4, 5: a display format for the selected columns in one key (as in Google Sheets). */
  function quickFormat(kind: "number" | "date" | "currency" | "percent"): void {
    const t = tab;
    if (!t) return;
    setFormat(selectedCols(t), defaultFormat(kind));
  }

  function openSymbols(): void {
    symbols = { caret: grid?.editorCaret() ?? null };
  }

  /** How Undo names a change made with `change`. */
  const CHANGE_WHAT: Partial<Record<Change["op"], string>> = {
    insert_rows: "inserting rows",
    delete_rows: "deleting rows",
    move_rows: "moving rows",
    insert_cols: "inserting columns",
    delete_cols: "deleting columns",
    move_cols: "moving columns",
    hide_rows: "hiding rows",
    show_rows: "showing rows",
    shuffle_rows: "shuffling rows",
    flip_rows: "flipping rows",
    delete_col_list: "deleting columns",
  };

  /** How custom row heights must follow a change to the rows (null: not
   *  needed). Shown rows are the file's rows unless some are hidden or filtered
   *  out; then the heights cannot be followed exactly, so they are cleared. */
  function rowMapFor(t: Tab, c: Change): RowMap | null {
    if (t.rowHeights.empty) return null;
    if (c.op !== "insert_rows" && c.op !== "delete_rows" && c.op !== "move_rows") return null;
    if (t.info.filtered || t.info.rows !== t.info.total_rows) return { kind: "reset" };
    if (c.op === "insert_rows") return { kind: "insert", at: c.at, count: c.count };
    if (c.op === "delete_rows") return { kind: "delete", at: c.at, count: c.count };
    return { kind: "move", at: c.at, count: c.count, to: c.to };
  }

  async function change(c: Change, dataChanged = true): Promise<boolean> {
    const t = tab;
    if (t && c.op !== "undo" && c.op !== "redo") t.whatNext = CHANGE_WHAT[c.op] ?? null;
    if (t) t.pendingRows = rowMapFor(t, c);
    const ok = await apply((x) => backend().change(x.id, c), dataChanged);
    if (t) t.pendingRows = null;
    return ok;
  }

  async function insertRows(below: boolean, howMany?: number): Promise<void> {
    const t = tab;
    if (!t) return;
    const r = oneBlock(t);
    if (!r) return;
    const at = below ? r.row + r.rows : r.row;
    const count = howMany ?? r.rows;
    if (await change({ op: "insert_rows", at, count })) {
      grid?.select({ row: at, col: r.col }, { row: at + count - 1, col: r.col + r.cols - 1 });
    }
  }

  async function deleteRows(): Promise<void> {
    const t = tab;
    if (!t) return;
    const r = oneBlock(t);
    if (!r) return;
    const count = Math.min(r.rows, t.info.rows - r.row);
    if (count <= 0) return;
    if (await change({ op: "delete_rows", at: r.row, count })) grid?.select({ row: r.row, col: r.col });
  }

  async function insertCols(right: boolean, howMany?: number): Promise<void> {
    const t = tab;
    if (!t) return;
    const r = oneBlock(t);
    if (!r) return;
    const at = right ? r.col + r.cols : r.col;
    const count = howMany ?? r.cols;
    if (await change({ op: "insert_cols", at, count })) {
      t.shiftCols(at, count);
      grid?.select({ row: r.row, col: at }, { row: r.row + r.rows - 1, col: at + count - 1 });
    }
  }

  async function deleteCols(): Promise<void> {
    const t = tab;
    if (!t) return;
    const r = oneBlock(t);
    if (!r) return;
    if (r.cols >= t.info.cols) {
      say("At least one column must remain.");
      return;
    }
    // Protection is there to stop accidents with a key column; deleting it
    // is the biggest one, so it asks.
    const guarded = (t.info.protected ?? []).filter((c) => c >= r.col && c < r.col + r.cols);
    if (guarded.length) {
      const names = guarded.slice(0, 3).map((c) => `“${columnTitle(t, c)}”`).join(", ") + (guarded.length > 3 ? ` and ${guarded.length - 3} more` : "");
      const a = await ask(
        `Delete ${guarded.length === 1 ? "a protected column" : "protected columns"}?`,
        `${names} ${guarded.length === 1 ? "is" : "are"} protected. Delete ${guarded.length === 1 ? "it" : "them"} anyway? Undo (Ctrl+Z) brings ${guarded.length === 1 ? "it" : "them"} back.`,
        [
          { id: "delete", label: "Delete anyway" },
          { id: "no", label: "Keep", primary: true },
        ],
        "no",
      );
      if (a !== "delete") return;
    }
    if (await change({ op: "delete_cols", at: r.col, count: r.cols })) {
      const dropped = t.shiftCols(r.col, -r.cols);
      if (dropped) say(`${dropped === 1 ? "1 data check rule" : `${dropped} data check rules`} used a deleted column and ${dropped === 1 ? "was" : "were"} removed.`);
      grid?.select({ row: r.row, col: Math.min(r.col, t.info.cols - 1) });
      if (t.info.filter_on_deleted) {
        say("The deleted column had a filter, so some rows stay hidden. Undo brings the column back, or click Filtered ✕ to show every row.");
      }
    }
  }

  /** Excel's Ctrl++ / Ctrl+-: whole columns selected act on columns, otherwise rows. */
  /** Ctrl+Plus: whole columns or rows go in at once; part of a row asks
   *  how (shift cells right or down, or entire rows or columns), as in Excel. */
  async function insertSmart(): Promise<void> {
    const t = tab;
    if (!t) return;
    const r = oneBlock(t);
    if (!r) return;
    if (wholeCols(r, t) && !wholeRows(r, t)) await insertCols(false);
    else if (wholeRows(r, t)) await insertRows(false);
    else shiftAsk = { insert: true };
  }
  /** Ctrl+Minus: the same for deleting (shift cells left or up). */
  async function deleteSmart(): Promise<void> {
    const t = tab;
    if (!t) return;
    const r = oneBlock(t);
    if (!r) return;
    if (wholeCols(r, t) && !wholeRows(r, t)) await deleteCols();
    else if (wholeRows(r, t)) await deleteRows();
    else shiftAsk = { insert: false };
  }
  let shiftAsk = $state<{ insert: boolean } | null>(null);

  // ---- compare two columns ----
  let compareAsk = $state<{ titles: string[]; first: number; second: number } | null>(null);

  function openCompareCols(): void {
    const t = tab;
    if (!t) return;
    if (t.info.cols < 2) {
      say("Comparing needs two columns.");
      return;
    }
    const titles = Array.from({ length: Math.min(t.info.cols, 5_000) }, (_, c) => columnTitle(t, c));
    const r = oneBlock(t);
    if (!r) return;
    // Two selected columns, or the active one and the next.
    const first = r.cols === 2 ? r.col : t.cell.col;
    const second = r.cols === 2 ? r.col + 1 : first + 1 < t.info.cols ? first + 1 : Math.max(0, first - 1);
    compareAsk = { titles, first: Math.min(first, titles.length - 1), second: Math.min(second, titles.length - 1) };
  }

  async function runCompareCols(spec: CompareCols, action: CompareAction): Promise<void> {
    compareAsk = null;
    const t = tab;
    if (!t) return;
    const name = (c: number) => `“${columnTitle(t, c)}”`;
    if (action === "column") {
      const rep = await runOp({ op: "compare_column", spec }, "Comparing");
      if (rep) say(`Added a column comparing ${name(spec.a)} with ${name(spec.b)} (Same, Different, Only in first, Only in second). Ctrl+Z undoes it.`);
      return;
    }
    if (action === "show") {
      const rep = await runOp({ op: "compare_show", spec }, "Comparing");
      if (rep) say(rep.rows ? `Showing ${plural(rep.rows, "row")} where ${name(spec.a)} and ${name(spec.b)} differ. Data › Clear filter shows every row.` : `${name(spec.a)} and ${name(spec.b)} are the same in every shown row.`);
      return;
    }
    try {
      const rep = await runJob("Comparing", () => backend().compareCols(t.id, spec));
      if (!rep || t !== tab) return;
      const differ = rep.different + rep.only_first + rep.only_second;
      if (!differ) {
        t.picked = new Set();
        t.bump();
        say(`${name(spec.a)} and ${name(spec.b)} are the same in all ${plural(rep.same, "shown row")}.`);
        return;
      }
      const lit = new Set<string>();
      for (const r of rep.rows) {
        if (lit.size >= MAX_LIT) break;
        lit.add(`${r}:${spec.a}`);
        lit.add(`${r}:${spec.b}`);
      }
      t.picked = lit;
      const r0 = rep.rows[0]!;
      grid?.select({ row: r0, col: spec.a }, { row: r0, col: spec.a });
      t.bump();
      const parts = [rep.different ? `${rep.different.toLocaleString()} different` : "", rep.only_first ? `${rep.only_first.toLocaleString()} only in ${name(spec.a)}` : "", rep.only_second ? `${rep.only_second.toLocaleString()} only in ${name(spec.b)}` : ""].filter(Boolean);
      say(`${plural(differ, "row")} differ (${parts.join(", ")}); ${rep.same.toLocaleString()} the same. They are lit up${lit.size >= MAX_LIT ? " (the first ones)" : ""}; any change turns the lights off.`);
    } catch (e) {
      say(errorText(e));
    }
  }

  // ---- compare lists ----
  let setsAsk = $state<{ tabs: SetsTab[]; current: number } | null>(null);

  function openSets(): void {
    const t = tab;
    if (!t) return;
    const list = tabs.map((x) => ({
      id: x.id,
      name: x.info.name,
      titles: Array.from({ length: Math.min(x.info.cols, 5_000) }, (_, c) => columnTitle(x, c)),
      hidden: hiddenColsOf(x),
      header: x.header && x.info.rows > 0,
    }));
    setsAsk = { tabs: list, current: t.id };
  }

  async function runSets(ids: number[], spec: SetsSpec): Promise<SetsRun | null> {
    const t = tab;
    if (!t) return null;
    try {
      return await runJob("Comparing lists", () => backend().setsRun(t.id, ids, spec));
    } catch (e) {
      say(errorText(e));
      return null;
    }
  }

  /** A slice or the counts in a new tab; false when it was refused. */
  async function openSetsTab(make: () => Promise<DocInfo>, header: boolean, id: number | null, saveIt = false): Promise<boolean> {
    try {
      const d = await runJob("Opening", make);
      if (!d) return true;
      setsAsk = null;
      if (id !== null) void backend().setsClose(id).catch(() => {});
      addTab(d, false);
      const nt = tabs[tabs.length - 1];
      if (nt) {
        nt.header = header;
        nt.headerTouched = true;
        nt.bump();
      }
      if (saveIt) {
        await tick();
        await save(true);
      } else say(`Opened ${plural(Math.max(0, d.rows - (header ? 1 : 0)), "row")} in a new tab.`);
      void tick().then(() => grid?.focus());
      return true;
    } catch (e) {
      say(errorText(e));
      return false;
    }
  }

  async function copySets(text: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
      say("Copied the counts.");
    } catch {
      say("Could not reach the clipboard.");
    }
  }

  // ---- patch files ----
  type PatchSave = NonNullable<import("svelte").ComponentProps<typeof PatchDialog>["save"]>;
  let patchAsk = $state<{ save: PatchSave | null; apply: { doc: number; name: string } | null } | null>(null);

  /** Save the differences of the comparison the current tab belongs to. */
  async function openPatchSave(): Promise<void> {
    const f = tab?.patchFrom;
    if (!f) {
      say("First compare two tables by key (Data > Compare files).");
      return;
    }
    const a = tabs.find((t) => t.id === f.a);
    const b = tabs.find((t) => t.id === f.b);
    if (!a || !b) {
      say("A compared tab was closed: compare the files again.");
      return;
    }
    const secret = (await sensitive(a)) || (await sensitive(b));
    patchAsk = {
      apply: null,
      save: {
        a: a.id,
        b: b.id,
        aName: a.info.name,
        bName: b.info.name,
        keys: f.keys.map((c) => columnTitle(a, c)),
        spec: { keys: [...f.keys], match_case: f.match_case, trim: f.trim, leave_out: false },
        sensitive: secret,
      },
    };
  }

  function openPatchApply(): void {
    const t = tab;
    if (!t || writeBlocked(t)) return;
    patchAsk = { save: null, apply: { doc: t.id, name: t.info.name } };
  }

  /** Apply the dry run; a refusal is returned in words. */
  async function applyPatch(): Promise<string> {
    const doc = patchAsk?.apply?.doc;
    const t = tab;
    if (doc === undefined || !t || t.id !== doc) return "Open the table again and choose the patch.";
    try {
      let done: import("./lib/patch").PatchApplied | null = null;
      const ok = await applyJob("Applying the patch", async (x) => {
        const r = await backend().patchApply(x.id);
        done = r;
        return r.info;
      });
      if (!ok || !done) return "";
      t.private = await backend().privateState(t.id);
      closePatch();
      say(appliedText(done));
      return "";
    } catch (e) {
      return errorText(e);
    }
  }

  function closePatch(): void {
    const doc = patchAsk?.apply?.doc;
    patchAsk = null;
    if (doc !== undefined) void backend().patchClose(doc).catch(() => {});
    grid?.focus();
  }

  // ---- golden records ----
  /** Open the new tables (the result in front) and close the match. */
  function goldenMade(made: import("./lib/golden").GoldenMade, summary: import("./lib/golden").GoldenSummary, away: boolean): void {
    const from = tab;
    matchOpen = false;
    if (from) {
      void backend().goldenClose(from.id).catch(() => {});
      void backend().matchClose(from.id);
    }
    for (const d of [made.away, made.info]) {
      if (!d) continue;
      addTab(d, false);
      const nt = tabs[tabs.length - 1];
      if (nt && from) {
        nt.header = from.header;
        nt.headerTouched = true;
        if (from.private.file || from.private.folder) nt.private = from.private;
        nt.bump();
      }
    }
    say(madeText(summary.counts, away));
  }

  // ---- relationships between columns ----
  let relationsAsk = $state<{ titles: string[] } | null>(null);

  function openRelations(): void {
    const t = tab;
    if (!t) return;
    if (t.info.cols < 2) {
      say("Finding relationships needs two columns.");
      return;
    }
    relationsAsk = { titles: Array.from({ length: Math.min(t.info.cols, 5_000) }, (_, c) => columnTitle(t, c)) };
  }

  async function runRelations(spec: RelationsSpec): Promise<RelationsReport | null> {
    const t = tab;
    if (!t) return null;
    try {
      return await runJob("Looking for relationships", () => backend().findRelations(t.id, spec));
    } catch (e) {
      say(errorText(e));
      return null;
    }
  }

  async function showExceptions(f: Finding): Promise<boolean> {
    const rep = await runOp({ op: "show_relation_exceptions", kind: f.kind, a: f.a, b: f.b }, "Filtering");
    if (!rep) return false;
    relationsAsk = null;
    say(`Showing the ${plural(rep.rows, "row")} that break it. Data › Clear filter shows every row.`);
    void tick().then(() => grid?.focus());
    return true;
  }

  async function copyRelations(text: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
      say("Copied the relationships.");
    } catch {
      say("Could not reach the clipboard.");
    }
  }

  // ---- clean matrix ----
  let matrixAsk = $state<{ titles: string[]; picked: number[] } | null>(null);

  function openMatrix(): void {
    const t = tab;
    if (!t) return;
    const titles = Array.from({ length: Math.min(t.info.cols, 5_000) }, (_, c) => columnTitle(t, c));
    matrixAsk = { titles, picked: [] };
  }

  /** From a right-click: the selected cells' columns start ticked. */
  function openMatrixHere(): void {
    const t = tab;
    if (!t) return;
    openMatrix();
    const r = t.singleRect();
    if (!matrixAsk || !r) return;
    const hidden = new Set(hiddenColsOf(t));
    const kept = new Set(t.info.protected ?? []);
    matrixAsk.picked = Array.from({ length: Math.min(r.cols, 5_000) }, (_, i) => r.col + i).filter((c) => c < matrixAsk!.titles.length && !hidden.has(c) && !kept.has(c));
  }

  async function previewMatrix(spec: CleanMatrix): Promise<MatrixPreview | null> {
    const t = tab;
    if (!t) return null;
    try {
      return await runJob("Cleaning preview", () => backend().cleanMatrixPreview(t.id, spec, localNow()));
    } catch (e) {
      say(errorText(e));
      return null;
    }
  }

  async function applyMatrix(spec: CleanMatrix, revision: number): Promise<void> {
    matrixAsk = null;
    const rep = await runOp({ op: "clean_matrix", spec, now: localNow(), revision }, "Cleaning");
    if (!rep) return;
    const left = rep.left ? ` ${plural(rep.left, "cell")} could not be read and ${rep.left === 1 ? "was" : "were"} left alone.` : "";
    say(rep.cells ? `Cleaned ${plural(rep.cells, "cell")} in ${plural(spec.entries.length, "column")}. Ctrl+Z undoes it.${left}` : `Nothing needed cleaning.${left}`);
  }

  // ---- gaps in a sequence ----
  let gapsAsk = $state<{ titles: string[]; col: number } | null>(null);

  function openGaps(): void {
    const t = tab;
    if (!t) return;
    const titles = Array.from({ length: Math.min(t.info.cols, 5_000) }, (_, c) => columnTitle(t, c));
    gapsAsk = { titles, col: Math.min(t.cell.col, titles.length - 1) };
  }

  async function runGaps(spec: GapsSpec): Promise<GapReport | null> {
    const t = tab;
    if (!t) return null;
    try {
      return await runJob("Looking for gaps", () => backend().gapAudit(t.id, spec));
    } catch (e) {
      say(errorText(e));
      return null;
    }
  }

  async function showGapRows(spec: GapsSpec): Promise<void> {
    gapsAsk = null;
    const rep = await runOp({ op: "show_gap_rows", spec }, "Filtering");
    if (rep) say(`Showing ${plural(rep.rows, "row")} next to a gap. Data › Clear filter shows every row.`);
  }

  async function openGapsTab(spec: GapsSpec, saveIt: boolean): Promise<void> {
    gapsAsk = null;
    const t = tab;
    if (!t) return;
    try {
      const d = await runJob("Looking for gaps", () => backend().newTable(t.id, { op: "gaps", spec }));
      if (!d) return;
      addTab(d, false);
      if (saveIt) {
        await tick();
        await save(true);
      } else say("The summary and the missing ranges are in a new tab.");
    } catch (e) {
      say(errorText(e));
    }
  }

  async function copyGaps(text: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
      say("Copied the missing ranges.");
    } catch {
      say("Could not reach the clipboard.");
    }
  }

  // ---- tidy column titles ----
  let tidyAsk = $state(false);
  let lastTidy = $state<TitleSpec | null>(null);

  function openTidyTitles(): void {
    const t = tab;
    if (!t) return;
    if (!t.header || t.info.rows === 0) {
      say("This table has no title row: switch on Format › First row holds titles (or Rename column adds one), then tidy the titles.");
      return;
    }
    tidyAsk = true;
  }

  async function runTidyTitles(spec: TitleSpec): Promise<void> {
    tidyAsk = false;
    lastTidy = spec;
    const t = tab;
    if (!t) return;
    let before: string[] = [];
    try {
      before = await backend().firstRowNames(t.id);
    } catch {
      before = [];
    }
    t.whatNext = "tidying column titles";
    if (!(await apply((x) => backend().tidyTitles(x.id, spec)))) return;
    let after: string[] = [];
    try {
      after = await backend().firstRowNames(t.id);
    } catch {
      after = [];
    }
    const changed = after.filter((s, i) => s !== before[i]).length;
    // Row rules in data checks follow the new names (undone with them).
    let followed = "";
    if (before.length && after.length === before.length && t.checks.some((k) => k.kind.type === "expression")) {
      const was = $state.snapshot(t.checks) as Check[];
      const r = remapChecks(was, before, after, (k) => k);
      if (r.changed) {
        t.checks = r.checks;
        t.checkVersion++;
        t.refEdits = [...t.refEdits.filter((e) => e.applied), { at: t.info.history_pos, before: was, after: r.checks, applied: true }].slice(-50);
        followed = ` ${r.changed === 1 ? "1 data check rule follows" : `${r.changed} data check rules follow`} the new names.`;
      }
    }
    say(`Tidied ${plural(changed, "column title")}.${followed} Ctrl+Z undoes it.`);
  }

  // ---- protected columns ----
  /** The columns Protect acts on: the right-clicked heading, or the
   *  selection's columns. */
  function protectTargets(t: Tab): number[] {
    const r = t.singleRect();
    if (!r) return [];
    if (contextCol !== null && (contextCol < r.col || contextCol >= r.col + r.cols)) return [contextCol];
    return Array.from({ length: Math.min(r.cols, t.info.cols) }, (_, i) => r.col + i);
  }

  async function toggleProtect(): Promise<void> {
    const t = tab;
    if (!t) return;
    const cols = protectTargets(t);
    const on = new Set(t.info.protected ?? []);
    const all = cols.every((c) => on.has(c));
    for (const c of cols) {
      if (all) on.delete(c);
      else on.add(c);
    }
    try {
      t.update(await backend().setProtected(t.id, [...on].sort((a, b) => a - b)), false);
      const names = cols.slice(0, 3).map((c) => `“${columnTitle(t, c)}”`).join(", ") + (cols.length > 3 ? ` and ${cols.length - 3} more` : "");
      say(all ? `${names} can be changed again.` : `${names} ${cols.length === 1 ? "is" : "are"} protected: typing, paste, fill, Replace and cleaning tools pass ${cols.length === 1 ? "it" : "them"} over. Sorting and filtering still work.`);
    } catch (e) {
      say(errorText(e));
    }
  }

  // ---- Go To Special ----
  let specialAsk = $state(false);
  let lastSpecial = $state<SpecialChoice | null>(null);
  /** Most cells Go To Special lights up at once. */
  const MAX_LIT = 100_000;

  /** The shown rows and columns Go To Special looks through. */
  function specialArea(t: Tab, inSelection: boolean): { row: number; rows: number; cols: number[] } {
    const hidden = new Set(hiddenColsOf(t));
    const first = t.firstRow;
    const block = t.singleRect();
    if (inSelection && block && cellCount(block) > 1) {
      const r = t.singleRect();
      if (!r) return { row: 0, rows: 0, cols: [] };
      const row = Math.max(r.row, first);
      const cols: number[] = [];
      for (let c = r.col; c < r.col + r.cols; c++) if (!hidden.has(c)) cols.push(c);
      return { row, rows: Math.max(0, r.row + r.rows - row), cols };
    }
    const cols: number[] = [];
    for (let c = 0; c < t.info.cols; c++) if (!hidden.has(c)) cols.push(c);
    return { row: first, rows: Math.max(0, t.info.rows - first), cols };
  }

  async function runSpecial(c: SpecialChoice): Promise<void> {
    const t = tab;
    if (!t) return;
    lastSpecial = c;
    const read = { decimal_comma: false, date_order: "mdy" as const };
    const kind: SpecialKind =
      c.pick === "row_diff"
        ? { kind: "row_diff", against: t.cell.col, exact: c.exact }
        : c.pick === "problems"
          ? { kind: "problems", checks: { checks: $state.snapshot(t.checks) as Check[], keep_first: t.header, read, now: localDays() } }
          : c.pick === "wrong_width"
            ? { kind: "wrong_width", header: t.header }
            : { kind: c.pick };
    const area = specialArea(t, c.inSelection);
    const spec: SpecialSpec = { kind, ...area, read };
    if (c.action === "light") {
      try {
        const res = await runJob("Looking", () => backend().specialCells(t.id, spec));
        if (!res || t !== tab) return;
        if (!res.total) {
          t.picked = new Set();
          t.bump();
          say("No cells like that here.");
          return;
        }
        t.picked = new Set(res.cells.slice(0, MAX_LIT).map(([r, col]) => `${r}:${col}`));
        const [r0, c0] = res.cells[0]!;
        grid?.select({ row: r0, col: c0 }, { row: r0, col: c0 });
        t.bump();
        const more = res.total > MAX_LIT ? ` The first ${MAX_LIT.toLocaleString()} are lit up.` : "";
        say(`${plural(res.total, "cell")} found and lit up.${more} Go to special again (Ctrl+Shift+G) to fill or clear them; any change turns the lights off.`);
      } catch (e) {
        say(errorText(e));
      }
      return;
    }
    const value = c.action === "fill" ? c.value : "";
    const rep = await runOp({ op: "fill_special", spec, value }, value ? "Filling" : "Clearing");
    if (!rep) return;
    say(rep.cells ? `${value ? "Filled" : "Cleared"} ${plural(rep.cells, "cell")}.${hiddenNote(rep)} Undo (Ctrl+Z) takes it back.` : "No cells like that here; nothing changed.");
  }

  /** What the Insert or Delete box chose. */
  async function runShift(insert: boolean, choice: ShiftChoice): Promise<void> {
    const t = tab;
    if (!t) return;
    const r = oneBlock(t);
    if (!r) return;
    if (choice === "rows") return insert ? insertRows(false) : deleteRows();
    if (choice === "cols") return insert ? insertCols(false) : deleteCols();
    const moved = choice === "up" || choice === "down" ? t.info.rows - r.row : r.rows;
    if ((choice === "up" || choice === "down") && moved > 1_000) {
      const a = await ask(
        `Shift cells in ${plural(moved, "row")}?`,
        `The cells below in ${r.cols === 1 ? "this column" : `these ${r.cols} columns`} move ${choice} in every row down to the end, so they no longer line up with the rest of each row. Undo (Ctrl+Z) takes it back.`,
        [
          { id: "go", label: `Shift cells ${choice}`, primary: true },
          { id: "rows", label: insert ? "Insert entire rows instead" : "Delete entire rows instead" },
          { id: "cancel", label: "Cancel" },
        ],
        "cancel",
      );
      if (a === "rows") return insert ? insertRows(false) : deleteRows();
      if (a !== "go") return;
    }
    const rep = await runOp({ op: "shift_cells", spec: { row: r.row, rows: r.rows, col: r.col, cols: r.cols, shift: choice } }, insert ? "Inserting cells" : "Deleting cells");
    if (!rep) return;
    const how = { left: "the cells to the right moved left", up: "the cells below moved up", right: "the cells moved right", down: "the cells moved down" }[choice];
    say(`${insert ? "Inserted" : "Deleted"} ${plural(cellCount(r), "cell")}; ${how}. Undo (Ctrl+Z) takes it back.`);
  }

  async function moveRows(dir: -1 | 1): Promise<void> {
    const t = tab;
    if (!t) return;
    const r = oneBlock(t);
    if (!r) return;
    const to = r.row + dir;
    if (to < t.firstRow || to + r.rows > t.info.rows) return;
    if (await change({ op: "move_rows", at: r.row, count: r.rows, to })) {
      const a = t.sel.active;
      grid?.select({ row: t.sel.anchor.row + dir, col: t.sel.anchor.col }, { row: t.sel.focus.row + dir, col: t.sel.focus.col }, true, a && { row: a.row + dir, col: a.col });
    }
  }

  async function moveCols(dir: -1 | 1): Promise<void> {
    const t = tab;
    if (!t) return;
    const r = oneBlock(t);
    if (!r) return;
    const to = r.col + dir;
    if (to < 0 || to + r.cols > t.info.cols) return;
    if (await change({ op: "move_cols", at: r.col, count: r.cols, to })) {
      // Carry the moved columns' widths, formats and wrapping with them.
      t.noteColMove(r.col, r.cols, to);
      const a = t.sel.active;
      grid?.select({ row: t.sel.anchor.row, col: t.sel.anchor.col + dir }, { row: t.sel.focus.row, col: t.sel.focus.col + dir }, true, a && { row: a.row, col: a.col + dir });
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
    t.whatNext = copy ? "copying cells" : "moving cells";
    if (await apply((x) => backend().moveCells(x.id, from.row, from.rows, from.col, from.cols, row, col, copy))) {
      grid?.select({ row, col }, { row: row + from.rows - 1, col: col + from.cols - 1 });
      t.noteStepSel(t.sel);
      const n = cellCount(from);
      say(`${copy ? "Copied" : "Moved"} ${n.toLocaleString()} cell${n === 1 ? "" : "s"}. Ctrl+Z undoes it.`);
    }
  }

  // ---- cells dropped on another tab (copy, Shift: move) ----------------------
  /** When each tab was last the front one (for listing the most recent first). */
  const usedAt = new Map<number, number>();
  let usedSeq = 0;
  $effect(() => {
    const t = tabs[active];
    if (t) usedAt.set(t.id, ++usedSeq);
  });

  /** The tab the cell drag is over (outlined in the bar), if any. */
  let cellDropTab = $state<number | null>(null);

  /** The tab under a point in the tab bar (not the active one: dropping on
   *  the tab the cells come from does nothing). */
  function tabUnder(x: number, y: number): Tab | null {
    const el = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-tab-index]");
    const i = el ? Number(el.dataset.tabIndex) : -1;
    const t = i >= 0 ? tabs[i] : undefined;
    return t && t !== tab ? t : null;
  }

  function cellsOver(x: number | null, y: number | null): void {
    cellDropTab = x === null || y === null ? null : (tabUnder(x, y)?.id ?? null);
  }

  /** Why a tab's cells may not go to another file, or "". Private and
   *  encrypted data never goes into a file that is not private. */
  async function sensitive(t: Tab): Promise<boolean> {
    if (t.private.file || t.private.folder) return true;
    return !!(await backend().encryptedOrigin(t.id).catch(() => null));
  }

  /** The most cells in any row (a loop: spreading a long list into Math.max overflows the stack). */
  function widest(rows: string[][]): number {
    let w = 0;
    for (const r of rows) if (r.length > w) w = r.length;
    return w;
  }

  /** The paste half of Paste, into any tab at a place: asks about rows
   *  that do not fit while rows are hidden, writes one undo step, and says
   *  how many rows were added. null: nothing was written. */
  async function pasteInto(dst: Tab, text: string, row: number, col: number): Promise<{ rows: number; width: number; added: number } | null> {
    const rows = parseTsv(text);
    if (rows.length === 0) return null;
    const fit = await askOverflow(dst, row, rows.length, dst.info.name);
    if (!fit) return null;
    const before = dst.info.rows;
    const done = await runOp({ op: "paste", spec: { row, col, op: "none", skip_blanks: false, transpose: false, stop_at_end: fit === "stop" }, text }, "Pasting", dst);
    if (!done) return null;
    return { rows: rows.length, width: widest(rows), added: Math.max(0, dst.info.rows - before) };
  }

  /** What a source tab looked like when its cells were copied: a change
   *  after that stops a move from clearing it. */
  // (The data revision moves on every edit, undo and redo, so edit, undo, edit
  // lands on the same undo position but not on the same key.)
  const sourceKey = (t: Tab): string => `${t.info.data_revision ?? 0}|${t.info.history_pos}|${t.info.rows}|${t.info.total_rows}|${JSON.stringify(t.filter ?? null)}`;

  /** Cells dropped on another tab: copied to that tab's active cell (Shift
   *  or "Move": then cleared here, but only once the other file has them;
   *  two files, so two undo steps). The source tab is the one the drag began
   *  in, whatever tab is active when the drop finishes. */
  async function dropCellsOnTab(src: Tab, r: Rect, dst: Tab, move: boolean): Promise<void> {
    if (dst === src) return;
    if (!src.info.indexed || !dst.info.indexed || src.busy || dst.busy) {
      say("Please wait until both files have been read, then try again.");
      return;
    }
    if (dst.info.read_only || dst.peeking || dst.info.peeking) {
      say(`${dst.info.name} is read-only (or showing an earlier step), so nothing can be put in it.`);
      return;
    }
    const n = cellCount(r);
    if (n * (move ? 2 : 1) > MAX_CELLS_PER_CHANGE) {
      say(
        move
          ? `Moving cells to another file works on up to ${(MAX_CELLS_PER_CHANGE / 2).toLocaleString()} at once. Use Copy and Paste for more.`
          : `Dropping cells on a tab copies up to ${MAX_CELLS_PER_CHANGE.toLocaleString()} cells at once. For more, use Copy (Ctrl+C) and Paste (Ctrl+V).`,
      );
      return;
    }
    if (src.columns.hiddenIn(r.col, r.col + r.cols)) {
      say("The selection has hidden columns, so it can't be dropped on another file. Show them first (Ctrl+Shift+0).");
      return;
    }
    if ((await sensitive(src)) && !(await sensitive(dst))) {
      say(`${src.info.name} is private (or from an encrypted file), so its cells are not copied into ${dst.info.name}, which is not. Make ${dst.info.name} private first, or copy them by hand.`);
      return;
    }
    const key = sourceKey(src);
    let text: string;
    try {
      text = await backend().copyCells(src.id, r.row, r.rows, r.col, r.cols, hiddenColsOf(src));
    } catch (e) {
      say(errorText(e));
      return;
    }
    const at = dst.cell;
    const put = await pasteInto(dst, text, at.row, at.col);
    if (!put) {
      say(`Nothing was copied to ${dst.info.name}.`);
      return;
    }
    // The cells actually sent (rows and columns of what was copied: hidden rows are not in it), not the size of the selection.
    const written = put.rows * put.width;
    const added = put.added > 0 ? `, added ${plural(put.added, "row")}` : "";
    const goTo = () => {
      const i = tabs.indexOf(dst);
      if (i >= 0) active = i;
    };
    if (!move) {
      say(`Copied ${plural(written, "cell")} to ${dst.info.name}${added}. Undo there with Ctrl+Z.`);
      messageAction = { label: `Go to ${dst.info.name}`, run: goTo };
      return;
    }
    // Move: the other file has the cells; only now clear the source tab,
    // which must still be what was copied (same changes, same rows, same filter).
    if (!tabs.includes(src) || sourceKey(src) !== key) {
      say(`Copied ${plural(written, "cell")} to ${dst.info.name}${added}, not removed from ${src.info.name}: it changed meanwhile.`);
      messageAction = { label: `Go to ${dst.info.name}`, run: goTo };
      return;
    }
    src.whatNext = "moving cells to another file";
    const cleared = await apply((x) => backend().fillCells(x.id, r.row, r.rows, r.col, r.cols, "", hiddenColsOf(x)), true, src);
    if (!cleared) {
      say(`Copied ${plural(written, "cell")} to ${dst.info.name}${added}, not removed from ${src.info.name}.`);
      messageAction = { label: `Go to ${dst.info.name}`, run: goTo };
      return;
    }
    say(`Moved ${plural(written, "cell")} to ${dst.info.name}${added}. Ctrl+Z in each file undoes its half.`);
    messageAction = { label: `Go to ${dst.info.name}`, run: goTo };
  }

  /** Released outside the grid: on a tab means drop there (Shift: move). */
  function cellsDropOutside(from: Rect, x: number, y: number, shift: boolean): void {
    cellDropTab = null;
    const src = tab;
    const dst = tabUnder(x, y);
    if (src && dst) void dropCellsOnTab(src, from, dst, shift).finally(() => void tick().then(() => grid?.focus()));
  }

  /** Copy or move the selection to a tab picked from a list (the keyboard
   *  and menu way of the same drop). */
  async function cellsToTab(move: boolean): Promise<void> {
    const src = tab;
    if (!src || !ready()) return;
    // Most recently used first.
    const others = tabs.filter((x) => x !== src).sort((a, b) => (usedAt.get(b.id) ?? 0) - (usedAt.get(a.id) ?? 0));
    if (!others.length) {
      say("Open a second file first.");
      return;
    }
    const shown = others.slice(0, 12);
    const a = await ask(
      move ? "Move the selected cells to which file?" : "Copy the selected cells to which file?",
      `They go in at that file's selected cell${move ? ", and are cleared here once they are there" : ""}.${others.length > shown.length ? ` (Only the first ${shown.length} other files are listed.)` : ""}`,
      [...shown.map((x, i) => ({ id: String(i), label: x.info.name, primary: i === 0 })), { id: "cancel", label: "Cancel" }],
      "cancel",
    );
    const dst = a === "cancel" ? undefined : shown[Number(a)];
    const from = oneBlock(src);
    if (dst && from && tabs.includes(src)) await dropCellsOnTab(src, from, dst, move);
  }

  // ---- file tabs: drag one along the bar to put it elsewhere -----------------
  // The tab follows the pointer and the tabs it passes slide aside, opening
  // a gap where it will land; it is put there on letting go (Esc first:
  // nothing moves). Text is never selected while dragging.
  type TabDrag = {
    /** The dragged tab (its place is looked up: a file opening or closing
     *  meanwhile ends the drag). */
    tab: Tab;
    count: number;
    from: number;
    x: number;
    /** The bar's scroll when the drag started (the tab stays under the
     *  pointer while the bar scrolls). */
    scroll0: number;
    dx: number;
    moved: boolean;
    pointer: number;
    to: number | null;
    w: number;
    /** Last pointer position, for scrolling while it rests near an edge. */
    clientX: number;
  };
  let tabDrag = $state<TabDrag | null>(null);
  let tabBar = $state<HTMLDivElement>();
  let tabDragged = false;
  let tabScrollTimer: ReturnType<typeof setInterval> | null = null;

  function tabDragKey(e: KeyboardEvent): void {
    if (e.key !== "Escape" || !tabDrag) return;
    e.preventDefault();
    e.stopPropagation();
    endTabDrag(false);
  }
  const noSelect = (e: Event) => e.preventDefault();

  function endTabDrag(drop: boolean): void {
    const d = tabDrag;
    tabDrag = null;
    window.removeEventListener("keydown", tabDragKey, true);
    window.removeEventListener("selectstart", noSelect, true);
    if (tabScrollTimer) clearInterval(tabScrollTimer);
    tabScrollTimer = null;
    // The click that ends a drag does not also pick a tab by its old place.
    tabDragged = !!d?.moved;
    if (!drop || !d?.moved || d.to === null) return;
    const from = tabs.indexOf(d.tab);
    if (from !== d.from || tabs.length !== d.count) return;
    const [moved] = tabs.splice(from, 1);
    tabs.splice(d.to, 0, moved!);
    // The tab just put down is the one shown, as when it is clicked.
    active = d.to;
    void tick().then(() => grid?.focus());
  }

  function tabPointerDown(e: PointerEvent, i: number): void {
    if (e.button !== 0 || (e.target as HTMLElement).closest("button")) return;
    const t = tabs[i];
    if (!t || !tabBar) return;
    // The press is not cancelled (menus still close on it); text selection
    // is stopped instead, for as long as the drag lasts.
    window.getSelection()?.removeAllRanges();
    window.addEventListener("selectstart", noSelect, true);
    tabDragged = false;
    tabDrag = {
      tab: t, count: tabs.length, from: i, x: e.clientX, scroll0: tabBar.scrollLeft, dx: 0, moved: false,
      pointer: e.pointerId, to: null, w: (e.currentTarget as HTMLElement).offsetWidth + 2, clientX: e.clientX,
    };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    window.addEventListener("keydown", tabDragKey, true);
  }

  /** Follow the pointer: where the tab is drawn and the gap it would land in. */
  function tabDragAt(clientX: number): void {
    const d = tabDrag;
    if (!d || !tabBar) return;
    // A file opened or closed meanwhile: the places changed, so stop.
    if (tabs.length !== d.count || tabs[d.from] !== d.tab) return endTabDrag(false);
    d.clientX = clientX;
    d.dx = clientX - d.x + (tabBar.scrollLeft - d.scroll0);
    // The gap nearest the pointer: before the first tab whose middle is
    // right of it. Tabs are measured where they sit in the bar (offsetLeft
    // ignores the sliding), so the gap does not chase the animation.
    const bar = tabBar.getBoundingClientRect();
    const x = clientX - bar.left + tabBar.scrollLeft;
    const els = [...tabBar.querySelectorAll<HTMLElement>("[data-tab-index]")];
    let gap = els.findIndex((el) => x < el.offsetLeft + el.offsetWidth / 2);
    if (gap < 0) gap = els.length;
    d.to = dropIndex(d.from, 1, gap, els.length);
  }

  function tabPointerMove(e: PointerEvent): void {
    const d = tabDrag;
    if (!d || e.pointerId !== d.pointer || !tabBar) return;
    if (!d.moved && Math.abs(e.clientX - d.x) < 6) return;
    d.moved = true;
    tabDragAt(e.clientX);
    // Near an edge of a long bar, keep scrolling while the pointer rests.
    if (!tabScrollTimer)
      tabScrollTimer = setInterval(() => {
        const t = tabDrag;
        if (!t || !tabBar) return;
        const bar = tabBar.getBoundingClientRect();
        const step = t.clientX < bar.left + 24 ? -12 : t.clientX > bar.right - 24 ? 12 : 0;
        if (!step) return;
        const before = tabBar.scrollLeft;
        tabBar.scrollLeft += step;
        if (tabBar.scrollLeft !== before) tabDragAt(t.clientX);
      }, 30);
  }

  /** Where tab `i` is drawn during a drag: the dragged one under the
   *  pointer, the ones it has passed slid aside by its width. */
  function tabShift(i: number): string | undefined {
    const d = tabDrag;
    if (!d?.moved) return undefined;
    if (i === d.from) return `translateX(${d.dx}px)`;
    if (d.to === null) return undefined;
    if (d.to > d.from && i > d.from && i <= d.to) return `translateX(${-d.w}px)`;
    if (d.to < d.from && i >= d.to && i < d.from) return `translateX(${d.w}px)`;
    return undefined;
  }

  function tabPointerUp(e: PointerEvent): void {
    if (!tabDrag || e.pointerId !== tabDrag.pointer) return;
    endTabDrag(e.type === "pointerup");
  }

  /** Move the shown tab one place left or right (Ctrl+Shift+PageUp/Down). */
  function moveTab(dir: number): void {
    const i = active;
    const to = i + dir;
    if (tabDrag || i < 0 || to < 0 || to >= tabs.length) return;
    const [t] = tabs.splice(i, 1);
    tabs.splice(to, 0, t!);
    active = to;
    void tick().then(() => grid?.focus());
  }

  /** Format > Rename column (also a double-click on a column heading). */
  async function askRename(col?: number): Promise<void> {
    const t = tab;
    if (!t || !ready()) return;
    if (t.info.read_only) {
      say("This file is locked (read-only); unlock it to rename a column.");
      return;
    }
    const c = col ?? t.cell.col;
    if (c < 0 || c >= t.info.cols) return;
    let name = "";
    const others = new Set<string>();
    let titles: string[] = [];
    if (t.header && t.info.rows > 0) {
      try {
        // The table's first row, even if a filter or hidden rows leave
        // another row at the top.
        name = await backend().firstRowCell(t.id, c);
        titles = await backend().firstRowNames(t.id);
        if (c < titles.length) titles[c] = name;
        titles.forEach((s, i) => {
          if (i !== c && s.trim()) others.add(s.trim().toLowerCase());
        });
      } catch (e) {
        say(errorText(e));
        return;
      }
    }
    renameAsk = { id: t.id, col: c, name, header: t.header && t.info.rows > 0, others, titles };
  }

  /** The columns Copy column names covers: the selected whole columns, or
   *  every column; hidden ones are left out (what is not shown is not
   *  copied). */
  function nameCols(t: Tab): number[] {
    const r = t.singleRect();
    if (!r) return [];
    const hidden = new Set(hiddenColsOf(t));
    const [from, n] = wholeCols(r, t) ? [r.col, r.cols] : [0, t.info.cols];
    const out: number[] = [];
    for (let c = from; c < from + n; c++) if (!hidden.has(c)) out.push(c);
    return out;
  }

  /** What every copy does first (as Ctrl+C): a pending cut and the copy
   *  outlines go, on every tab. */
  function clearCopyMarks(): void {
    cutEpoch++;
    for (const x of tabs) {
      if (x.cut || x.copied) {
        x.cut = null;
        x.copied = null;
        x.bump();
      }
    }
  }

  /** Edit › Copy column names: the names in one row, split by tabs (pastes
   *  into a table, here or in a spreadsheet) or, `asText`, by this file's
   *  delimiter (a line for a text file). */
  /** The last names line Copy column names put on the clipboard, and what
   *  splits it (a plain Paste of it fills one column per name). */
  let namesLine: { text: string; sep: string } | null = null;

  async function copyColumnNames(asText: boolean): Promise<void> {
    const t = tab;
    if (!t) return;
    if (!t.header) {
      say("This file has no column names: First row is header is off (Format menu). Turn it on if the first row holds the names.");
      return;
    }
    const cols = nameCols(t);
    if (!cols.length) return;
    const sep = asText ? t.info.delimiter || "," : "\t";
    clearCopyMarks();
    try {
      const line = await backend().columnNamesLine(t.id, cols, sep);
      await navigator.clipboard.writeText(line);
      namesLine = { text: line, sep };
      copiedFrom = t.id;
    } catch (e) {
      say(errorText(e));
      return;
    }
    const n = cols.length === 1 ? "1 column name" : `${cols.length.toLocaleString()} column names`;
    say(
      asText
        ? `Copied ${n} as one line split by ${sep === "," ? "commas" : `"${sep}"`}, for a text file. Pasted into a tab here it still fills one column per name; Paste as column names makes them its title row.`
        : `Copied ${n}. In another tab, Edit › Paste as column names makes them its title row; in a spreadsheet, paste them into a row.`,
    );
  }

  /** Edit › Copy with column names: the selection, with its columns' names
   *  as the first line. */
  async function copyWithNames(): Promise<void> {
    const t = tab;
    if (!t) return;
    const r = oneBlock(t);
    if (!r) return;
    // Without names, or with the title row in the selection: a plain copy.
    if (!t.header || r.row === 0) {
      await copy(false);
      return;
    }
    const hidden = new Set(hiddenColsOf(t));
    const cols: number[] = [];
    for (let c = r.col; c < r.col + r.cols; c++) if (!hidden.has(c)) cols.push(c);
    if (cellCount(r) >= LARGE_COPY && !(await privacyGate("The copy goes to the clipboard, where other programs can read it.", [r.col, r.cols]))) return;
    clearCopyMarks();
    const epoch = cutEpoch;
    const pos = t.info.history_pos;
    try {
      const names = await backend().columnNamesLine(t.id, cols, "\t");
      const body = await backend().copyCells(t.id, r.row, r.rows, r.col, r.cols, hiddenColsOf(t));
      await navigator.clipboard.writeText(`${names}\r\n${body}`);
      copiedFrom = t.id;
    } catch (e) {
      say(errorText(e));
      return;
    }
    // The copy outline, as Ctrl+C shows it.
    if (epoch === cutEpoch && t.info.history_pos === pos) {
      t.copied = { ...r };
      t.bump();
    }
    say(`Copied ${cellCount(r).toLocaleString()} cell${cellCount(r) === 1 ? "" : "s"} with the column names on top. Esc takes the outline away.`);
  }

  /** Edit › Paste as column names: copied names (a row, a line of a text
   *  file or a list) become this file's title row, from the active cell's
   *  column (the first selected one for whole columns, so the first after
   *  Select all) rightwards over shown columns; one undo step (two when a
   *  title row is added). Names past the last column ask first. */
  async function pasteColumnNames(): Promise<void> {
    const t = tab;
    if (!t || !ready()) return;
    if (t.info.read_only) {
      say("This file is locked (read-only); unlock it to change its column names.");
      return;
    }
    let text = "";
    try {
      text = await navigator.clipboard.readText();
    } catch {
      say("The clipboard could not be read.");
      return;
    }
    const names = namesFromText(text, t.info.delimiter);
    if (!names.length || names.every((n) => n === "")) {
      say("Nothing to paste: copy the names first (Edit › Copy column names in another tab, or a row or list of names from anywhere).");
      return;
    }
    if (names.length > 20_000) {
      say(`That is ${names.length.toLocaleString()} names; at most 20,000 can be pasted as column names at once.`);
      return;
    }
    const r = oneBlock(t);
    if (!r) return;
    const hidden = new Set(hiddenColsOf(t));
    let cells: [number, string][] = [];
    let widened = 0;
    for (let c = wholeCols(r, t) ? r.col : t.cell.col, i = 0; i < names.length; c++) {
      if (hidden.has(c)) continue;
      cells.push([c, names[i]!]);
      i++;
    }
    // A new, empty file (one blank cell): the names are all it will hold, so
    // no question and no extra row.
    let empty = false;
    if (!t.info.path && t.info.rows <= 1 && t.info.cols <= 1) {
      try {
        empty = (await backend().cellText(t.id, 0, 0)).text === "";
      } catch {
        empty = false;
      }
    }
    // More names than columns: widening the file changes its shape, so ask.
    const added = cells.length ? Math.max(0, cells.at(-1)![0] + 1 - t.info.cols) : 0;
    if (empty) widened = added;
    else if (added > 0) {
      const fit = cells.filter(([c]) => c < t.info.cols).length;
      const a = await ask(
        `${added === 1 ? "1 name runs" : `${added.toLocaleString()} names run`} past the last column`,
        `${plural(cells.length, "name")} from column ${cells[0]![0] + 1}, but the file has ${plural(t.info.cols, "column")}. Add ${added === 1 ? "a new column" : `${added.toLocaleString()} new columns`} at the right for ${added === 1 ? "it" : "them"}?`,
        [
          { id: "add", label: `Add ${added === 1 ? "1 column" : `${added.toLocaleString()} columns`}`, primary: true },
          ...(fit ? [{ id: "fit", label: `Paste only ${plural(fit, "name")}` }] : []),
          { id: "cancel", label: "Cancel" },
        ],
        "cancel",
      );
      if (a !== "add" && a !== "fit") return;
      if (a === "fit") cells = cells.filter(([c]) => c < t.info.cols);
      else widened = added;
    }
    // Repeats among the names pasted (after any "Paste only" choice).
    const pasted = cells.map(([, n]) => n);
    const dupes = new Set(pasted.filter((n, i) => n !== "" && pasted.findIndex((m) => m.toLowerCase() === n.toLowerCase()) !== i).map((n) => n.toLowerCase())).size;
    if (!t.header && empty) {
      // The blank row becomes the title row.
      t.header = true;
      t.headerTouched = true;
    } else if (!t.header) {
      if (t.info.filtered) {
        say("Show all rows first (a filter or hidden rows are on), then paste the column names.");
        return;
      }
      if (!(await change({ op: "insert_rows", at: 0, count: 1 }))) return;
      t.header = true;
      t.headerTouched = true;
      t.headerAddedAt = t.info.history_pos;
    }
    t.whatNext = "pasting column names";
    if (await apply((x) => backend().setColumnNames(x.id, cells))) {
      const wider = widened ? ` ${widened === 1 ? "1 column was" : `${widened.toLocaleString()} columns were`} added at the right.` : "";
      const same = dupes ? ` ${dupes === 1 ? "1 name is" : `${dupes} names are`} used twice.` : "";
      say(`Pasted ${cells.length === 1 ? "1 column name" : `${cells.length.toLocaleString()} column names`} into the title row.${wider}${same} Ctrl+Z undoes it.`);
    }
  }

  /** Give column `col` a new title: the header row's cell (one undo step).
   *  Without a header row, one is added first and switched on. */
  async function renameColumn(id: number, col: number, name: string, header: boolean, titles: string[] = []): Promise<void> {
    const t = tab;
    // The tab may have changed while the box was open.
    if (!t || t.id !== id || col >= t.info.cols) return;
    if (!header) {
      if (t.info.filtered) {
        say("Show all rows first (a filter or hidden rows are on), then rename the column.");
        return;
      }
      if (!(await change({ op: "insert_rows", at: 0, count: 1 }))) return;
      t.header = true;
      t.headerTouched = true;
      t.headerAddedAt = t.info.history_pos;
    }
    t.whatNext = "renaming a column";
    if (await apply((x) => backend().setFirstRowCell(x.id, col, name))) {
      // Row rules in data checks that named the column follow the new name
      // (undone and redone with the rename).
      let followed = "";
      // Without a header row every column was "Column N" to the rules.
      if (!header) titles = Array.from({ length: Math.min(t.info.cols, 20_000) }, () => "");
      // Past the names read (a file over 20,000 columns wide): rules are
      // not followed, and the status says so.
      const unread = col >= titles.length && t.checks.some((k) => k.kind.type === "expression");
      if (unread) titles = [];
      if (titles.length && t.checks.some((k) => k.kind.type === "expression")) {
        const before = $state.snapshot(t.checks) as Check[];
        const r = renameInChecks(before, titles, col, name);
        if (r.changed) {
          t.checks = r.checks;
          t.checkVersion++;
          t.refEdits = [...t.refEdits.filter((e) => e.applied), { at: t.info.history_pos, before, after: r.checks, applied: true }].slice(-50);
          followed = ` ${r.changed === 1 ? "1 data check rule now uses" : `${r.changed} data check rules now use`} the new name.`;
          if (r.pinned) followed += ` Another column has that name too, so ${r.pinned === 1 ? "1 reference" : `${r.pinned} references`} now name${r.pinned === 1 ? "s" : ""} the column by number ([#${col + 1}] style) to keep pointing at the same column.`;
        }
      }
      if (unread) followed = " Data check rules were not updated: this file is too wide to read every column's name.";
      say(name.trim() ? `Column ${col + 1} is now "${name}".${followed} Ctrl+Z undoes it.` : `Column ${col + 1} has no name now.${followed} Ctrl+Z undoes it.`);
    }
  }

  async function makeHeader(): Promise<void> {
    const t = tab;
    if (!t) return;
    const row = t.cell.row;
    if (row !== 0 && !(await change({ op: "move_rows", at: row, count: 1, to: 0 }))) return;
    t.header = true;
    t.headerTouched = true;
    t.cache.invalidate();
    grid?.select({ row: 1, col: t.cell.col });
  }

  // ---- Milestone 9: rows, links, formats -------------------------------------------
  /** Shown data rows of the selection (the header row left out). */
  function selectedRows(t: Tab): { at: number; count: number } {
    const r = t.singleRect();
    if (!r) return { at: 0, count: 0 };
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
      grid?.select({ row: Math.min(at, Math.max(t.firstRow, t.info.rows - 1)), col: t.cell.col });
      say(`${plural(count, "row")} hidden. Hidden rows are still saved; Ctrl+Shift+9 shows them again.`);
    }
  }

  async function showRows(all: boolean): Promise<void> {
    const t = tab;
    if (!t || !t.info.hidden_rows) return;
    const before = t.info.hidden_rows;
    const r = oneBlock(t);
    if (!r) return;
    // From the top when the selection starts at the first data row.
    const at = r.row <= t.firstRow ? 0 : r.row;
    const count = r.row + r.rows - at;
    const whole = r.rows >= t.info.rows - t.firstRow;
    const ok = await change({ op: "show_rows", which: all || whole ? "all" : { around: { at, count } } }, false);
    if (!ok || t !== tab) return;
    const n = before - t.info.hidden_rows;
    say(n ? `${plural(n, "hidden row")} shown again.` : "No hidden rows in the selection. Select the rows on both sides of the hidden ones, or use Unhide all rows.");
  }

  /** The dotted line in the row numbers was clicked: show the rows hidden by hand in that gap; a gap that is only
   *  filtered out opens the filter menu (which never clears anything by itself). */
  async function openSeam(row: number, x: number, y: number): Promise<void> {
    const t = tab;
    if (!t) return;
    const byHand = t.info.hidden_rows ? await backend().gapHidden(t.id, row).catch(() => 0) : 0;
    if (t !== tab) return;
    if (!byHand) {
      const col = t.info.filter_cols[0];
      if (col !== undefined) filterMenu = { col, x: x + 12, y };
      say("A filter leaves these rows out. Data > Clear filter shows every row.");
      return;
    }
    const at = Math.max(t.firstRow, row - 1);
    const to = Math.min(t.info.rows - 1, row);
    grid?.select({ row: at, col: 0 }, { row: to, col: Math.max(0, t.info.cols - 1) });
    void tick().then(() => showRows(false));
  }

  async function shuffleRows(): Promise<void> {
    const t = tab;
    if (!t) return;
    const one = (t.singleRect()?.rows ?? 2) <= 1;
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
        say(`Combined into a new table of ${plural(Math.max(0, d.rows - (spec.header ? 1 : 0)), "row")}. It is not saved yet (Ctrl+S).${d.run_notes?.length ? ` ${d.run_notes.join(" ")}` : ""}`);
        return;
      }
      if (r.split) {
        const spec = r.split;
        // null: the folder dialog was cancelled.
        const rep = await runJob("Splitting into files", () => backend().splitToFiles(t.id, spec, false));
        if (rep) say(`Wrote ${plural(rep.files, "file")} holding ${plural(rep.rows, "row")} into the folder you chose.${splitPrivateText(rep)}${guardNote(rep.guarded)}`);
        return;
      }
      if (r.diff) {
        const dreq = r.diff;
        const rep = await runJob("Comparing", () => backend().diffFiles(t.id, dreq.other, dreq.spec));
        if (!rep) return;
        addTab(rep.info, false);
        const nt = tabs[tabs.length - 1];
        if (nt) {
          nt.header = true;
          nt.headerTouched = true;
          nt.diffOn = true;
          nt.bump();
        }
        t.diffOn = true;
        t.diffVersion++;
        const other = tabs.find((x) => x.id === dreq.other);
        // The comparison all three tabs can save as a patch.
        const from = { a: t.id, b: dreq.other, keys: dreq.spec.keys, match_case: dreq.spec.match_case, trim: dreq.spec.trim };
        if (dreq.spec.keep_first) for (const x of [t, other, nt]) if (x) x.patchFrom = from;
        if (other) {
          other.diffOn = true;
          other.diffVersion++;
        }
        const c = rep.report;
        const warn: string[] = [];
        if (c.repeated) warn.push(`${plural(c.repeated, "key")} appear more than once${c.dup_a.length ? ` in the first file (${c.dup_a.slice(0, 3).join(", ")}${c.dup_a.length > 3 ? ", …" : ""})` : ""}${c.dup_b.length ? ` and in the second (${c.dup_b.slice(0, 3).join(", ")}${c.dup_b.length > 3 ? ", …" : ""})` : ""} (paired in order)`);
        if (c.empty_a || c.empty_b) warn.push(`rows with an empty key are never paired (${c.empty_a} in the first file, ${c.empty_b} in the second)`);
        if (c.only_a.length || c.only_b.length) warn.push(`columns only in the first file: ${c.only_a.join(", ") || "none"}; only in the second: ${c.only_b.join(", ") || "none"}`);
        if (c.truncated) warn.push("more rows differ than are listed (showing the first 1,000,000; the counts are exact)");
        say(`Compared by key: ${plural(c.changed, "changed row")} (${plural(c.cells, "cell")}), ${plural(c.added, "added row")}, ${plural(c.removed, "removed row")}, ${plural(c.same, "row")} the same. The differences opened as a new tab and are marked in all three tabs (Alt+F7 next, Alt+Shift+F7 previous).${warn.length ? ` Note: ${warn.join("; ")}.` : ""}`);
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
        // A sample's record opens beside it; the sample stays in front.
        if (rep.record) {
          addTab(rep.record, false);
          const rt = tabs[tabs.length - 1];
          if (rt) {
            rt.header = true;
            rt.headerTouched = true;
            rt.bump();
          }
          if (nt) active = tabs.indexOf(nt);
        }
        const c = rep.compare;
        say(
          c
            ? `Compared: ${plural(c.changed, "changed row")} (${plural(c.cells, "cell")}), ${plural(c.added, "added row")}, ${plural(c.removed, "removed row")}, ${plural(c.same, "row")} the same. The list opened as a new tab.`
            : rep.record
              ? `Drew ${plural(Math.max(0, rep.info.rows - (header ? 1 : 0)), "row")}${req.op === "sample" ? ` with seed ${req.spec.seed}` : ""}. The sample and its record (seed, method, file fingerprint, counts) opened as new tabs; save both with the audit.`
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
          if (rep) say(`Wrote ${plural(rep.files, "file")} holding ${plural(rep.rows, "row")} (${plural(rep.replaced, "file")} replaced).${splitPrivateText(rep)}${guardNote(rep.guarded)}`);
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
    const one = (t.singleRect()?.rows ?? 2) <= 1;
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
    const f = t.cell;
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
    const col = t.cell.col;
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
    if (!t) return;
    if (!t.info.path) {
      // A tab with no file yet (new, or made by a tool): name it; Save
      // suggests this name for the file.
      prompt = {
        title: "Rename tab",
        label: "Name",
        value: t.info.name === "Untitled" ? "" : t.info.name,
        note: "Save suggests this name for the file.",
        check: (v) => (v.trim().length > 120 ? "Keep the name to 120 characters." : ""),
        done: (v) => {
          void backend()
            .setDocTitle(t.id, v)
            .then((d) => {
              t.update(d, false);
              say(v.trim() ? `This tab is now called ${d.name}.` : "The tab's name is back to Untitled.");
            })
            .catch((e: unknown) => say(errorText(e)));
        },
      };
      return;
    }
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

  /** Close every tab but the active one (or those to its right). */
  async function closeOthers(rightOnly: boolean): Promise<void> {
    const keep = tab;
    if (!keep) return;
    const at = tabs.indexOf(keep);
    const gone = tabs.filter((t, i) => t !== keep && (!rightOnly || i > at));
    let closed = 0;
    for (const t of gone) {
      const i = tabs.indexOf(t);
      if (i < 0) continue;
      // Cancel on a "Save changes?" question stops here.
      if (!(await closeTab(i))) break;
      closed++;
    }
    const i = tabs.indexOf(keep);
    if (i >= 0) active = i;
    if (closed) say(`Closed ${plural(closed, "tab")}.`);
  }

  /** Ctrl+Shift+T: open again the most recent file that is not open (as
   *  browsers reopen a closed tab). */
  async function reopenClosed(): Promise<void> {
    await loadRecent();
    const r = recent.find((x) => x.open_doc === null);
    if (!r) {
      say("No recently closed file to open again.");
      return;
    }
    await openRecent(r);
  }

  /** Right-click on a file tab: its menu (the tab becomes the active one). */
  function tabMenu(e: MouseEvent, i: number): void {
    e.preventDefault();
    active = i;
    openContext(e.clientX, e.clientY, "tab", -1);
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
    const r = oneBlock(t);
    if (!r) return;
    try {
      const hidden = hiddenColsOf(t);
      const shown = shownColsIn(t, r);
      if (shown === 0) {
        say("Every selected column is hidden, so there is nothing to copy.");
        return;
      }
      const text = await backend().copyCells(t.id, r.row, r.rows, r.col, r.cols, hidden);
      const d = await backend().newDocument();
      addTab(d, false);
      const nt = tabs[tabs.length - 1];
      if (!nt) return;
      const rep = await backend().dataOp(nt.id, { op: "paste", spec: { row: 0, col: 0, op: "none", skip_blanks: false, transpose: false }, text });
      if (rep.info) nt.update(rep.info, true);
      const left = r.cols - shown;
      say(`Copied ${plural(r.rows, "row")} × ${plural(shown, "column")} into a new tab${left ? ` (${plural(left, "hidden column")} left out)` : ""}. It is not saved yet (Ctrl+S).`);
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
    if (!t || (!t.checks.length && !t.tableChecks.length)) {
      say("There are no data checks here.");
      return;
    }
    keepChecks([], false, []);
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
    const c = t.cell.col;
    t.divider = t.divider === c ? null : c;
    t.bump();
    say(t.divider === null ? "Group lines are off." : `A line now marks each change in ${columnTitle(t, c)}.`);
  }

  /** A colour scale or data bar on the active column (again: off). */
  function toggleScale(kind: ScaleKind): void {
    const t = tab;
    if (!t) return;
    const c = t.cell.col;
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
  /** The shown columns the selection touches (every block of it, each column once). */
  function selectedCols(t: Tab): number[] {
    const out: number[] = [];
    for (const c of colsOfAreas(t.areas)) {
      if (out.length >= 16_384) break;
      if (!t.columns.isHidden(c)) out.push(c);
    }
    return out;
  }

  function alignIs(a: "left" | "center" | "right" | null): boolean {
    const t = tab;
    if (!t) return false;
    const cols = selectedCols(t);
    return cols.length > 0 && cols.every((c) => (t.columns.aligns.get(c) ?? null) === a);
  }

  /** Line up the selected columns' text (null: automatic). */
  function setAlign(a: "left" | "center" | "right" | null): void {
    const t = tab;
    if (!t) return;
    const cols = selectedCols(t);
    for (const c of cols) {
      if (a) t.columns.aligns.set(c, a);
      else t.columns.aligns.delete(c);
    }
    t.bump();
    const how = a === null ? "automatic (numbers and dates on the right, text on the left)" : a;
    say(`${plural(cols.length, "column")}: alignment ${how}.`);
  }

  /** Long text in the selected columns stops at the column's edge, or spills
   *  into the empty cells beside it again. */
  function toggleClip(): void {
    const t = tab;
    if (!t) return;
    const cols = selectedCols(t);
    const on = !cols.every((c) => t.columns.clips.has(c));
    for (const c of cols) {
      if (on) t.columns.clips.add(c);
      else t.columns.clips.delete(c);
    }
    t.bump();
    say(on ? `Long text in ${plural(cols.length, "column")} now stops at the column's edge.` : `Long text in ${plural(cols.length, "column")} spills into empty cells beside it again.`);
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

  /** Ctrl+Shift+0: the hidden columns inside the selection come back (as in
   *  Excel); with none inside it, every hidden column does. */
  function unhideCols(): void {
    const t = tab;
    if (!t) return;
    const r = oneBlock(t);
    if (!r) return;
    if (r.cols > 1 && t.columns.hiddenIn(r.col, r.col + r.cols)) {
      t.columns.showRange(r.col, r.col + r.cols);
      say("Showed the hidden columns inside the selection.");
    } else {
      const n = t.columns.hiddenCount();
      t.columns.showAll();
      say(`Showed all ${plural(n, "hidden column")}. (Select across hidden columns first to show just those.)`);
    }
    t.bump();
  }

  /** Format › Column width…: an exact width for the selected columns. */
  function askColWidth(): void {
    const t = tab;
    if (!t) return;
    const cols = selectedCols(t).filter((c) => !t.columns.isHidden(c));
    if (!cols.length) return;
    const w0 = Math.round(t.columns.width(cols[0]!));
    prompt = {
      title: "Column width",
      label: `Width of ${cols.length === 1 ? "the column" : plural(cols.length, "column")} in pixels`,
      value: String(w0),
      note: "20 to 2,000. Double-click a column's right edge to fit it to its text instead.",
      check: (v) => (/^\d+$/.test(v.trim()) && Number(v) >= 20 && Number(v) <= 2000 ? "" : "Type a whole number from 20 to 2,000."),
      done: (v) => {
        for (const c of cols) t.columns.setWidth(c, Number(v));
        t.bump();
      },
    };
  }

  /** The data rows of the selected block (the rows a height applies to). */
  function selectedRowList(t: Tab): number[] | null {
    const r = oneBlock(t);
    if (!r) return null;
    const out: number[] = [];
    for (let i = Math.max(r.row, t.firstRow); i < r.row + r.rows && out.length <= MAX_CUSTOM_ROWS; i++) out.push(i);
    if (out.length > MAX_CUSTOM_ROWS) {
      say(`Row heights work on up to ${MAX_CUSTOM_ROWS.toLocaleString()} rows at once. Select fewer rows.`);
      return null;
    }
    return out;
  }

  function askRowsPx(): void {
    const t = tab;
    if (!t) return;
    const rows = selectedRowList(t);
    if (!rows || !rows.length) return;
    prompt = {
      title: "Height of the selected rows",
      label: "Height in pixels",
      value: String(t.rowHeights.custom(rows[0]!) ?? ""),
      note: `${MIN_ROW_PX} to ${MAX_ROW_PX}. Only the ${plural(rows.length, "selected row")} change; drag the line under a row number to do the same with the mouse. Reset puts them back to the height every row has.`,
      check: (v) => (/^\d+$/.test(v.trim()) && Number(v) >= MIN_ROW_PX && Number(v) <= MAX_ROW_PX ? "" : `Type a whole number from ${MIN_ROW_PX} to ${MAX_ROW_PX}.`),
      done: (v) => {
        if (grid?.setRowsHeight(rows, Number(v))) say(`${plural(rows.length, "row")} now ${v} pixels tall. Ctrl+Z undoes it.`);
      },
    };
  }

  function fitSelectedRows(): void {
    const t = tab;
    if (!t) return;
    const rows = selectedRowList(t);
    if (rows && rows.length) grid?.fitRowHeights(rows);
  }

  function resetSelectedRows(): void {
    const t = tab;
    if (!t) return;
    const rows = selectedRowList(t);
    if (!rows) return;
    grid?.resetRowsHeight(rows);
    say(`${plural(rows.length, "row")} back to the height every row has. Ctrl+Z undoes it.`);
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
    const r0 = Math.max(t.firstRow, t.singleRect()?.row ?? t.cell.row);
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
  /** Alt+Down: on a cell, a pick list (the column's allowed values, or the
   *  values already in it, as in Excel); on the header row, the filter menu. */
  async function altDown(): Promise<void> {
    if (grid && (await grid.openPick())) return;
    openFilterMenu();
  }

  function toggleHeader(): void {
    const t = tab;
    if (!t) return;
    t.headerAddedAt = null;
    t.headerOffByUndo = false;
    t.header = !t.header;
    t.headerTouched = true;
    // With unsaved changes (a title row added, say), the choice is kept
    // only when they are saved; Don't save keeps the one from before.
    if (!t.info.modified) void rememberTab(t);
    grid?.select(t.sel.anchor, t.sel.focus, true, t.sel.active);
  }

  let splitEl = $state<HTMLDivElement | null>(null);
  /** View › Split: two panes of the tab, each scrolling on its own; again
   *  (or the other way) changes or closes it. */
  function toggleSplit(dir: "rows" | "cols"): void {
    const t = tab;
    if (!t) return;
    if (t.split?.dir === dir) {
      t.split = null;
      say("Split closed.");
    } else {
      if (!t.split) t.splitScroll = { scrollX: t.scrollX, scrollY: t.scrollY };
      t.split = { dir, at: t.split?.at ?? 0.5 };
      say(dir === "rows" ? "Split top and bottom: each pane scrolls on its own. Drag the bar to the edge to close it." : "Split side by side: each pane scrolls on its own. Drag the bar to the edge to close it.");
    }
    void tick().then(() => grid?.focus());
  }
  /** Drag the split bar: resize, or close at an edge. */
  function splitDrag(e: PointerEvent): void {
    const t = tab;
    const box = splitEl?.getBoundingClientRect();
    if (!t?.split || !box) return;
    e.preventDefault();
    const bar = e.currentTarget as HTMLElement;
    bar.setPointerCapture(e.pointerId);
    const dir = t.split.dir;
    const at = (ev: PointerEvent) => (dir === "cols" ? (ev.clientX - box.left) / box.width : (ev.clientY - box.top) / box.height);
    const move = (ev: PointerEvent) => {
      if (t.split) t.split = { dir, at: Math.min(0.97, Math.max(0.03, at(ev))) };
    };
    const up = (ev: PointerEvent) => {
      bar.removeEventListener("pointermove", move);
      bar.removeEventListener("pointerup", up);
      const f = at(ev);
      if (f < 0.05 || f > 0.95) {
        t.split = null;
        say("Split closed.");
      }
    };
    bar.addEventListener("pointermove", move);
    bar.addEventListener("pointerup", up);
  }

  function freezeAtCursor(): void {
    const t = tab;
    if (!t) return;
    t.frozenRows = Math.max(0, t.cell.row - t.firstRow);
    t.frozenCols = t.cell.col;
    t.scrollX = 0;
    t.scrollY = 0;
    if (t.frozenRows === 0 && t.frozenCols === 0) say("Select the cell below and right of what should stay in view, then freeze.");
  }

  /** The columns a type command acts on: the one right-clicked, else the selected ones. */
  function typeTargets(t: Tab): number[] {
    return contextCol !== null ? [contextCol] : selectedCols(t);
  }

  function typeIsNow(ty: ColumnType): boolean {
    const t = tab;
    if (!t) return false;
    const cols = typeTargets(t);
    return cols.length > 0 && cols.every((c) => JSON.stringify(typeAt(t.info.typed, c)) === JSON.stringify(ty));
  }

  /** Say what the selected (or right-clicked) columns hold. Nothing in the file changes: it only decides how
   *  the cells are read as numbers and dates. One undo step. */
  /** A ready-made data rule from the column's type: the cells must read as a number (or a date) the way the
   *  column is typed. Marks only; nothing is rewritten. */
  function addTypeRule(): void {
    const t = tab;
    if (!t) return;
    const c = t.cell.col;
    const ty = typeAt(t.info.typed, c);
    const type = ty === "number" ? "number" : typeof ty === "object" ? "date" : null;
    if (!type) {
      say("Treat this column as Number or Date first; then this adds a rule that its cells must read that way.");
      return;
    }
    const check = { ...newCheck(type, c), name: `${columnTitle(t, c)} must be a ${type}` };
    t.checks = [...t.checks, check];
    t.checksMarked = true;
    t.checkVersion++;
    t.bump();
    say(`Added the rule “${check.name}”. Cells that do not read that way are marked (Data > Check now lists them); nothing is changed.`);
  }

  /** A "no repeats" rule over the selected columns together (2 to 8), then the checks dialog to look at it.
   *  One column selected: the dialog opens with that column's rule, to tick the others there. */
  function addUniqueTogether(): void {
    const t = tab;
    if (!t) return;
    const cols = selectedCols(t);
    if (cols.length > 8) {
      say("A rule can use at most 8 columns together: select fewer.");
      return;
    }
    const [col, ...also] = cols.length ? cols : [t.cell.col];
    const check: Check = { col: col!, kind: also.length ? { type: "unique", match_case: false, also } : { type: "unique", match_case: false } };
    t.checks = [...t.checks, check];
    t.checksMarked = true;
    t.checkVersion++;
    t.bump();
    checksOpen = true;
  }

  /** A "value must exist in another file" rule on the active column (the file and its column are picked in
   *  the checks dialog, which opens on it). */
  function addExistsCheck(): void {
    const t = tab;
    if (!t) return;
    const other = tabs.find((o) => o.id !== t.id);
    const check: Check = { col: t.cell.col, kind: { type: "foreign_key", table: other?.info.name ?? "", column: "", match_case: false, trim: true, allow_blank: true } };
    t.checks = [...t.checks, check];
    t.checksMarked = true;
    t.checkVersion++;
    t.bump();
    checksOpen = true;
  }

  /** Table checks (row count, no blanks, totals that add up): the checks dialog opens on them. */
  function openTableChecks(): void {
    if (!tab) return;
    checksAtTable = true;
    checksOpen = true;
  }

  async function setTypeOfColumns(ty: ColumnType): Promise<void> {
    const t = tab;
    if (!t) return;
    const cols = typeTargets(t);
    if (!cols.length) return;
    t.recordView();
    try {
      for (const c of cols) t.update(await backend().setColumnType(t.id, c, ty), true);
    } catch (e) {
      say(errorText(e));
      return;
    }
    t.recordView();
    void rememberTypes(t);
    const names = cols.length === 1 ? `Column ${cols[0]! + 1}` : `${cols.length} columns`;
    say(
      ty === "auto"
        ? `${names} read from the cells again (Auto).`
        : `${names} treated as ${typeName(ty)}. The file is not changed; numbers and dates in it are read by that rule. Ctrl+Z puts it back.`,
    );
  }

  /** Types remembered for a file that were not applied because its columns changed (the person can still apply them). */
  const heldTypes = new WeakMap<Tab, TypeMemory>();

  /** A file opened again gets the column types it had. Id and Text are always safe to put back; Number and Date
   *  only on columns where most of the first 1,000 rows really read that way. If the number of columns is not
   *  the same as last time nothing is applied (a note says so, and one click applies them anyway). */
  async function applyRememberedTypes(t: Tab, anyway: boolean): Promise<void> {
    const path = t.info.path;
    if (!path) return;
    try {
      const p = await backend().privateState(t.id);
      if (p.file || p.folder) {
        await rememberColumnTypes(path, null, true);
        return;
      }
      const mem = anyway ? (heldTypes.get(t) ?? null) : await recallColumnTypes(path);
      if (!mem || mem.types.every((x) => x === "auto")) return;
      let info = t.info;
      for (let i = 0; i < 40 && !info.indexed && (info.rows < 20 || info.cols === 0); i++) {
        await new Promise((r) => setTimeout(r, 50));
        info = await backend().docInfo(t.id);
      }
      if (!anyway && mem.cols !== info.cols) {
        heldTypes.set(t, mem);
        say("Column types from last time were not applied (the columns changed). Use Format > Apply last time's column types to apply them anyway.");
        return;
      }
      const first = t.header ? 1 : 0;
      const sample = anyway ? null : await fetchRows(t, first, 1000, mem.types.map((_, c) => c).filter((c) => c < info.cols));
      const keep: ColumnType[] = [];
      let skipped = 0;
      mem.types.forEach((ty, c) => {
        if (ty === "auto" || c >= info.cols) {
          keep.push("auto");
          return;
        }
        if (typeof ty === "object" || ty === "number") {
          // Only where most of the sample really reads that way.
          const idx = mem.types.map((_, k) => k).filter((k) => k < info.cols).indexOf(c);
          const cells = (sample ?? []).map((r) => r[idx] ?? "").filter((v) => v.trim() !== "");
          const fit = cells.filter((v) => (ty === "number" ? readNumberFor(undefined, c, v) !== null : readDateFor(undefined, c, v) !== null)).length;
          if (!anyway && cells.length > 0 && fit * 2 < cells.length) {
            keep.push("auto");
            skipped++;
            return;
          }
        }
        keep.push(ty);
      });
      if (keep.every((x) => x === "auto")) return;
      t.recordView();
      t.update(await backend().setColumnTypes(t.id, { types: keep }), true);
      t.recordView();
      const n = keep.filter((x) => x !== "auto").length;
      say(`Column types from last time were put back on ${n === 1 ? "1 column" : `${n} columns`}${skipped ? ` (${skipped} left on Auto because their cells no longer read that way)` : ""}. Ctrl+Z undoes it.`);
    } catch {
      // Nothing is applied.
    }
  }

  /** Keep this file's column types for next time (a fingerprint of its path and the types, never values;
   *  nothing for a file in private mode). */
  async function rememberTypes(t: Tab): Promise<void> {
    const path = t.info.path;
    if (!path) return;
    try {
      const p = await backend().privateState(t.id);
      if (p.file || p.folder) {
        await rememberColumnTypes(path, null, true);
        return;
      }
      await rememberColumnTypes(path, { cols: t.info.cols, types: t.info.column_types.types }, false);
    } catch {
      // Not remembered this time.
    }
  }

  function hideCols(): void {
    const t = tab;
    if (!t) return;
    const r = oneBlock(t);
    if (!r) return;
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

  /** Go To / the name box: a row as numbered on screen (the whole file's
   *  numbers while filtered), "R12 C3", Excel's "F200", or a column title
   *  with or without a row ("Last name 200"). */
  async function goTo(text: string): Promise<void> {
    const t = tab;
    if (!t) return;
    const titles = Array.from({ length: t.info.cols }, (_, c) => grid?.title(c) ?? String(c + 1));
    const g = parseGoto(text, titles);
    if (g && g.col !== null && (g.col < 0 || g.col >= t.info.cols)) {
      say(`There is no column ${g.col + 1}: the file has ${plural(t.info.cols, "column")}.`);
      return;
    }
    if (!g) {
      say('Type a row number ("250"), a row and column ("R250 C3" or "250, 3"), Excel\'s "F250", or a column title ("Last name 250").');
      return;
    }
    const col = g.col ?? t.cell.col;
    let row = t.cell.row;
    let note = "";
    if (g.row !== null) {
      const n = Math.max(1, g.row) - 1;
      if (t.info.filtered) {
        // The numbers on screen are the whole file's: find that row among
        // those the filter shows.
        const r = await backend().shownRowOf(t.id, n).catch(() => null);
        if (!r) {
          say("The filter shows no rows.");
          return;
        }
        row = r[0];
        if (!r[1]) note = `Row ${g.row.toLocaleString()} is hidden by the filter, so this is the next row shown.`;
      } else row = Math.min(t.info.rows - 1, n);
    }
    if (t !== tab) return;
    grid?.select({ row, col });
    grid?.focus();
    // The name box shows where it went, even when that is the same cell.
    const no = t.info.filtered ? t.cache.rowNumber(row) : row;
    gotoText = `R${(no ?? row) + 1} C${col + 1}`;
    if (note) say(note);
  }

  /** Undo or redo, or with `jump` go to another place in the history in ONE engine call (a checkpoint) while doing
   *  for every step crossed what a Ctrl+Z press does for it: look changes, saved-view groups, rename follow-ups,
   *  the header switch, the filter, row heights. */
  async function undoRedo(op: "undo" | "redo", jump?: { target: number; looks?: number; run: (t: Tab) => Promise<DocInfo> }): Promise<void> {
    if (grid?.isEditing()) return;
    const t = tab;
    if (!t) return;
    // One at a time per tab: a held Ctrl+Z repeats faster than a big undo.
    if (undoBusy.has(t)) return;
    // A change to the look (widths, hiding, formats, highlights...) made at
    // this undo position is undone before the engine step that reached it,
    // and redone before the next one.
    t.recordView();
    const pos = t.info.history_pos;
    const look = op === "undo" ? t.lookSteps.at(-1) : t.lookRedo.at(-1);
    if (jump) {
      // Every look change crossed goes back (or forward) too, the earliest one deciding what the look ends as.
      let ends: NonNullable<typeof look>["before"] | undefined;
      // A checkpoint named by the person remembers how many look changes there were then: the ones made after it go
      // too (even one made at the same place in the history). One made by the app has only the place to go by.
      const keep = jump.looks;
      if (op === "undo") {
        for (let l = t.lookSteps.at(-1); l && (keep !== undefined ? t.lookSteps.length > keep : l.pos > jump.target); l = t.lookSteps.at(-1)) {
          t.lookRedo.push(t.lookSteps.pop()!);
          ends = l.before;
        }
      } else {
        for (let l = t.lookRedo.at(-1); l && (keep !== undefined ? t.lookSteps.length < keep : l.pos < jump.target); l = t.lookRedo.at(-1)) {
          t.lookSteps.push(t.lookRedo.pop()!);
          ends = l.after;
        }
      }
      if (ends) t.restoreView({ look: ends, sort: null }, false);
    } else if (look && look.pos === pos) {
      if (op === "undo") t.lookRedo.push(t.lookSteps.pop()!);
      else t.lookSteps.push(t.lookRedo.pop()!);
      t.restoreView({ look: op === "undo" ? look.before : look.after, sort: null }, false);
      say(`${op === "undo" ? "Undid" : "Redid"} a change to how the table looks (widths, formats, hiding and so on).`);
      return;
    }
    if (!jump && op === "redo" && t.redoCut === pos) return;
    // A saved view's sort, filter and look go back (or forward) together.
    const group = jump ? undefined : op === "undo" ? t.groups.find((g) => g.end === pos) : t.groups.find((g) => g.start === pos);
    const target = jump ? jump.target : group ? (op === "undo" ? group.start : group.end) : op === "undo" ? pos - 1 : pos + 1;
    undoBusy.add(t);
    if (op === "redo") t.redoing++;
    const posBefore = t.info.history_pos;
    try {
      if (jump) {
        // One call for the whole way; the rename follow-ups cover every step crossed at once (they compare
        // positions, not presses).
        if (!(await apply(jump.run, true, t))) return;
        followRefEdits(t, op);
        followHeaderEdits(t);
      } else {
        for (;;) {
          const at = t.info.history_pos;
          await change({ op });
          // Data-check rules a rename rewrote follow each step.
          followRefEdits(t, op);
          followHeaderEdits(t);
          if (t.info.history_pos === at || t.info.history_pos === target) break;
        }
      }
      // The widths, formats and so on as they were before the step undone,
      // or after the step redone; the filter rules as the engine has them.
      const now = t.info.history_pos;
      const step = now !== target ? undefined : op === "undo" ? t.stepViews.get(target + 1)?.before : t.stepViews.get(now)?.after;
      if (step) t.restoreView(step, true);
      // The look kept with the step was let go: heights would sit on the wrong rows.
      if (t.clearHeightsIfLookLost(posBefore, !!step)) {
        // The "Undid ..." message below takes it along; otherwise it is said alone.
        rowNote = t.rowHeightsNote;
        t.rowHeightsNote = "";
        window.setTimeout(() => {
          if (rowNote) say("");
        }, 800);
      }
      // Select what the step changed and say what it was (as Excel does).
      if (now !== pos) {
        const w = t.stepWhat.get(op === "undo" ? pos : now);
        // A sort, filter or tool over the whole table: the selection stays
        // where it is (as in Excel), only the message says what was undone.
        if (w && !w.wide && t.info.rows > 0 && t.info.cols > 0) {
          const fit = (c: { row: number; col: number }) => ({ row: Math.min(c.row, t.info.rows - 1), col: Math.min(c.col, t.info.cols - 1) });
          // Scrolled to the active cell (the top of a whole column, not
          // its last row).
          const act = fit(w.sel.active ?? w.sel.anchor);
          grid?.select(fit(w.sel.anchor), fit(w.sel.focus), false, act);
          grid?.scrollIntoView(act);
        }
        const n = Math.abs(now - pos);
        const what = w?.what ?? "the last change";
        say(`${op === "undo" ? "Undid" : "Redid"} ${what}${n > 1 ? ` (${n} steps together)` : ""}.`);
      }
      if (now !== pos) {
        t.filter = t.info.has_filter ? await backend().filterSpec(t.id).catch(() => t.filter) : null;
        if (t.info.has_filter) t.filterButtons = true;
      }
      // Undoing the header row Rename column added: the setting it switched
      // on goes off again, so the first record is not taken for titles.
      if (t.headerAddedAt !== null && t.info.history_pos < t.headerAddedAt && !t.headerOffByUndo) {
        if (t.header) {
          t.header = false;
          t.headerOffByUndo = true;
          grid?.select(t.sel.anchor, t.sel.focus, true, t.sel.active);
          say('The added header row is undone, so "First row is header" is off again.');
        } else t.headerAddedAt = null;
      } else if (op === "redo" && t.headerOffByUndo && t.headerAddedAt !== null && t.info.history_pos >= t.headerAddedAt) {
        // Redo brought the header row back: switch the setting back on.
        t.headerOffByUndo = false;
        t.header = true;
        grid?.select(t.sel.anchor, t.sel.focus, true, t.sel.active);
        say('The header row is back, so "First row is header" is on again.');
      }
    } finally {
      // What the undo or redo put back is not a change of its own.
      t.baseline();
      if (op === "redo") t.redoing--;
      undoBusy.delete(t);
    }
  }
  const undoBusy = new Set<Tab>();

  /** Go to a checkpoint: the engine moves in one call, the screen does for each step crossed what Ctrl+Z does. */
  async function restoreCheckpoint(c: Checkpoint): Promise<void> {
    const t = tab;
    if (!t || t.busy) return;
    // The steps away are asked again now: the list on screen may be older than the last change.
    const fresh = (await backend().checkpointList(t.id).catch(() => [])).find((x) => x.id === c.id);
    if (!fresh || !fresh.reachable) {
      say("That checkpoint can no longer be reached: the undo history was trimmed past it, a newer change replaced it, or the file was saved.");
      return;
    }
    if (fresh.current || fresh.steps === 0) return;
    const target = t.info.history_pos + fresh.steps;
    await undoRedo(fresh.steps < 0 ? "undo" : "redo", { target, looks: t.checkpointLooks.get(c.id), run: (x) => backend().checkpointRestore(x.id, c.id) });
    t.checkpointTick++;
    say(`Went to the checkpoint “${c.name}”. Ctrl+Z steps back through what was undone.`);
  }

  /** "First row is header" follows the undo position across the steps that
   *  set it together with rows set aside or put back (C4): undo, redo,
   *  checkpoints and the steps panel. */
  function followHeaderEdits(t: Tab): void {
    const pos = t.info.history_pos;
    let header: boolean | null = null;
    for (const e of [...t.headerEdits].reverse()) {
      if (e.applied && pos < e.at) {
        e.applied = false;
        header = e.before;
      }
    }
    for (const e of t.headerEdits) {
      if (!e.applied && pos >= e.at) {
        e.applied = true;
        header = e.after;
      }
    }
    if (header !== null && header !== t.header) {
      t.header = header;
      t.cache.invalidate();
      grid?.select(t.sel.anchor, t.sel.focus, true, t.sel.active);
      t.bump();
    }
  }

  // ---- report notes (C4) ----------------------------------------------------------
  /** The rows a "set aside" would take: above the cursor row ("head"), or
   *  the selected rows ("foot"). */
  function asideSpan(t: Tab, place: NotePlace): { at: number; count: number } | null {
    if (place === "head") return { at: 0, count: t.cell.row };
    const r = oneBlock(t);
    if (!r) return null;
    return { at: r.row, count: Math.max(0, Math.min(r.rows, t.info.rows - r.row)) };
  }
  function asideKey(t: Tab): string {
    const h = asideSpan(t, "head");
    const f = asideSpan(t, "foot");
    return `${t.id}:${t.info.history_pos}:${t.info.step_mark}:${t.info.total_rows}:${h?.count}:${f?.at}:${f?.count}`;
  }
  /** The engine's answer for the current selection (a sort may have put other
   *  rows at the ends), asked shortly after the selection settles. */
  let asideChecked = $state<{ key: string; head: string | null; foot: string | null } | null>(null);
  let asideTimer = 0;
  $effect(() => {
    const t = tab;
    if (!t || !t.info.indexed || t.info.filtered) return;
    const key = asideKey(t);
    const h = asideSpan(t, "head");
    const f = asideSpan(t, "foot");
    window.clearTimeout(asideTimer);
    asideTimer = window.setTimeout(() => {
      const doc = t.id;
      const one = (s: { at: number; count: number } | null, place: NotePlace) =>
        s && s.count > 0 ? backend().setAsideCheck(doc, s.at, s.count, place).catch((e) => errorText(e)) : Promise.resolve(null);
      void Promise.all([one(h, "head"), one(f, "foot")]).then(([head, foot]) => {
        if (tab?.id === doc) asideChecked = { key, head, foot };
      });
    }, 120);
  });
  /** Why "Headers start here" / "Set aside rows as footer notes" is greyed ("" when it is not). */
  function asideWhy(place: NotePlace): string {
    const t = tab;
    if (!t) return "open a file first";
    if (!t.info.indexed) return "the file is still being read";
    if (t.info.read_only) return "the file is open read-only";
    if (t.info.filtered) return "show all rows first: some rows are hidden or filtered out, so the first and last rows of the file may not be on screen";
    const s = asideSpan(t, place);
    if (place === "head" && s?.count === 0) return t.header ? "this row already holds the column titles" : "";
    if (!s || s.count === 0) return "select the rows to set aside";
    const c = asideChecked;
    if (c && c.key === asideKey(t)) return (place === "head" ? c.head : c.foot) ?? "";
    return "";
  }
  /** Lines of report notes, in words, for messages. */
  function notesWords(n: NoteCountsLike): string {
    const parts: string[] = [];
    if (n.above_file) parts.push(`the ${plural(n.above_file, "line")} above the table when the file was opened`);
    if (n.above_set + n.below_set) parts.push(`the ${plural(n.above_set + n.below_set, "line")} you set aside`);
    if (n.below_file) parts.push(`the ${plural(n.below_file, "line")} below the table in the file`);
    return parts.length > 1 ? `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}` : (parts[0] ?? "no lines");
  }
  type NoteCountsLike = { above_file: number; above_set: number; below_file: number; below_set: number };
  const noteLines = (t: Tab | undefined): number => {
    const n = t?.info.notes;
    return n ? n.above_file + n.above_set + n.below_file + n.below_set : 0;
  };
  /** "Headers start here": the rows above the cursor row (or the first
   *  `count` rows) go aside as report notes and the row becomes the header,
   *  in one undo step. */
  async function headersStartHere(count?: number): Promise<void> {
    const t = tab;
    if (!t) return;
    const n = count ?? t.cell.row;
    if (n <= 0) {
      if (!t.header) {
        t.header = true;
        t.headerTouched = true;
        t.bump();
      }
      say("The first row holds the column titles; there is nothing above it to set aside.");
      return;
    }
    const why = t.info.filtered ? asideWhy("head") : await backend().setAsideCheck(t.id, 0, n, "head").catch((e) => errorText(e));
    if (why) {
      say(`The rows above cannot be set aside: ${why}.`);
      return;
    }
    const before = t.header;
    t.whatNext = "setting the title lines aside";
    if (!t.rowHeights.empty) t.pendingRows = { kind: "delete", at: 0, count: n };
    const ok = await apply((x) => backend().setAside(x.id, 0, n, "head"));
    t.pendingRows = null;
    if (!ok) return;
    t.header = true;
    t.headerTouched = true;
    t.headerAskClosed = true;
    t.titleLines = 0;
    t.headerEdits.push({ at: t.info.history_pos, before, after: true, applied: true });
    t.cache.invalidate();
    grid?.select({ row: Math.min(1, Math.max(0, t.info.rows - 1)), col: t.cell.col });
    t.bump();
    say(`Set ${plural(n, "row")} aside above the table as report notes; the first row now holds the column titles. Saving writes them back above the table (View › Report notes shows them). Ctrl+Z undoes it.`);
  }
  /** The selected last rows of the file go aside below the table. */
  async function setAsideFoot(): Promise<void> {
    const t = tab;
    if (!t) return;
    const s = asideSpan(t, "foot");
    if (!s || s.count === 0) return;
    const why = t.info.filtered ? asideWhy("foot") : await backend().setAsideCheck(t.id, s.at, s.count, "foot").catch((e) => errorText(e));
    if (why) {
      say(`Those rows cannot be set aside: ${why}.`);
      return;
    }
    t.whatNext = "setting rows aside as footer notes";
    if (!t.rowHeights.empty) t.pendingRows = { kind: "delete", at: s.at, count: s.count };
    const ok = await apply((x) => backend().setAside(x.id, s.at, s.count, "foot"));
    t.pendingRows = null;
    if (!ok) return;
    grid?.select({ row: Math.max(0, Math.min(s.at, t.info.rows) - 1), col: t.cell.col });
    say(`Set ${plural(s.count, "row")} aside below the table as report notes. Saving writes them after the last row, whatever the sort (View › Report notes shows them). Ctrl+Z undoes it.`);
  }
  /** Every report note goes back into the table (one undo step). */
  async function putBackNotes(): Promise<void> {
    const t = tab;
    if (!t) return;
    const above = (t.info.notes?.above_file ?? 0) + (t.info.notes?.above_set ?? 0);
    const before = t.header;
    t.whatNext = "putting the report notes back";
    if (!t.rowHeights.empty) t.pendingRows = { kind: "reset" };
    const ok = await apply((x) => backend().putBackNotes(x.id));
    t.pendingRows = null;
    if (!ok) return;
    // The lines above the table are its first rows again: they are not titles.
    const after = above > 0 ? false : before;
    if (after !== before) {
      t.header = after;
      t.headerTouched = true;
      t.cache.invalidate();
    }
    t.headerEdits.push({ at: t.info.history_pos, before, after, applied: true });
    t.bump();
    say(`The report notes are rows of the table again${above > 0 ? ' (at the top, so "First row is header" is off)' : ""}. Lines with another number of fields than the rest are ragged rows: Data › Show only rows with the wrong number of fields finds them. Ctrl+Z undoes it.`);
  }
  /** The switch that leaves the report notes out of saves (asks first). */
  async function setKeepNotes(keep: boolean): Promise<void> {
    const t = tab;
    if (!t || !t.info.notes || t.info.notes.keep === keep) return;
    if (!keep) {
      const a = await ask("Leave out the report notes?", `Leave out the report notes when saving: ${notesWords(t.info.notes)}. They stay here (switch it back on and nothing is lost); Ctrl+Z undoes the switch.`, [
        { id: "off", label: "Leave them out" },
        { id: "keep", label: "Keep them", primary: true },
      ], "keep");
      if (a !== "off") return;
    }
    if (await apply((x) => backend().keepNotes(x.id, keep))) say(keep ? "Saving writes the report notes above and below the table again." : "Saving leaves the report notes out; they are kept here until then.");
  }

  /** Undo/redo of a rename: the data-check rules it rewrote go back, or
   *  are rewritten again. Rules changed by hand since are left as they are. */
  function followRefEdits(t: Tab, op: "undo" | "redo"): void {
    const pos = t.info.history_pos;
    // The tab's rules are a live copy: compare by content.
    const is = (list: Check[]) => JSON.stringify(t.checks) === JSON.stringify(list);
    let moved = false;
    if (op === "undo") {
      for (const e of [...t.refEdits].reverse()) {
        if (!e.applied || pos >= e.at) continue;
        e.applied = false;
        if (is(e.after)) {
          t.checks = e.before;
          moved = true;
        }
      }
    } else {
      for (const e of t.refEdits) {
        if (e.applied || pos < e.at) continue;
        e.applied = true;
        if (is(e.before)) {
          t.checks = e.after;
          moved = true;
        }
      }
    }
    if (moved) t.checkVersion++;
  }

  // ---- long operations ------------------------------------------------------------
  /** Run a long engine call: the status bar shows progress and offers
   *  Cancel. Returns null when cancelled. */
  async function runJob<T>(label: string, f: () => Promise<T>): Promise<T | null> {
    const t = tab;
    if (!t) return null;
    // (No early several-blocks check here: this also carries read-only work
    // such as Find. Any write inside it is refused by the backend guard.)
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

  const NO_CHECKPOINT = "No checkpoint yet: name this moment first (Ctrl+Alt+K).";
  /** The newest checkpoint the table can be compared with, or why there is none. */
  function newestSince(list: Checkpoint[]): { checkpoint: Checkpoint | null; why: string } {
    const checkpoint = [...list].reverse().find((c) => c.since_note === "") ?? null;
    return { checkpoint, why: checkpoint ? "" : (list.at(-1)?.since_note ?? NO_CHECKPOINT) };
  }
  // Whether the menu entry is greyed follows every change of the table and of the checkpoints.
  $effect(() => {
    const t = tab;
    if (!t) return;
    void t.info.history_pos;
    void t.info.step_mark;
    void t.checkpointTick;
    const doc = t.id;
    void backend()
      .checkpointList(doc)
      .then((list) => {
        const n = newestSince(list);
        if (tab?.id === doc) sinceNewest = { doc, ready: !!n.checkpoint, why: n.why };
      })
      .catch(() => {});
  });
  async function openSince(): Promise<void> {
    const t = tab;
    if (!t) return;
    try {
      const n = newestSince(await backend().checkpointList(t.id));
      if (n.checkpoint) sinceFor = { doc: t.id, checkpoint: n.checkpoint };
      else say(n.why);
    } catch (e) {
      say(errorText(e));
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
      t.whatNext = label.toLowerCase();
      t.wideNext = true;
      const d = await runJob(label, () => f(t));
      if (!d) return false;
      t.update(d, true);
      return true;
    } catch (e) {
      say(errorText(e));
      return false;
    } finally {
      t.whatNext = null;
      t.wideNext = false;
    }
  }

  // ---- sort and filter ---------------------------------------------------------------
  function columnTitle(t: Tab, c: number): string {
    const v = t.header ? t.cache.get(0, c) : undefined;
    if (v) return v.replace(/[\r\n\t]+/g, " ").slice(0, 60);
    const shown = t === tab ? grid?.title(c) : undefined;
    return shown && shown !== String(c + 1) ? shown.slice(0, 60) : `Column ${c + 1}`;
  }

  // The counts the dialogs' notes about hidden rows use.
  $effect(() => {
    const i = tab?.info;
    const titles = tab?.header ? 1 : 0;
    hiddenRows.total = i ? Math.max(0, i.total_rows - titles) : 0;
    hiddenRows.hidden = i ? Math.max(0, i.total_rows - i.rows) : 0;
  });

  async function sortRows(spec: SortSpec): Promise<void> {
    const t = tab;
    if (await applyJob("Sorting", (x) => backend().sortRows(x.id, spec))) {
      if (t) {
        t.sort = spec;
        t.sortAt = t.info.history_pos;
        t.bump();
      }
      const kept = spec.scope === "shown" && hiddenRows.hidden;
      say(kept ? `Sorted the shown rows; ${plural(hiddenRows.hidden, "hidden row")} stayed in place. Undo (Ctrl+Z) puts the rows back.` : "Sorted. Undo (Ctrl+Z) puts the rows back.");
    }
  }

  function sortByColumn(descending: boolean, col = tab?.cell.col ?? 0): void {
    const t = tab;
    if (!t) return;
    void sortRows({ keys: [{ col, descending, kind: "auto", match_case: false }], keep_first: t.header, scope: rowsHidden(t) ? "shown" : "all" });
  }

  function openSortDialog(): void {
    const t = tab;
    if (!t) return;
    sortDialog = [{ col: t.cell.col, descending: false, kind: "auto", match_case: false }];
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
    grid?.select({ row: t.firstRow, col: t.cell.col });
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
    const f = t.cell;
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

  function openFilterMenu(col = tab?.cell.col ?? 0): void {
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
    // As in Excel: with several cells selected, Find and Replace look only
    // in them (whole columns: in those columns); with one cell, everywhere.
    // Pressed again while Find is open, a new block selected since is
    // taken; one cell (a hit) keeps the block searched so far.
    const t = tab;
    const r = t?.singleRect();
    if (t && !r && t.extra.length > 0) {
      // Several blocks selected: Find looks in all of them (kept whatever a hit selects).
      findOptions.scope = "selection";
      findBlocks = t.areas.map((a) => ({ ...a }));
      findBlock = null;
    } else if (t && r && cellCount(r) > 1) {
      findBlocks = [];
      findOptions.scope = wholeCols(r, t) && !wholeRows(r, t) ? "column" : wholeRows(r, t) && wholeCols(r, t) ? "all" : "selection";
      // The block searched stays the one selected now, whatever a hit selects.
      findBlock = findOptions.scope !== "all" ? { ...r } : null;
    } else if (t && r && !findOpen) {
      findBlocks = [];
      if (findOptions.scope === "selection") findOptions.scope = "all";
      findBlock = null;
    }
    findOpen = true;
    findReplace = replace;
    if (sel && sel.length < 200 && !sel.includes("\n")) findOptions.text = sel;
    void tick().then(() => findPanel?.focus());
  }

  /** Right-click › Find this value / Replace this value / Find all like
   *  this in the column: the active cell's text, as plain text. */
  function findValue(mode: "find" | "replace" | "column"): void {
    const t = tab;
    if (!t) return;
    const v = t.cache.get(t.cell.row, t.cell.col) ?? "";
    if (!v) {
      say("The cell is empty: there is nothing to look for.");
      return;
    }
    findOptions.text = v.slice(0, 1000);
    findOptions.kind = "plain";
    findOptions.wholeCell = mode === "column";
    // The value as it is: no escapes, no word or accent matching; "like
    // this" also matches case.
    findOptions.escapes = false;
    findOptions.wholeWord = false;
    findOptions.ignoreAccents = false;
    if (mode === "column") findOptions.matchCase = true;
    if (mode === "column") {
      findOptions.scope = "column";
      findBlock = { row: 0, rows: Math.max(1, t.info.rows), col: t.cell.col, cols: 1 };
    } else {
      findOptions.scope = "all";
      findBlock = null;
    }
    findOpen = true;
    findReplace = mode === "replace";
    void tick().then(() => {
      if (mode === "column") void findPanel?.findAll();
      else if (mode === "replace") findPanel?.focusReplace();
      else findPanel?.focus();
    });
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
    const f = t.cell;
    void t.version;
    const key = `${t.id}:${f.row}:${f.col}:${t.version}`;
    if (key === cellBarFor || cellBarDirty) return;
    cellBarFor = key;
    // While filtered, the row's number in the whole file (as the row
    // headings show it).
    const shownNo = t.info.filtered ? t.cache.rowNumber(f.row) : f.row;
    gotoText = `R${(shownNo ?? f.row) + 1} C${f.col + 1}`;
    // A value that came from a formula shows its formula.
    let fxShown = false;
    if (t.hasFormulas)
      void backend()
        .cellFormulaOf(t.id, f.row, f.col)
        .then((fx) => {
          if (fx !== null && cellBarFor === key && !cellBarDirty) {
            cellBarText = fx;
            fxShown = true;
          }
        })
        .catch(() => {});
    const cached = t.cache.get(f.row, f.col);
    if (cached !== undefined && !t.cache.isTruncated(f.row, f.col)) {
      cellBarText = cached;
      cellBarTruncated = false;
      return;
    }
    backend()
      .cellText(t.id, f.row, f.col)
      .then((v) => {
        if (cellBarFor !== key || cellBarDirty || fxShown) return;
        cellBarText = v.text;
        cellBarTruncated = v.truncated;
      })
      .catch(() => {});
  });

  /** Enter in the cell bar: the value goes into the active cell and the
   *  cursor moves on, as Enter in a cell does (Excel's formula bar).
   *  Ctrl+Enter puts it into every selected cell and stays. */
  async function applyCellBar(all = false): Promise<void> {
    const t = tab;
    if (!t) return;
    if (cellBarTruncated) {
      say("This cell is too long to edit here.");
      return;
    }
    const f = t.cell;
    const multi = all && cellCountAreas(t.areas) > 1;
    const ok = await commitCell(f.row, f.col, cellBarText, multi);
    if (ok) {
      cellBarDirty = false;
      cellBarFor = "";
      grid?.focus();
      if (!all) grid?.advance(...enterStep(prefs.data.enterMove, false));
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
      // In the expanded bar Enter is a line break, so Ctrl+Enter applies
      // there (to the active cell, unless several are selected).
      void applyCellBar(e.ctrlKey);
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
  // Several blocks selected (Ctrl+click): only commands that say they handle
  // them may run; the write helpers and the backend refuse everything else.
  reg.guard = (c) => (tab && tab.extra.length > 0 && !c.multiArea && !SELECTION_FREE.test(c.id) ? ONE_BLOCK : null);
  reg.onRefuse = (why) => say(why);
  writeGuard.check = (doc) => {
    const t = tabs.find((x) => x.id === doc);
    return t && t.extra.length > 0 && !reg.multiOk ? ONE_BLOCK : null;
  };
  Tab.activeCommand = () => reg.active;
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
    { id: "file.openSqlite", label: "Open a table from a &database or GeoPackage…", run: () => { sqliteStart = null; sqliteMode = "open"; } },
    { id: "file.saveSqlite", label: "Save as a table in a data&base…", run: () => { sqliteMode = "save"; }, enabled: ready },
    { id: "file.save", label: "&Save", keys: ["Ctrl+S", "Shift+F12"], run: () => void save(false), enabled: ready },
    { id: "file.saveAs", label: "Save &As…", keys: ["F12"], run: () => void save(true), enabled: ready },
    { id: "file.import", label: "&Import…", run: () => void importFile() },
    { id: "file.importClipboard", label: "Import from clip&board…", run: () => void importClipboard() },
    { id: "file.importNotes", label: "Show what the last import kept exactly (long numbers, leading zeros)", run: () => say(lastImportNote || "The last import had nothing that needed a note: no long numbers, no leading-zero formats.") },
    { id: "file.export", label: "&Export…", run: () => void privacyGate("Exporting writes another copy of it.").then(async (ok) => { if (ok) { exportTabs = await exportTabList(); exportOpen = true; } }), enabled: ready },
    { id: "file.safeCopy", label: "Save a share-safe cop&y…", run: () => void openPrivacy(), enabled: ready },
    { id: "file.private", label: "Private file mode (this file)", run: () => void togglePrivate(false), enabled: hasTab, checked: () => !!tab?.private.file },
    { id: "file.privateFolder", label: "Private file mode (every file in this folder)", run: () => void togglePrivate(true), enabled: hasTab, checked: () => !!tab?.private.folder },
    { id: "tools.findPrivate", label: "Find &private data…", run: () => void openPrivacy(), enabled: ready },
    { id: "file.print", label: "&Print…", keys: ["Ctrl+P"], run: () => void openPrint(false), enabled: ready },
    { id: "file.pdf", label: "Save as P&DF…", run: () => void openPrint(true), enabled: ready },
    { id: "file.mail", label: "&Mailing labels and letters…", run: () => void openMail(), enabled: ready },
    { id: "edit.note", label: "&Note and flag on this row…", keys: ["Shift+F2"], run: () => { const t = tab; if (t) notesRow = t.cell.row; }, enabled: ready },
    { id: "edit.changes", label: "List of &my changes…", run: () => void openChanges(), enabled: ready },
    { id: "edit.checkpoint", label: "&Name this moment (checkpoint)…", keys: ["Ctrl+Alt+K"], run: () => nameMoment(), enabled: hasTab },
    { id: "edit.sinceCheckpoint", label: "What changed since a checkpoint…", run: () => void openSince(), enabled: () => !!tab && sinceNewest.doc === tab.id && sinceNewest.ready, why: () => sinceNewest.why },
    { id: "tools.recipe", label: "&Recipe (save or run steps)…", run: () => void openRecipes(), enabled: ready },
    { id: "tools.searchFolder", label: "Search a &folder of files…", keys: ["Ctrl+Shift+F"], run: () => { folderSearchOpen = true; } },
    { id: "view.showEdits", label: "Show my &edits", run: () => setShowEdits(!tab?.showEdits), enabled: hasTab, checked: () => !!tab?.showEdits },
    { id: "file.csvGuard", label: GUARD_LABEL, run: () => { prefs.data.csvFormulaGuard = !prefs.data.csvFormulaGuard; prefs.save(); say(prefs.data.csvFormulaGuard ? "Reports and share-safe copies now put an apostrophe in front of cells Excel could run as formulas." : "Reports and share-safe copies now write every cell as it is."); }, checked: () => prefs.data.csvFormulaGuard },
    { id: "file.options", label: "Op&tions…", run: () => { optionsOpen = true; }, enabled: () => !SECOND_WINDOW },
    { id: "data.profile", label: "Column &profile…", run: () => { profileCol = tab?.cell.col ?? null; }, enabled: ready },
    { id: "data.counts", label: "Cou&nt values in a new tab", keys: ["Alt+Shift+F"], run: () => void openCounts(), enabled: ready },
    { id: "data.countsShow", label: "Show these rows", run: () => void countsShow(false), enabled: () => ready() && !!tab?.countsOf },
    { id: "data.countsHide", label: "Hide these rows", run: () => void countsShow(true), enabled: () => ready() && !!tab?.countsOf },
    { id: "data.compute", label: "Compu&ted column…", run: () => { computeMode = "compute"; computeOpen = true; }, enabled: ready },
    { id: "data.changeFormula", label: "Change column with a formula…", run: () => { computeMode = "change"; computeOpen = true; }, enabled: ready },
    { id: "data.filterFormula", label: "Filter by formula…", run: () => { computeMode = "filter"; computeOpen = true; }, enabled: ready },
    { id: "data.checks", label: "Data checks (&validation)…", run: () => { checksOpen = true; }, enabled: ready },
    { id: "data.existsCheck", label: "Check that values exist in another file…", run: addExistsCheck, enabled: ready },
    { id: "data.uniqueTogether", label: "No repeats across these columns together…", run: addUniqueTogether, enabled: ready },
    { id: "data.tableChecks", label: "Table checks (row count, totals that add up)…", run: openTableChecks, enabled: ready },
    { id: "data.typeRule", label: "Data rule from this column's type (must read as a number or date)", run: addTypeRule, enabled: ready },
    { id: "data.fieldCheck", label: "Check &fields with patterns…", run: () => { fieldCheckOpen = true; }, enabled: ready },
    { id: "data.nextProblem", label: "Jump to &next problem", keys: ["F7"], run: () => void jumpProblem(false), enabled: ready },
    { id: "data.prevProblem", label: "Jump to &previous problem", keys: ["Shift+F7"], run: () => void jumpProblem(true), enabled: ready },
    { id: "data.checkNow", label: "Check the whole table &now", run: () => { const t = tab; if (t) void checkNow(t); }, enabled: () => ready() && !!(tab?.checks.length || tab?.tableChecks.length) },
    { id: "data.diffNext", label: "Compared files: &next change", keys: ["Alt+F7"], run: () => void jumpDiff(false), enabled: () => !!tab?.diffOn },
    { id: "data.diffPrev", label: "Compared files: &previous change", keys: ["Alt+Shift+F7"], run: () => void jumpDiff(true), enabled: () => !!tab?.diffOn },
    { id: "data.diffClear", label: "Compared files: &clear the colours", run: () => void clearDiff(), enabled: () => !!tab?.diffOn },
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

    { id: "edit.undo", label: "&Undo", multiArea: true, keys: ["Ctrl+Z"], run: () => undoRedo("undo"), enabled: () => ready() && !!tab?.canUndo },
    { id: "edit.redo", label: "&Redo", multiArea: true, keys: ["Ctrl+Y", "Ctrl+Shift+Z"], run: () => undoRedo("redo"), enabled: () => ready() && !!tab?.canRedo },
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
    { id: "edit.copy", label: "&Copy", multiArea: true, keys: ["Ctrl+C"], run: () => copy(false), enabled: hasTab },
    { id: "edit.paste", label: "&Paste", multiArea: true, run: pasteFromMenu, enabled: ready },
    { id: "edit.moveCut", label: "&Move what was cut here", run: () => void pasteCut(null), enabled: () => ready() && !!tab?.cut },
    { id: "edit.cancelCut", label: "Ca&ncel cut", run: cancelCut, enabled: () => !!tab?.cut },
    { id: "edit.copyNames", label: "Copy column &names", run: () => void copyColumnNames(false), enabled: () => !!tab?.info.indexed },
    { id: "edit.copyNamesText", label: "Copy column names as a text line (for text files)", run: () => void copyColumnNames(true), enabled: () => !!tab?.info.indexed },
    { id: "edit.copyWithNames", label: "Copy with column names", keys: ["Ctrl+Shift+C"], run: () => void copyWithNames(), enabled: hasTab },
    { id: "edit.pasteNames", label: "Paste as column names", run: () => void pasteColumnNames(), enabled: () => ready() && !tab?.info.read_only },
    { id: "edit.pasteSpecial", label: "Paste &special…", keys: ["Ctrl+Alt+V"], run: () => openOp("paste_special"), enabled: ready },
    { id: "edit.clear", label: "Cle&ar contents", multiArea: true, keys: ["Delete"], run: clearSelection, enabled: ready, repeatable: true },
    { id: "edit.fillDown", label: "Fill &down", keys: ["Ctrl+D"], run: () => fill("down"), enabled: ready, repeatable: true },
    { id: "edit.fillRight", label: "Fill r&ight", keys: ["Ctrl+R"], run: () => fill("right"), enabled: ready, repeatable: true },
    { id: "edit.fillUp", label: "Fill &up", run: () => fill("up"), enabled: ready, repeatable: true },
    { id: "edit.fillLeft", label: "Fill le&ft", run: () => fill("left"), enabled: ready, repeatable: true },
    { id: "edit.fillSeries", label: "Fill series…", run: () => openOp("series"), enabled: ready },
    { id: "edit.cell", label: "&Edit cell", multiArea: true, keys: ["F2"], run: () => grid?.startEdit(), enabled: ready },
    {
      id: "edit.today",
      label: "Insert today's date",
      keys: ["Ctrl+;"],
      // As in Excel: the date goes into the cell being edited, so a time can
      // be added, Enter keeps it and Esc drops it.
      run: () => { if (tab && !lockedSay(tab)) void grid?.startEdit(grid.today()); },
      enabled: ready,
    },
    {
      id: "edit.pickDate",
      label: "Pick a date from a calendar…",
      keys: ["Ctrl+Shift+D"],
      run: () => {
        const t = tab!;
        const { row, col } = t.cell;
        datePick = { row, col, text: t.cache.get(row, col) ?? "" };
      },
      enabled: ready,
    },
    {
      id: "edit.now",
      label: "Insert current time",
      keys: ["Ctrl+Shift+:", "Ctrl+Shift+;"],
      run: () => { if (tab && !lockedSay(tab)) void grid?.startEdit(grid.nowTime()); },
      enabled: ready,
    },
    {
      id: "edit.selectAll",
      label: "Select &all",
      keys: ["Ctrl+A", "Ctrl+Shift+Space"],
      run: () => selectAll(),
      enabled: hasTab,
    },
    { id: "edit.selectRegion", label: "Select the current block of data", multiArea: true, keys: ["Ctrl+Shift+*", "Ctrl+Shift+8"], run: () => selectRegion(), enabled: hasTab },
    { id: "edit.contextMenu", label: "Open the right-click menu", multiArea: true, keys: ["Shift+F10", "ContextMenu"], run: () => contextFromKeys(), enabled: hasTab },
    { id: "edit.copyAbove", label: "Copy the value from the cell above", keys: ["Ctrl+'", 'Ctrl+Shift+"'], run: () => void copyAbove(), enabled: ready },
    { id: "view.toActive", label: "Scroll back to the active cell", keys: ["Ctrl+Backspace"], run: () => { if (tab) grid?.scrollIntoView(tab.cell); }, enabled: hasTab },
    { id: "edit.shrinkSel", label: "Select just the active cell", keys: ["Shift+Backspace"], run: () => { if (tab) grid?.select(tab.cell); }, enabled: hasTab },
    { id: "edit.nextCorner", label: "Move the active cell to the next corner of the selection", keys: ["Ctrl+."], run: () => nextCorner(), enabled: hasTab },
    { id: "edit.extendMode", label: "Extend the selection with the arrow keys (on or off)", multiArea: true, keys: ["F8"], run: () => toggleExtend(), enabled: hasTab, checked: () => extendMode },
    {
      id: "edit.gotoSpecial",
      label: "Go to s&pecial…",
      keys: ["Ctrl+Shift+G"],
      run: () => {
        if (tab) specialAsk = true;
      },
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
    { id: "edit.deleteCellsAsk", label: "Delete cells…", run: () => { shiftAsk = { insert: false }; }, enabled: ready },
    { id: "edit.deleteRows", label: "Delete &rows", run: deleteRows, enabled: ready, repeatable: true },
    { id: "edit.deleteCols", label: "Delete co&lumns", run: deleteCols, enabled: ready, repeatable: true },
    { id: "edit.moveRowsUp", label: "Move rows up", keys: ["Alt+Shift+ArrowUp"], run: () => moveRows(-1), enabled: ready, repeatable: true },
    { id: "edit.moveRowsDown", label: "Move rows down", keys: ["Alt+Shift+ArrowDown"], run: () => moveRows(1), enabled: ready, repeatable: true },
    { id: "edit.moveColsLeft", label: "Move columns left", keys: ["Alt+Shift+ArrowLeft"], run: () => moveCols(-1), enabled: ready, repeatable: true },
    { id: "edit.moveColsRight", label: "Move columns right", keys: ["Alt+Shift+ArrowRight"], run: () => moveCols(1), enabled: ready, repeatable: true },

    { id: "insert.smart", label: "&Cells…", keys: ["Ctrl+Shift++", "Ctrl+Shift+=", "Ctrl++"], run: insertSmart, enabled: ready, repeatable: true },
    { id: "insert.cellsAsk", label: "Insert cells…", run: () => { shiftAsk = { insert: true }; }, enabled: ready },
    { id: "insert.rowsAbove", label: "&Rows above", run: () => insertRows(false), enabled: ready, repeatable: true },
    { id: "insert.rowsBelow", label: "Rows &below", run: () => insertRows(true), enabled: ready, repeatable: true },
    { id: "insert.colsLeft", label: "Columns &left", run: () => insertCols(false), enabled: ready, repeatable: true },
    { id: "insert.colsRight", label: "Columns r&ight", run: () => insertCols(true), enabled: ready, repeatable: true },
    { id: "insert.rowsMany", label: "Rows (choose how &many)…", run: () => { insertAsk = "rows"; }, enabled: ready },
    { id: "insert.colsMany", label: "Columns (choose how ma&ny)…", run: () => { insertAsk = "cols"; }, enabled: ready },
    { id: "insert.symbol", label: "&Symbol…", run: openSymbols, enabled: ready },

    { id: "format.autofit", label: "&AutoFit column width", run: () => { const r = oneBlock(tab!); if (r) void grid?.fitCols(r.col, r.cols); }, enabled: hasTab },
    { id: "format.hideCols", label: "&Hide columns", keys: ["Ctrl+0"], run: hideCols, enabled: hasTab },
    { id: "format.tidyTitles", label: "&Tidy column titles…", run: openTidyTitles, enabled: ready },
    { id: "data.compareCols", label: "Compare t&wo columns…", run: openCompareCols, enabled: ready },
    { id: "data.gaps", label: "Find gaps in a sequence…", run: openGaps, enabled: ready },
    { id: "data.relations", label: "Find relationships…", run: openRelations, enabled: ready },
    { id: "data.patchSave", label: "Save changes as a patch…", run: () => void openPatchSave(), enabled: () => ready() && !!tab?.patchFrom },
    { id: "data.patchApply", label: "Apply a patch…", run: openPatchApply, enabled: ready },
    { id: "data.compareLists", label: "Compare lists…", run: openSets, enabled: ready },
    {
      id: "format.protectCols",
      label: "&Protect column (no changes)",
      run: () => void toggleProtect(),
      enabled: ready,
      checked: () => {
        const t = tab;
        if (!t) return false;
        const on = new Set(t.info.protected ?? []);
        return protectTargets(t).every((c) => on.has(c));
      },
    },
    { id: "format.showCols", label: "&Unhide columns", keys: ["Ctrl+Shift+0", "Ctrl+Shift+)"], run: () => unhideCols(), enabled: () => !!tab && tab.columns.hiddenCount() > 0 },
    { id: "format.colWidth", label: "Set column &width…", run: () => askColWidth(), enabled: hasTab },
    { id: "view.whichRows", label: "Which rows do edits act on?", run: () => say(actingWords(hiddenRows.hidden, hiddenRows.total, hiddenRows.including > 0)), enabled: ready },
    { id: "format.showAllCols", label: "Unhide all columns", run: () => { tab!.columns.showAll(); tab!.bump(); }, enabled: () => !!tab && tab.columns.hiddenCount() > 0 },
    { id: "format.header", label: "First row is &header", run: toggleHeader, enabled: hasTab, checked: () => !!tab?.header },
    { id: "col.typeAuto", label: "Treat this column as Auto (guess from the cells)", run: () => void setTypeOfColumns("auto"), enabled: hasTab, checked: () => typeIsNow("auto") },
    { id: "col.typeText", label: "Treat this column as Text", run: () => void setTypeOfColumns("text"), enabled: hasTab, checked: () => typeIsNow("text") },
    { id: "col.typeNumber", label: "Treat this column as Number", run: () => void setTypeOfColumns("number"), enabled: hasTab, checked: () => typeIsNow("number") },
    { id: "col.typeDate", label: "Treat this column as Date (the file's date order)", run: () => void setTypeOfColumns({ date: null }), enabled: hasTab, checked: () => typeIsNow({ date: null }) },
    { id: "col.typeDateDmy", label: "Treat this column as Date, day first", run: () => void setTypeOfColumns({ date: "dmy" }), enabled: hasTab, checked: () => typeIsNow({ date: "dmy" }) },
    { id: "col.typeDateMdy", label: "Treat this column as Date, month first", run: () => void setTypeOfColumns({ date: "mdy" }), enabled: hasTab, checked: () => typeIsNow({ date: "mdy" }) },
    { id: "col.typeId", label: "Treat this column as Id (a code: never a number)", run: () => void setTypeOfColumns("id"), enabled: hasTab, checked: () => typeIsNow("id") },
    { id: "format.typesAnyway", label: "Apply last time's column types", run: () => { if (tab) void applyRememberedTypes(tab, true); }, enabled: () => !!tab && heldTypes.has(tab) },
    { id: "format.askHeader", label: "Ask about the title row when a file opens", run: toggleAskHeader, checked: () => prefs.data.askHeader },
    { id: "format.askHeaderAgain", label: "Ask about the title row again", run: askHeaderAgain, enabled: () => !!tab?.info.path && !tab.private.file && !tab.private.folder },
    { id: "format.headerYes", label: "Title-row question: yes, column names", keys: ["Alt+Shift+Y"], run: () => void answerHeader(true), enabled: headerAsking },
    { id: "format.headerNo", label: "Title-row question: no, it's data", keys: ["Alt+Shift+N"], run: () => void answerHeader(false), enabled: headerAsking },
    { id: "format.forgetHeader", label: "Forget this file's title-row choice", run: () => void forgetHeaderOf(), enabled: () => !!tab?.info.path },
    { id: "format.forgetHeaders", label: "Forget remembered title rows…", run: () => void forgetAllHeaders() },
    { id: "data.checkDates", label: "Check dates in this column…", run: () => void checkDates(), enabled: datesReady, why: datesWhy },
    { id: "data.reviewDates", label: "Review and convert dates…", run: () => void reviewDates(), enabled: datesReady, why: datesWhy },
    { id: "data.nextDate", label: "Next ambiguous date", run: () => void nextDate(true), enabled: () => !!tab?.dateCheck, why: () => (tab?.dateCheck ? "" : "use Check dates in this column first") },
    { id: "data.prevDate", label: "Previous ambiguous date", run: () => void nextDate(false), enabled: () => !!tab?.dateCheck, why: () => (tab?.dateCheck ? "" : "use Check dates in this column first") },
    { id: "data.dateMarksOff", label: "Remove the ambiguous date marks", run: endDateCheck, enabled: () => !!tab?.dateCheck },
    { id: "data.repairRow", label: "Repair this row…", run: () => { if (tab) repairFrom = { doc: tab.id, row: tab.cell.row }; }, enabled: () => ready() && repairWhyNow() === "", why: repairWhyNow },
    { id: "data.ragged", label: "Show only rows with the wrong number of fields", run: () => void showRagged("all"), enabled: raggedReady, why: raggedWhy },
    { id: "data.raggedShort", label: "Show only rows with too few fields", run: () => void showRagged("short"), enabled: raggedReady, why: raggedWhy },
    { id: "data.raggedLong", label: "Show only rows with too many fields", run: () => void showRagged("long"), enabled: raggedReady, why: raggedWhy },
    { id: "format.makeHeader", label: "&Make this row the header", run: makeHeader, enabled: ready },
    { id: "data.headersStartHere", label: "Headers start here (set the rows above aside as report notes)", run: () => void headersStartHere(), enabled: () => ready() && asideWhy("head") === "", why: () => asideWhy("head") },
    { id: "data.setAsideFoot", label: "Set aside rows as footer notes (the last rows: a total line)", run: () => void setAsideFoot(), enabled: () => ready() && asideWhy("foot") === "", why: () => asideWhy("foot") },
    { id: "data.putBackNotes", label: "Put the report notes back into the table", run: () => void putBackNotes(), enabled: () => ready() && noteLines(tab) > 0 && !tab?.info.filtered && !tab?.info.read_only, why: () => (noteLines(tab) === 0 ? "this file has no report notes" : tab?.info.filtered ? "show all rows first" : tab?.info.read_only ? "the file is open read-only" : "") },
    { id: "file.keepNotes", label: "Save report notes with the file", run: () => void setKeepNotes(!(tab?.info.notes?.keep ?? true)), enabled: () => ready() && noteLines(tab) > 0, why: () => (noteLines(tab) === 0 ? "this file has no report notes" : ""), checked: () => tab?.info.notes?.keep ?? true },
    { id: "format.renameCol", label: "Re&name column…", run: () => void askRename(menuCol ?? undefined), enabled: () => ready() && !tab?.info.read_only && (tab?.info.cols ?? 0) > 0 },

    { id: "view.freeze", label: "&Freeze panes at cursor", run: freezeAtCursor, enabled: hasTab },
    { id: "view.splitRows", label: "Split top and &bottom", keys: ["Alt+Shift+W"], run: () => toggleSplit("rows"), enabled: hasTab, checked: () => tab?.split?.dir === "rows" },
    { id: "view.splitCols", label: "Split si&de by side", run: () => toggleSplit("cols"), enabled: hasTab, checked: () => tab?.split?.dir === "cols" },
    { id: "view.freezeRow", label: "Freeze top &row", run: () => { tab!.frozenRows = 1; tab!.frozenCols = 0; }, enabled: hasTab },
    { id: "view.freezeCol", label: "Freeze first &column", run: () => { tab!.frozenCols = 1; tab!.frozenRows = 0; }, enabled: hasTab },
    { id: "view.unfreeze", label: "&Unfreeze panes", run: () => { tab!.frozenRows = 0; tab!.frozenCols = 0; }, enabled: () => !!tab && (tab.frozenRows > 0 || tab.frozenCols > 0) },
    { id: "view.zoomIn", label: "Zoom &in", keys: ["Ctrl+Alt+=", "Ctrl+Alt++"], run: () => prefs.setZoom(prefs.data.zoom + 0.1) },
    { id: "view.zoomOut", label: "Zoom &out", keys: ["Ctrl+Alt+-"], run: () => prefs.setZoom(prefs.data.zoom - 0.1) },
    { id: "view.zoomReset", label: "&Zoom 100%", keys: ["Ctrl+Alt+0"], run: () => prefs.setZoom(1) },
    { id: "view.cellBar", label: "&Expand cell bar", keys: ["Ctrl+Shift+U"], run: () => { cellBarExpanded = !cellBarExpanded; }, checked: () => cellBarExpanded },
    { id: "view.gridLines", label: "&Gridlines", run: () => { prefs.data.gridLines = !prefs.data.gridLines; prefs.save(); }, checked: () => prefs.data.gridLines },
    { id: "view.problemsFill", label: "Show problems as a red &fill", run: () => { prefs.data.problemStyle = "fill"; prefs.save(); }, checked: () => prefs.data.problemStyle === "fill" },
    { id: "view.problemsUnderline", label: "Show problems as a wavy &underline", run: () => { prefs.data.problemStyle = "underline"; prefs.save(); }, checked: () => prefs.data.problemStyle === "underline" },
    { id: "view.problemsBoth", label: "Show problems as fill and underline", run: () => { prefs.data.problemStyle = "both"; prefs.save(); }, checked: () => prefs.data.problemStyle === "both" },
    { id: "view.xray", label: "&X-ray gremlins (look-alike letters, odd spaces, smart quotes)", keys: ["Ctrl+Shift+X"], run: () => { prefs.data.xray = !prefs.data.xray; prefs.save(); if (!prefs.data.xray) xrayNow = null; }, checked: () => prefs.data.xray },
    { id: "tools.encoding", label: "Change &encoding… (read the file as another encoding)", run: () => { encodingOpen = true; }, enabled: () => !!tab?.info.path && !!tab.info.indexed },
    { id: "view.xrayReport", label: "X-ray report for the selected columns or the file…", run: () => { xrayOpen = true; }, enabled: ready },
    { id: "edit.sanitize", label: "Sanitize selection (look-alike letters, odd spaces, smart quotes)…", run: () => { cleanTool = "sanitize"; }, enabled: ready },
    { id: "view.invisible", label: "Show &invisible characters", keys: ["Ctrl+Shift+H"], run: () => { prefs.data.showInvisible = !prefs.data.showInvisible; prefs.save(); }, checked: () => prefs.data.showInvisible },
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
    { id: "window.next", label: "Next file", keys: ["Ctrl+PageDown", "Ctrl+Tab", "Ctrl+Shift+PageDown"], run: () => switchTab(1), enabled: () => tabs.length > 1 },
    { id: "window.prev", label: "Previous file", keys: ["Ctrl+PageUp", "Ctrl+Shift+Tab", "Ctrl+Shift+PageUp"], run: () => switchTab(-1), enabled: () => tabs.length > 1 },
    { id: "edit.copyToTab", label: "Copy selection to another file…", run: () => void cellsToTab(false), enabled: () => ready() && tabs.length > 1 },
    { id: "edit.moveToTab", label: "Move selection to another file…", run: () => void cellsToTab(true), enabled: () => ready() && !tab?.info.read_only && tabs.length > 1 },
    { id: "window.moveToNew", label: "Move tab to a new window", run: () => void moveTabToWindow(), enabled: () => !SECOND_WINDOW && ready() },
    { id: "window.moveToMain", label: "Move tab back to the main window", run: () => void moveTabBack(), enabled: () => SECOND_WINDOW && !!tab },
    { id: "window.moveTabLeft", label: "Move file tab left", keys: ["Ctrl+Alt+Shift+PageUp"], run: () => moveTab(-1), enabled: () => active > 0 },
    { id: "window.moveTabRight", label: "Move file tab right", keys: ["Ctrl+Alt+Shift+PageDown"], run: () => moveTab(1), enabled: () => active >= 0 && active < tabs.length - 1 },
    { id: "data.sortAsc", label: "Sort &A → Z", run: () => sortByColumn(false), enabled: ready, repeatable: true },
    { id: "data.sortDesc", label: "Sort &Z → A", run: () => sortByColumn(true), enabled: ready, repeatable: true },
    { id: "data.sort", label: "&Custom sort…", run: openSortDialog, enabled: ready },
    { id: "data.filter", label: "&Filter", keys: ["Ctrl+Shift+L"], run: toggleFilterButtons, enabled: ready, checked: () => !!tab && (tab.filterButtons || tab.info.has_filter) },
    { id: "data.filterMenu", label: "Filter &menu for this column", run: () => openFilterMenu(), enabled: ready },
    { id: "edit.pickList", label: "Pick from a list of the column's values", keys: ["Alt+ArrowDown"], run: () => void altDown(), enabled: ready },
    { id: "format.hideRows", label: "Hide &rows", keys: ["Ctrl+9"], run: () => void hideRows(), enabled: ready, repeatable: true },
    { id: "format.showRows", label: "Unhide rows in the selection", keys: ["Ctrl+Shift+9", "Ctrl+Shift+("], run: () => void showRows(false), enabled: () => ready() && !!tab?.info.hidden_rows },
    { id: "format.showAllRows", label: "Unhide all ro&ws", run: () => void showRows(true), enabled: () => ready() && !!tab?.info.hidden_rows },
    { id: "format.alignLeft", label: "Align &left", multiArea: true, run: () => setAlign("left"), enabled: hasTab, checked: () => alignIs("left") },
    { id: "format.alignCenter", label: "Align &center", multiArea: true, run: () => setAlign("center"), enabled: hasTab, checked: () => alignIs("center") },
    { id: "format.alignRight", label: "Align &right", multiArea: true, run: () => setAlign("right"), enabled: hasTab, checked: () => alignIs("right") },
    { id: "format.alignAuto", label: "Align a&utomatically", multiArea: true, run: () => setAlign(null), enabled: hasTab, checked: () => alignIs(null) },
    { id: "format.clip", label: "Cut off long te&xt", multiArea: true, run: toggleClip, enabled: hasTab, checked: () => !!tab && selectedCols(tab).length > 0 && selectedCols(tab).every((c) => tab!.columns.clips.has(c)) },
    { id: "view.listArrows", label: "List arrows on every cell", run: () => { prefs.data.listArrows = !prefs.data.listArrows; prefs.save(); }, checked: () => prefs.data.listArrows },
    { id: "view.colLetters", label: "Column &letters (A, B, C)", run: () => { prefs.data.colLetters = !prefs.data.colLetters; prefs.save(); }, checked: () => prefs.data.colLetters },
    {
      id: "view.insights",
      label: "Column &insights",
      run: () => {
        prefs.data.insights = !prefs.data.insights;
        prefs.save();
        if (prefs.data.insights) say("Under each heading: how full the column is, how many different values, and a tiny chart (from a sample on big files). Click a strip for the column profile.");
      },
      checked: () => prefs.data.insights,
    },
    { id: "view.alignNumbers", label: "&Numbers on the right", run: () => { prefs.data.alignNumbers = !prefs.data.alignNumbers; prefs.save(); }, checked: () => prefs.data.alignNumbers },
    { id: "format.wrap", label: "&Wrap text", multiArea: true, run: toggleWrap, enabled: hasTab, checked: () => !!tab && selectedCols(tab).length > 0 && selectedCols(tab).every((c) => tab!.columns.wraps.has(c)) },
    { id: "format.rowHeight", label: "Row &height of every row (lines)…", run: askRowLines, enabled: hasTab },
    { id: "format.rowHeightSet", label: "Height of the selected rows (pixels)…", run: askRowsPx, enabled: hasTab },
    { id: "format.rowHeightFit", label: "Fit the selected rows to their text", run: fitSelectedRows, enabled: hasTab },
    { id: "format.rowHeightReset", label: "Reset the height of the selected rows", run: resetSelectedRows, enabled: () => !!tab && !tab.rowHeights.empty },
    { id: "format.fitRows", label: "Fit row height to wrapped text", run: () => grid?.fitRowLines(), enabled: () => !!tab && tab.columns.wraps.size > 0 },
    { id: "format.display", label: "&Display format (numbers, dates, tick boxes)…", keys: ["Ctrl+1"], run: openFormat, enabled: hasTab },
    { id: "format.asNumber", label: "Show as number (1,234.00)", multiArea: true, keys: ["Ctrl+Shift+1", "Ctrl+Shift+!"], run: () => quickFormat("number"), enabled: hasTab },
    { id: "format.asDate", label: "Show as date", multiArea: true, keys: ["Ctrl+Shift+3", "Ctrl+Shift+#"], run: () => quickFormat("date"), enabled: hasTab },
    { id: "format.asCurrency", label: "Show as currency ($1,234.00)", multiArea: true, keys: ["Ctrl+Shift+4", "Ctrl+Shift+$"], run: () => quickFormat("currency"), enabled: hasTab },
    { id: "format.asPercent", label: "Show as percent (12%)", multiArea: true, keys: ["Ctrl+Shift+5", "Ctrl+Shift+%"], run: () => quickFormat("percent"), enabled: hasTab },
    { id: "format.bakeDisplay", label: "Write the display format into the cells", run: () => void bakeFormats(), enabled: () => !!tab && !tab.busy && selectedCols(tab).some((c) => tab!.columns.formats.has(c)) },
    { id: "format.clearDisplay", label: "Remove display format", multiArea: true, keys: ["Ctrl+\\"], run: () => { if (tab) setFormat(selectedCols(tab), null); }, enabled: () => !!tab && selectedCols(tab).some((c) => tab!.columns.formats.has(c)) },
    { id: "view.banded", label: "Alternating row &colours", run: toggleBanded, enabled: hasTab, checked: () => !!tab?.banded },
    { id: "view.totalsRow", label: "Tota&ls row (under the shown rows)", run: () => { if (tab) { tab.totalsOn = !tab.totalsOn; tab.bump(); } }, enabled: hasTab, checked: () => !!tab?.totalsOn },
    { id: "view.total.sum", ...totalCommand("sum") },
    { id: "view.total.average", ...totalCommand("average") },
    { id: "view.total.count", ...totalCommand("count") },
    { id: "view.total.blank", ...totalCommand("blank") },
    { id: "view.total.min", ...totalCommand("min") },
    { id: "view.total.max", ...totalCommand("max") },
    { id: "view.total.none", ...totalCommand("none") },
    { id: "view.total.auto", label: "Totals row: Automatic", run: () => chooseTotal(null), enabled: totalsOn, checked: () => !!tab && !tab.columns.totals.has(totalColumn()) },
    { id: "view.copyTotal", label: "Copy this column's total", run: () => void copyTotals([totalColumn()]), enabled: totalsOn },
    { id: "view.copyTotals", label: "Copy all totals as a row", run: () => void copyTotals(null), enabled: totalsOn },
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
    { id: "tools.verifyReceipt", label: "&Verify a processing receipt…", run: () => { receiptOpen = true; } },
    { id: "tools.recipeFiles", label: "Run recipe on &files…", run: () => { recipeFilesOpen = true; } },
    { id: "tools.updates", label: "Take &changes from a newer file…", run: () => { updatesOpen = true; }, enabled: ready },
    { id: "tools.compare", label: "Co&mpare with another open file…", run: () => openTools("compare"), enabled: ready },
    { id: "tools.split", label: "S&plit into files…", run: () => openTools("split"), enabled: () => ready() },
    { id: "tools.combine", label: "Combine &files into one…", run: () => openTools("combine"), enabled: hasTab },
    { id: "tools.nextChange", label: "&Next change in this column", keys: ["Ctrl+Alt+PageDown"], run: () => void jumpChange(false), enabled: ready },
    { id: "tools.prevChange", label: "Pre&vious change in this column", keys: ["Ctrl+Alt+PageUp"], run: () => void jumpChange(true), enabled: ready },
    { id: "tools.largest", label: "Go to the lar&gest number", run: () => void jumpExtreme(true), enabled: ready },
    { id: "tools.smallest", label: "Go to the smallest num&ber", run: () => void jumpExtreme(false), enabled: ready },
    { id: "file.revert", label: "Re&vert to saved", run: () => void revertFile(), enabled: () => !!tab?.info.path },
    { id: "file.rename", label: "Rena&me…", run: () => void renameFile(), enabled: () => !!tab && !tab.info.read_only },
    { id: "file.showFolder", label: "Show in File E&xplorer", run: () => void showInFolder(), enabled: () => !!tab?.info.path },
    { id: "file.copyPath", label: "Cop&y file path", run: () => void copyPath(), enabled: () => !!tab?.info.path },
    { id: "file.saveAll", label: "Save a&ll", keys: ["Ctrl+Alt+S"], run: () => void saveAll(), enabled: () => tabs.some((t) => t.info.modified) },
    { id: "file.closeOthers", label: "Close other tabs", run: () => void closeOthers(false), enabled: () => tabs.length > 1 },
    { id: "file.closeRight", label: "Close tabs to the right", run: () => void closeOthers(true), enabled: () => active < tabs.length - 1 },
    { id: "file.reopenClosed", label: "Reopen closed file", keys: ["Ctrl+Shift+T"], run: () => void reopenClosed() },
    { id: "file.closeUnchanged", label: "Close &unchanged tabs", run: () => void closeUnchanged(), enabled: () => tabs.length > 0 },
    { id: "view.valueTypes", label: "Show value t&ypes (numbers, dates)", keys: ["Ctrl+F8"], run: toggleValueTypes, enabled: hasTab, checked: () => !!tab?.valueTypes },
    { id: "view.divider", label: "&Line at each change in this column", run: toggleDivider, enabled: hasTab, checked: () => tab?.divider !== null && tab?.divider === tab?.cell.col },
    { id: "view.scale3", label: "Colour scale (red, yellow, green)", run: () => toggleScale("scale3"), enabled: ready, checked: () => !!tab?.scales.some((s) => s.col === tab?.cell.col && s.kind === "scale3") },
    { id: "view.scale2", label: "Colour scale (white to green)", run: () => toggleScale("scale2"), enabled: ready, checked: () => !!tab?.scales.some((s) => s.col === tab?.cell.col && s.kind === "scale2") },
    { id: "view.bars", label: "Data &bars", run: () => toggleScale("bar"), enabled: ready, checked: () => !!tab?.scales.some((s) => s.col === tab?.cell.col && s.kind === "bar") },
    { id: "view.clearHighlights", label: "Clear all highlight rules", run: clearHighlights, enabled: hasTab },
    { id: "data.clearChecks", label: "Clear all data checks", run: clearChecks, enabled: () => !!tab?.checks.length },
    { id: "view.typeahead", label: "Suggest values while typing", run: () => { prefs.data.typeahead = !prefs.data.typeahead; prefs.save(); say(prefs.data.typeahead ? "Values already in the column are suggested while you type." : "No suggestions while typing."); }, checked: () => prefs.data.typeahead },
    { id: "clean.suggest", label: "Clean-up &suggestions…", run: () => void openSuggestions(), enabled: ready },
    { id: "file.recover", label: "Recover unsaved &work…", run: () => void showRecovery(false) },
    { id: "file.versions", label: "Earlier sa&ves of this file…", run: () => void showVersions(), enabled: () => !!tab?.info.path },
    { id: "file.checkDisk", label: "Check open files for o&utside changes", run: () => void checkDisk(true), enabled: hasTab },
    { id: "data.filterBySel", label: "Filter by &selected value", run: filterBySelection, enabled: ready },
    { id: "data.clearFilter", label: "C&lear filter", run: clearFilter, enabled: () => ready() && !!tab?.info.has_filter },
    { id: "data.reapply", label: "&Reapply filter", keys: ["Ctrl+Alt+L"], run: reapplyFilter, enabled: () => ready() && !!tab?.filter },
    { id: "data.removeDuplicates", label: "Remove &duplicates…", run: () => openOp("remove_duplicates"), enabled: ready },
    { id: "data.patternOdd", label: "Show rows that break the column's pattern", run: () => void showShape(patternCol(), true, false), enabled: () => ready() && !!patternShape(patternCol()) },
    { id: "data.patternOddBlanks", label: "Show rows that break the pattern, and blank ones", run: () => void showShape(patternCol(), true, true), enabled: () => ready() && !!patternShape(patternCol()) },
    { id: "data.patternMatch", label: "Show rows that match the column's pattern", run: () => void showShape(patternCol(), false, false), enabled: () => ready() && !!patternShape(patternCol()) },
    { id: "data.patternCheck", label: "Check this pattern (add a rule)", run: () => checkPattern(patternCol()), enabled: () => !!patternShape(patternCol()) },
    { id: "data.patternCopy", label: "Copy the column's pattern", run: () => void copyPattern(patternCol()), enabled: () => !!patternShape(patternCol()) },
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
    { id: "data.pivotRefresh", label: "Refres&h pivot table", keys: ["Alt+F5"], run: () => void pivotRefresh(), enabled: () => ready() && !!tab && pivotOf.has(tab.id) },
    { id: "data.unpivot", label: "U&npivot…", run: () => openOp("unpivot"), enabled: ready },
    { id: "data.join", label: "&Join with another table…", run: () => { joinKind = "join"; }, enabled: ready },
    { id: "data.street", label: "Look up by &street range (precinct, district)…", run: () => { streetOpen = true; }, enabled: ready },
    { id: "data.match", label: "&Match people (spelled differently)…", run: () => { matchOpen = true; }, enabled: ready },
    { id: "data.values", label: "Edit &values (rename, merge, translate)…", run: () => { valuesMethod = "letters"; valuesOpen = true; }, enabled: ready },
    { id: "data.valuesLookalikes", label: "Group look-alike values by sound or spelling…", run: () => { valuesMethod = "both"; valuesOpen = true; }, enabled: ready },
    { id: "data.group", label: "&Group figures (count, total, rank)…", run: () => { editKind = "group"; }, enabled: ready },
    { id: "data.arrange", label: "Choose and &order columns…", run: () => { editKind = "arrange"; }, enabled: ready },
    { id: "data.extract", label: "E&xtract into a column…", run: () => { editKind = "extract"; }, enabled: ready },
    { id: "data.build", label: "&Build a column (template or rules)…", run: () => { editKind = "build"; }, enabled: ready },
    { id: "data.append", label: "Appen&d another table…", run: () => { joinKind = "append"; }, enabled: ready },
    { id: "data.form", label: "Form v&iew (one row per page)…", run: () => { if (tab) formRow = tab.cell.row; }, enabled: ready },
    { id: "data.fillExample", label: "Fill by e&xample", keys: ["Ctrl+E"], run: () => void fillByExample(), enabled: ready, repeatable: true },
    { id: "data.splitName", label: "Split names into columns…", run: () => { splitNameOpen = true; }, enabled: ready },
    { id: "clean.tools", label: "&Cleaning tools…", keys: ["Ctrl+Shift+K"], run: () => { cleanTool = "spaces"; }, enabled: ready },
    { id: "clean.matrix", label: "Clean matrix…", run: openMatrix, enabled: ready },
    { id: "clean.matrixHere", label: "Clean matrix (these columns)…", run: openMatrixHere, enabled: ready },
    ...TOOLS.map((tl) => ({ id: `clean.tool.${tl.id}`, label: `${tl.label}…`, run: () => { cleanTool = tl.id; }, enabled: ready })),
    { id: "find.find", label: "&Find…", multiArea: true, keys: ["Ctrl+F", "Shift+F5"], run: () => openFind(false), enabled: hasTab },
    { id: "find.replace", label: "&Replace…", multiArea: true, keys: ["Ctrl+H"], run: () => openFind(true), enabled: ready },
    { id: "find.batch", label: "Replace from a &list…", run: () => { batchOpen = true; }, enabled: ready },
    { id: "find.next", label: "Find &next", multiArea: true, keys: ["F3", "Shift+F4"], run: () => findAgain(false), enabled: hasTab },
    { id: "find.prev", label: "Find &previous", multiArea: true, keys: ["Shift+F3"], run: () => findAgain(true), enabled: hasTab },
    { id: "find.ghosts", label: "Show &changes in the grid (Replace)", run: () => { prefs.data.showGhosts = !prefs.data.showGhosts; prefs.save(); say(prefs.data.showGhosts ? "The cells Replace all would change show their new text in green while you type the replacement." : "Changes are not shown in the grid."); }, checked: () => prefs.data.showGhosts, enabled: hasTab },
    { id: "find.thisValue", label: "Find this &value", run: () => findValue("find"), enabled: hasTab },
    { id: "find.replaceValue", label: "Replace this val&ue…", run: () => findValue("replace"), enabled: ready },
    { id: "find.allInColumn", label: "Find all &like this in the column", run: () => findValue("column"), enabled: hasTab },
    { id: "view.panel", label: "Side &panel", keys: ["Ctrl+F1"], run: () => showPanel(prefs.data.panel ? null : panelTab), checked: () => !!prefs.data.panel },
    { id: "view.panelSummary", label: "Column summary", run: () => showPanel("summary"), checked: () => prefs.data.panel === "summary" },
    { id: "view.panelFilters", label: "Filter panel", keys: ["Alt+Shift+L"], run: () => showPanel("filters"), checked: () => prefs.data.panel === "filters" },
    { id: "view.panelRecord", label: "&Record panel (this row as a list of fields)", keys: ["Alt+R"], run: () => showPanel(prefs.data.panel === "record" ? null : "record"), checked: () => prefs.data.panel === "record" },
    { id: "view.panelNotes", label: "Report notes (lines above and below the table)", run: () => showPanel(prefs.data.panel === "notes" ? null : "notes"), checked: () => prefs.data.panel === "notes" },
    { id: "view.recordNext", label: "Record: next row", keys: ["Ctrl+Alt+ArrowDown"], run: () => recordFlip(1), enabled: hasTab },
    { id: "view.recordPrev", label: "Record: previous row", keys: ["Ctrl+Alt+ArrowUp"], run: () => recordFlip(-1), enabled: hasTab },
    { id: "view.panelSteps", label: "Recipe steps", keys: ["Alt+Shift+R"], run: () => showPanel("steps"), checked: () => prefs.data.panel === "steps" },
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
    { id: "help.search", label: "&Search commands…", keys: ["Alt+Q", "Alt+/"], run: () => { search = ""; } },
    { id: "help.shortcuts", label: "&Customize shortcuts…", run: () => { shortcutsOpen = true; } },
    { id: "help.keys", label: "&Keyboard shortcuts", keys: ["Ctrl+/", "F1"], run: () => { keysMouse = false; keysOpen = true; } },
    { id: "help.mouse", label: "&Mouse and drag moves", run: () => { keysMouse = true; keysOpen = true; } },
    { id: "help.about", label: "&About Natendo Data Edit", run: showAbout },
  ];
  reg.add(cmds);
  // Custom keys (taken from the commands' defaults, which are Excel's).
  reg.setOverrides(cleanShortcuts(prefs.data.shortcuts, (id) => !!reg.get(id)));

  const menus: Menu[] = [
    { label: "&File", items: ["file.new", "file.open", "file.recent", "-", "file.save", "file.saveAs", "file.saveAll", "-", "file.import", "file.importClipboard", "file.importNotes", "file.export", "file.safeCopy", "-", "file.openEncrypted", "file.saveEncrypted", "file.openSqlite", "file.saveSqlite", "-", "file.print", "file.pdf", "file.mail", "-", "file.layout", "file.reopenLines", "file.readOnly", "file.private", "file.privateFolder", "file.revert", "-", "file.rename", "file.showFolder", "file.copyPath", "file.checkDisk", "-", "file.recover", "file.versions", "-", "window.moveToNew", "window.moveToMain", "-", "file.options", "-", "file.close", "file.closeOthers", "file.closeUnchanged", "file.reopenClosed", "file.quit"] },
    {
      label: "&Edit",
      items: [
        "edit.undo", "edit.redo", "edit.repeat", "-", "edit.cut", "edit.copy", "edit.paste", "edit.moveCut", "edit.cancelCut", "edit.pasteSpecial", "edit.copyToTab", "edit.moveToTab", "edit.clear", "edit.sanitize", "-",
        "edit.copyNames", "edit.copyNamesText", "edit.copyWithNames", "edit.pasteNames", "-",
        "edit.fillDown", "edit.fillRight", "edit.fillUp", "edit.fillLeft", "edit.fillSeries", "-", "edit.cell", "edit.today", "edit.pickDate", "edit.now", "-", "edit.selectAll", "edit.goto", "edit.gotoSpecial", "-", "edit.note", "edit.changes", "edit.checkpoint", "edit.sinceCheckpoint", "-",
        "edit.deleteRows", "edit.deleteCols", "edit.moveRowsUp", "edit.moveRowsDown", "edit.moveColsLeft", "edit.moveColsRight",
      ],
    },
    {
      label: "&View",
      items: [
        "view.freeze", "view.freezeRow", "view.freezeCol", "view.unfreeze", "-", "view.splitRows", "view.splitCols", "-", "view.zoomIn", "view.zoomOut", "view.zoomReset", "-",
        "view.panel", "view.panelSummary", "view.panelFilters", "view.panelSteps", "view.panelRecord", "view.panelNotes", "view.recordPrev", "view.recordNext", "view.panelChart", "view.panelFiles", "view.panelActivity", "-",
        "view.showEdits", "view.highlights", "view.scale3", "view.scale2", "view.bars", "view.clearHighlights", "-",
        "view.saveView", "view.applyView", "view.saveViewFile", "view.openViewFile", "-",
        "view.cellBar", "view.gridLines", "view.problemsFill", "view.problemsUnderline", "view.problemsBoth", "view.invisible", "view.xray", "view.xrayReport", "view.whichRows", "view.colLetters", "view.insights", "view.alignNumbers", "view.listArrows", "view.totalsRow", "view.banded", "view.valueTypes", "view.divider", "view.typeahead", "view.spellcheck", "view.fontBigger", "view.fontSmaller", "-", "view.themeSystem", "view.themeLight", "view.themeDark", "-",
        ...ACCENTS.map((a) => `view.accent.${a.name.toLowerCase()}`), "-", ...FONTS.map((f) => `view.font.${f.name.toLowerCase()}`), "view.fontOther",
      ],
    },
    { label: "&Insert", items: ["insert.rowsAbove", "insert.rowsBelow", "insert.rowsMany", "-", "insert.colsLeft", "insert.colsRight", "insert.colsMany", "-", "insert.symbol"] },
    { label: "F&ormat", items: ["col.typeAuto", "col.typeText", "col.typeNumber", "col.typeDate", "col.typeDateDmy", "col.typeDateMdy", "col.typeId", "format.typesAnyway", "-", "format.header", "format.makeHeader", "format.renameCol", "format.tidyTitles", "format.askHeaderAgain", "format.forgetHeader", "format.forgetHeaders", "format.askHeader", "-", "format.display", "format.bakeDisplay", "format.clearDisplay", "-", "format.alignLeft", "format.alignCenter", "format.alignRight", "format.alignAuto", "format.wrap", "format.clip", "format.rowHeight", "format.rowHeightSet", "format.rowHeightFit", "format.rowHeightReset", "format.fitRows", "-", "format.autofit", "format.colWidth", "format.hideCols", "format.showCols", "format.showAllCols", "-", "format.protectCols", "-", "format.hideRows", "format.showRows", "format.showAllRows"] },
    {
      label: "&Data",
      items: [
        "data.sortAsc", "data.sortDesc", "data.sort", "data.sortColsAsc", "data.sortColsDesc", "data.shuffle", "-",
        "data.filter", "data.filterMenu", "data.filterBySel", "data.filterFormula", "view.panelFilters", "data.clearFilter", "data.reapply", "-",
        "data.removeDuplicates", "data.showDuplicates", "data.removeBlankRows", "data.removeBlankCols", "data.repairRow", "data.ragged", "data.raggedShort", "data.raggedLong", "-",
        "data.headersStartHere", "data.setAsideFoot", "data.putBackNotes", "file.keepNotes", "view.panelNotes", "-",
        "data.splitColumn", "data.mergeColumns", "data.splitRows", "data.mergeRows", "-",
        "data.pivot", "data.pivotRefresh", "data.unpivot", "data.subtotals", "data.form", "-",
        "data.counts", "data.profile", "data.checkDates", "data.reviewDates", "data.nextDate", "data.prevDate", "data.dateMarksOff", "data.compute", "data.changeFormula", "data.checks", "data.uniqueTogether", "data.existsCheck", "data.tableChecks", "data.typeRule", "data.fieldCheck", "data.diffNext", "data.diffPrev", "data.diffClear", "data.nextProblem", "data.prevProblem", "data.checkNow", "data.unmarkChecks", "data.clearChecks", "-",
        "data.fillExample", "data.splitName", "-",
        "data.join", "data.match", "data.street", "data.append", "data.compareCols", "data.compareLists", "data.gaps", "data.relations", "data.patchSave", "data.patchApply", "-",
        "data.values", "data.valuesLookalikes", "data.group", "data.arrange", "data.extract", "data.build", "-",
        "find.find", "find.replace", "find.batch", "find.next", "find.prev", "find.ghosts",
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
        "tools.compare", "tools.updates", "tools.split", "tools.combine", "tools.searchFolder", "tools.recipe", "tools.recipeFiles", "tools.watch", "tools.verifyReceipt", "tools.encoding", "-",
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
    totals: [...TOTAL_CHOICES.map(([c]) => `view.total.${c}`), "view.total.auto", "-", "view.copyTotal", "view.copyTotals", "-", "view.totalsRow"],
    tab: ["file.save", "file.saveAs", "-", "data.compareLists", "data.patchSave", "data.patchApply", "tools.encoding", "edit.checkpoint", "edit.sinceCheckpoint", "data.pivotRefresh", "view.panelSteps", "tools.recipe", "tools.recipeFiles", "-", "file.rename", "file.copyPath", "file.showFolder", "format.forgetHeader", "-", "window.moveTabLeft", "window.moveTabRight", "window.moveToNew", "window.moveToMain", "-", "file.close", "file.closeOthers", "file.closeRight", "file.closeUnchanged", "file.reopenClosed"],
    cell: ["data.countsShow", "data.countsHide", "-", "edit.cut", "edit.copy", "edit.copyWithNames", "edit.paste", "edit.pasteNames", "edit.moveCut", "edit.cancelCut", "edit.pasteSpecial", "edit.copyToTab", "edit.moveToTab", "edit.clear", "-", "edit.cell", "edit.pickList", "edit.note", "edit.gotoSpecial", "view.panelRecord", "data.repairRow", "data.checkDates", "data.reviewDates", "data.nextDate", "data.dateMarksOff", "edit.checkpoint", "edit.sinceCheckpoint", "-", "find.thisValue", "find.replaceValue", "find.allInColumn", "find.ghosts", "-", "data.filterBySel", "data.sortAsc", "data.sortDesc", "data.gaps", "clean.matrixHere", "view.totalsRow", "-", "insert.cellsAsk", "edit.deleteCellsAsk", "-", "insert.rowsAbove", "insert.rowsBelow", "edit.deleteRows", "-", "insert.colsLeft", "insert.colsRight", "edit.deleteCols", "-", "format.hideRows", "format.hideCols", "-", "edit.fillDown", "edit.fillRight", "-", "view.showEdits", "edit.changes", "-", "data.nextProblem", "data.diffNext", "data.diffPrev", "data.diffClear"],
    row: ["edit.cut", "edit.copy", "edit.paste", "edit.moveCut", "edit.cancelCut", "-", "view.panelRecord", "-", "insert.rowsAbove", "insert.rowsBelow", "insert.rowsMany", "edit.deleteRows", "-", "edit.moveRowsUp", "edit.moveRowsDown", "-", "format.hideRows", "format.showRows", "-", "format.rowHeightSet", "format.rowHeightFit", "format.rowHeightReset", "-", "data.shuffle", "data.repairRow", "data.ragged", "-", "format.makeHeader", "data.headersStartHere", "data.setAsideFoot", "-", "edit.clear"],
    type: ["col.typeAuto", "col.typeText", "col.typeNumber", "col.typeDate", "col.typeDateDmy", "col.typeDateMdy", "col.typeId"],
    col: ["format.renameCol", "format.tidyTitles", "format.askHeaderAgain", "-", "edit.copyNames", "edit.pasteNames", "-", "edit.cut", "edit.copy", "edit.paste", "edit.moveCut", "edit.cancelCut", "-", "col.typeAuto", "col.typeText", "col.typeNumber", "col.typeDate", "col.typeDateDmy", "col.typeDateMdy", "col.typeId", "-", "data.sortAsc", "data.sortDesc", "data.filterMenu", "data.filterFormula", "data.counts", "data.profile", "data.checkDates", "data.reviewDates", "data.gaps", "clean.matrixHere", "data.values", "data.valuesLookalikes", "-", "data.changeFormula", "-", "insert.colsLeft", "insert.colsRight", "insert.colsMany", "edit.deleteCols", "-", "edit.moveColsLeft", "edit.moveColsRight", "-", "format.display", "format.wrap", "format.autofit", "format.colWidth", "format.hideCols", "format.showCols", "-", "format.protectCols", "data.compareCols", "data.compareLists", "data.relations", "data.uniqueTogether", "data.existsCheck", "data.tableChecks", "-", "view.panelSummary", "-", "edit.clear"],
    pill: ["data.patternMatch", "data.patternOdd", "data.patternOddBlanks", "-", "data.patternCheck", "data.patternCopy"],
    corner: ["edit.selectAll", "edit.copy", "edit.copyNames", "edit.copyWithNames", "edit.pasteNames", "-", "format.header", "format.askHeaderAgain", "format.forgetHeader", "format.showAllCols"],
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

  /** Right-click menu items that say how many, and which (as in Google
   *  Sheets): "Insert 3 rows above", "Delete rows 4–6". */
  function contextLabel(id: string): string | null {
    const t = tab;
    if (!t) return null;
    const r = t.singleRect();
    if (!r) return null;
    const nRows = r.rows === 1 ? "1 row" : `${r.rows.toLocaleString()} rows`;
    // Row numbers as the headings show them; just how many when one is not
    // known yet (or a filter makes them non-consecutive).
    const a = shownRowNo(t, r.row);
    const z = shownRowNo(t, r.row + r.rows - 1);
    const rows = a === null || z === null || z - a !== r.rows - 1 ? nRows : r.rows === 1 ? `row ${a.toLocaleString()}` : `rows ${a.toLocaleString()}–${z.toLocaleString()}`;
    const colName = (c: number) => (prefs.data.colLetters ? colLetters(c) : `“${columnTitle(t, c)}”`);
    const cols = r.cols === 1 ? `column ${colName(r.col)}` : `columns ${colName(r.col)}–${colName(r.col + r.cols - 1)}`;
    const nCols = r.cols === 1 ? "1 column" : `${r.cols.toLocaleString()} columns`;
    switch (id) {
      case "insert.rowsAbove":
        return `Insert ${nRows} above`;
      case "insert.rowsBelow":
        return `Insert ${nRows} below`;
      case "edit.deleteRows":
        return `Delete ${rows}`;
      case "insert.colsLeft":
        return `Insert ${nCols} left`;
      case "insert.colsRight":
        return `Insert ${nCols} right`;
      case "edit.deleteCols":
        return `Delete ${cols}`;
      case "format.hideRows":
        return `Hide ${rows}`;
      case "format.hideCols":
        return `Hide ${cols}`;
      default:
        return null;
    }
  }

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
    // Picked from a heading's right-click menu: the column it was opened on.
    menuCol = context ? contextCol : null;
    openMenu = null;
    context = null;
    try {
      await reg.run(id);
    } catch (e) {
      say(errorText(e));
    } finally {
      menuCol = null;
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

  const SAFE_IN_FIELDS = /^(file\.|view\.zoom|view\.record|window\.|help\.|find\.)/;

  function menuKey(e: KeyboardEvent): boolean {
    const list = openMenu !== null ? menus[openMenu]?.items : context?.items;
    if (!list) return false;
    const items = list.filter((x) => x !== "-");
    const idx = openMenu !== null ? menuIndex : (context?.index ?? 0);
    const setIdx = (i: number) => {
      // Up from "nothing picked yet" goes to the last item.
      const n = idx < 0 && i < 0 ? items.length - 1 : (i + items.length) % items.length;
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
    if (cleanTool || splitNameOpen || insertAsk || search !== null || shortcutsOpen || highlightOpen || subtotalOpen || batchOpen || pivotDoc !== null || prompt) return;
    if ((openMenu !== null || context) && menuKey(e)) return;
    altAlone = e.key === "Alt";
    // The find bar's own keys (Alt+C, Alt+W, Alt+R, Alt+L, Alt+Enter) are
    // not menu keys.
    if (e.defaultPrevented && (e.target as HTMLElement | null)?.closest?.(".find")) return;
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

    // F8 (extend mode): arrows grow the selection as if Shift were held.
    const extend = e.shiftKey || extendMode;
    const last = Math.max(t.firstRow, t.info.rows - 1);
    const lastCol = Math.max(0, t.info.cols - 1);
    const f = extend ? t.sel.focus : t.cell;
    const to = (row: number, col: number) => {
      if (extend) grid!.select(t.sel.anchor, { row, col }, true, t.sel.active);
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
        if (altOf(e)) grid.move(0, grid.pageCols(), extend);
        else grid.move(grid.pageRows(), 0, extend);
        return;
      case "PageUp":
        e.preventDefault();
        if (altOf(e)) grid.move(0, -grid.pageCols(), extend);
        else grid.move(-grid.pageRows(), 0, extend);
        return;
      case "Home": {
        e.preventDefault();
        // Hidden columns are skipped (as in Excel).
        const first = Math.max(0, t.columns.firstVisible(0));
        if (e.ctrlKey) to(t.firstRow, first);
        else to(f.row, first);
        return;
      }
      case "End": {
        e.preventDefault();
        const end = t.columns.width(lastCol) > 0 ? lastCol : t.columns.step(lastCol, -1);
        if (e.ctrlKey) to(last, end);
        else to(f.row, end);
        return;
      }
      case "Enter":
        e.preventDefault();
        // In a counts tab, Enter shows the selected lines' rows.
        if (t.countsOf && !e.shiftKey && !e.ctrlKey) {
          void countsShow(false);
          return;
        }
        grid.advance(...enterStep(prefs.data.enterMove, e.shiftKey));
        return;
      case "Tab":
        e.preventDefault();
        grid.advance(0, e.shiftKey ? -1 : 1);
        return;
      case "Backspace":
        e.preventDefault();
        void grid.startEdit("");
        return;
      case "Escape":
        // Several blocks (Ctrl+click): Esc goes back to the last block alone.
        if (t.extra.length) {
          t.extra = [];
          return;
        }
        // Esc drops a cut or the copy outline and leaves extend mode; the
        // selection stays (as in Excel and Google Sheets).
        // With nothing of that to cancel, it closes the title-row question.
        if (!t.cut && !t.copied && !extendMode && closeHeaderAsk()) return;
        if (t.cut) cancelCut();
        else cutEpoch++;
        if (t.copied) {
          t.copied = null;
          t.bump();
        }
        if (extendMode) toggleExtend();
        return;
    }
    if (key === "Space") {
      // Space on a tick box ticks or unticks it (as in Google Sheets).
      const c = t.cell;
      if (t.columns.formats.get(c.col)?.kind === "checkbox" && !(t.header && c.row === 0)) {
        e.preventDefault();
        void grid.toggleActiveTick();
        return;
      }
    }
    if (key === "Shift+Space") {
      e.preventDefault();
      // Whole rows; the active cell stays where it is (as in a spreadsheet).
      grid.select({ row: t.sel.anchor.row, col: 0 }, { row: t.sel.focus.row, col: lastCol }, false, t.cell);
      return;
    }
    if (key === "Ctrl+Space") {
      e.preventDefault();
      grid.select({ row: t.firstRow, col: t.sel.anchor.col }, { row: last, col: t.sel.focus.col }, false, t.cell);
      return;
    }
    // Typed characters reach the grid's hidden text box and start editing.
  }

  /** After each click or key: a change to the look is one undo step. */
  function lookBoundary(): void {
    const t = tab;
    if (t) setTimeout(() => t.recordView(), 0);
  }

  function onKeyUp(e: KeyboardEvent): void {
    lookBoundary();
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
      get backend() {
        return backend();
      },
      /** Tests: the command registry (every command, run by id). */
      get reg() {
        return reg;
      },
      get tabs() {
        return tabs;
      },
      /** Tests: the zip picker for a list of tables. */
      showZip(list: ZipList) {
        zipPick = list;
      },
      /** Tests: switch to tab `i`. */
      activate(i: number) {
        if (i >= 0 && i < tabs.length) active = i;
      },
      /** Tests: as at a new start, before files from Windows arrive. */
      resetStart() {
        sessionReady = false;
        launchTab = null;
      },
      /** Tests: run the start-up offer. */
      startSession() {
        return startSession();
      },
    };
  }

  // ---- computed columns, data checks --------------------------------------------
  async function runCompute(spec: ComputeSpec): Promise<void> {
    computeOpen = false;
    const t = tab;
    if (computeMode === "filter") {
      const rep = await runOp({ op: "formula_filter", spec }, "Filtering");
      if (!rep || !t) return;
      const errs = rep.errors ? ` ${plural(rep.errors, "row")} gave an error and ${rep.errors === 1 ? "is" : "are"} hidden.` : "";
      say(rep.rows ? `Showing ${plural(rep.rows, "row")} where the formula is TRUE.${errs} Data › Clear filter shows every row.` : `The formula is TRUE in no shown row, so nothing is shown but the titles.${errs} Data › Clear filter or Ctrl+Z brings them back.`);
      void tick().then(() => grid?.focus());
      return;
    }
    const rep = await runOp({ op: "compute", spec }, "Computing");
    if (!rep || !t) return;
    if (spec.target === null && spec.at !== null) t.shiftCols(spec.at, 1);
    if (spec.target !== null) {
      const kept = rep.errors ? ` ${plural(rep.errors, "cell")} gave an error and kept ${rep.errors === 1 ? "its" : "their"} value.` : "";
      say(`Changed ${plural(rep.cells, "cell")} in the shown rows.${kept} Undo (Ctrl+Z) takes it back.`);
      void tick().then(() => grid?.focus());
      return;
    }
    const errs = rep.errors ? ` ${plural(rep.errors, "cell")} came out as errors (like #VALUE!).` : "";
    say(`Computed ${plural(rep.cells, "cell")}.${errs} The values are saved as text. Undo (Ctrl+Z) takes it back.`);
    void tick().then(() => grid?.focus());
  }

  function keepChecks(checks: Check[], mark: boolean, tableChecks?: TableCheck[]): void {
    const t = tab;
    if (!t) return;
    t.checks = checks;
    if (tableChecks) t.tableChecks = tableChecks;
    t.checksMarked = mark && checks.length > 0;
    t.checkVersion++;
    t.bump();
  }

  /** The rules as a spec (what the engine runs). */
  function ruleSpec(t: Tab): CheckSpec {
    return { checks: $state.snapshot(t.checks) as Check[], keep_first: t.header, read: READ, now: localDays(), table_checks: $state.snapshot(t.tableChecks) as TableCheck[] };
  }

  /** Run the rules over the whole table once (a job with progress and Cancel) and keep the figures. */
  async function checkNow(t: Tab): Promise<boolean> {
    if (!t.checks.length && !t.tableChecks.length) return false;
    try {
      const rep = await runJob("Checking", () => backend().runChecks(t.id, ruleSpec(t)));
      if (!rep) return false;
      const table = rep.table ?? [];
      const failed = table.filter((r) => r.ok !== true);
      t.lastRun = { rules: t.checks.length, problems: rep.problems, revision: t.info.data_revision, tableFailed: failed.length };
      t.checkVersion++;
      t.bump();
      // A "value must exist in" rule whose file is not open did not run: say so, never a clean count alone.
      const skipped = rep.not_run ?? [];
      if (skipped.length) say(`${skipped.length === 1 ? "1 rule" : `${skipped.length} rules`} did not run: ${skipped[0]!.why}.`);
      else if (failed.length) {
        const f = failed[0]!;
        say(`${plural(failed.length, "table check")} did not pass: ${f.name}: ${f.ok === null ? `did not run (${f.why ?? ""})` : `${f.actual}, wanted ${f.expected}`}.`);
      } else if (table.length) say(`${plural(table.length, "table check")} passed; ${plural(rep.problems, "problem")} in the rows.`);
      return true;
    } catch (e) {
      say(errorText(e));
      return false;
    }
  }

  /** F7 / Shift+F7: the next (previous) problem cell, in the order the table is shown. Problems in hidden
   *  columns are passed over and counted in the message. Rules with a Unique check need the last full run to
   *  be current: if it is not, it runs once first, and the jump then uses that result even if the data moved on. */
  async function jumpProblem(backward: boolean): Promise<void> {
    const t = tab;
    if (!t) return;
    if (!t.checks.length) {
      say("There are no rules yet. Add some in Data checks.");
      checksOpen = true;
      return;
    }
    if (!t.checksMarked) {
      t.checksMarked = true;
      t.checkVersion++;
    }
    const stale = !t.lastRun || t.lastRun.revision !== t.info.data_revision;
    if (stale && t.checks.some((c) => c.kind.type === "unique")) await checkNow(t);
    try {
      const r = await runJob("Looking for the next problem", () => backend().nextProblem(t.id, t.cell.row, t.cell.col, backward, hiddenColsOf(t), ruleSpec(t)));
      if (!r) return;
      const rule = r.hit ? describeCheck(t.checks[r.hit.check] ?? t.checks[0]!) + (r.hit.row_rule ? " (row rule)" : "") : null;
      const notCurrent = t.lastRun && t.lastRun.revision !== t.info.data_revision ? " (not current)" : "";
      say(jumpText(r, rule) + notCurrent);
      if (r.hit) grid?.select({ row: r.hit.row, col: r.hit.col });
    } catch (e) {
      say(errorText(e));
    }
  }

  /** Alt+F7 / Alt+Shift+F7: the next (previous) difference of a compared tab, wrapping round. */
  async function jumpDiff(backward: boolean): Promise<void> {
    const t = tab;
    if (!t || !t.diffOn) return;
    if (t.diffOutOfDate) {
      say("The differences are out of date: the tab was edited after the compare. Compare again (Tools > Compare).");
      return;
    }
    try {
      const j = await runJob("Looking for the next difference", () => backend().diffNext(t.id, t.cell.row, backward));
      if (j === null) return;
      if (!j) {
        say("No differences found in this tab.");
        return;
      }
      grid?.select({ row: j.row, col: j.col ?? t.cell.col });
      say(j.wrapped ? "Back at the top: the first difference." : "Difference");
    } catch (e) {
      say(errorText(e));
    }
  }

  /** Take the differences off the compared tabs. */
  async function clearDiff(): Promise<void> {
    const t = tab;
    if (!t) return;
    try {
      await backend().diffClear(t.id);
    } catch {
      // Closed meanwhile.
    }
    for (const x of tabs) {
      if (x.diffOn) {
        x.diffOn = false;
        x.diffVersion++;
        x.bump();
      }
    }
    say("The differences are cleared.");
  }

  /** Check this pattern (pill menu): a rule that every value in the column must have the column's shape. */
  function checkPattern(col: number): void {
    const t = tab;
    const s = patternShape(col);
    if (!t || !s) return;
    if (s.endsWith("…")) {
      say("That pattern is cut short (long values), so it cannot be made a rule.");
      return;
    }
    const rule: Check = { col, name: `Matches ${s}`, kind: { type: "pattern", pattern: { text: shapeRegex(s), kind: "regex", match_case: true } as never } };
    keepChecks([...t.checks, rule], true);
    t.lastRun = null;
    say(`Added the rule “Matches ${s}”: cells in this column that are not written that way are marked. F7 jumps to the next one.`);
  }

  /** The rule the active cell breaks, for the status bar ("" when none). */
  function problemNote(t: Tab): string {
    void t.problemsVersion;
    if (!t.problems.size) return "";
    const own = t.problems.get(`${t.cell.row}:${t.cell.col}`);
    const i = own ?? t.problems.get(`${t.cell.row}:*`);
    const c = i === undefined ? undefined : t.checks[i];
    return c ? `Rule: ${describeCheck(c)}${own === undefined ? " (row rule)" : ""}` : "";
  }

  /** Rules kept beside the file load when it opens (never run on their own: "Check now" does that). */
  async function loadSidecar(t: Tab): Promise<void> {
    try {
      const s = await backend().checksSidecarLoad(t.id);
      if (!s || (!s.checks.length && !s.table_checks?.length) || t.checks.length || t.tableChecks.length) return;
      t.checks = s.checks;
      t.tableChecks = s.table_checks ?? [];
      t.checksMarked = true;
      t.checkVersion++;
      t.rulesNote = `Loaded ${s.checks.length === 1 ? "1 rule" : `${s.checks.length} rules`} from ${s.file}${s.skipped ? ` (${s.skipped} did not fit this table)` : ""}`;
      t.bump();
      say(t.rulesNote);
    } catch (e) {
      say(`The rules file beside this file was not used: ${errorText(e)}`);
    }
  }

  /** The column a pattern command acts on: the one whose pill menu it was picked from, else the active column. */
  const patternCol = (): number => menuCol ?? tab?.cell.col ?? 0;
  /** The dominant shape the strip shows for a column (null: none, or the figures are for rows that have changed). */
  function patternShape(col: number): string | null {
    const t = tab;
    const ins = t?.insights;
    if (!t || !ins || (ins.revision !== undefined && t.info.data_revision !== undefined && ins.revision !== t.info.data_revision)) return null;
    return ins.byCol.get(col)?.top_shape ?? null;
  }
  async function copyPattern(col: number): Promise<void> {
    const s = patternShape(col);
    if (!s) return;
    try {
      await navigator.clipboard.writeText(s);
      say(`Copied the pattern ${s}.`);
    } catch {
      say("The clipboard could not be used.");
    }
  }
  /** The pattern pill's click: show the rows that break (or match) the column's dominant pattern, among the rows shown now. */
  async function showShape(col: number, negate: boolean, includeBlanks: boolean): Promise<void> {
    const t = tab;
    if (!t) return;
    const shape = patternShape(col);
    if (!shape) {
      say("The pattern is being worked out again after a change; try again in a moment.");
      return;
    }
    const before = t.info.rows - (t.header ? 1 : 0);
    const was = t.info.filtered;
    const rep = await runOp({ op: "show_shape", spec: { col, keep_first: t.header, shape, negate, include_blanks: includeBlanks, revision: t.insights?.revision } }, "Filtering");
    if (!rep) return;
    t.filter = null;
    const share = before > 0 ? (rep.rows / before) * 100 : 0;
    const pct = before > 0 ? ` (${share > 0 && share < 0.1 ? "under 0.1" : Math.round(share * 10) / 10}%)` : "";
    const what = negate ? "that do not match" : "that match";
    const within = was ? ", within the current filter" : "";
    if (rep.rows) say(`Showing the ${plural(rep.rows, "row")}${pct} ${what} ${shape}${includeBlanks ? " (and the blank ones)" : ""}${within}. Click “Filtered ✕” to show every row again.`);
    else {
      say(`No row ${negate ? "breaks" : "has"} the pattern ${shape}${within}.`);
      void clearFilter();
    }
    void tick().then(() => grid?.focus());
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

  /** The repair bar, open on this file (row: the shown row to start at, null for the first ragged one). */
  let repairFrom = $state<{ doc: number; row: number | null } | null>(null);
  /** Whether "Repair this row" can run on the active row; read for that row only, as it changes. */
  let repairState = $state<{ doc: number; row: number; why: string }>({ doc: -1, row: -1, why: "" });
  $effect(() => {
    const t = tab;
    if (!t) return;
    const row = t.cell.row;
    const info = t.info;
    void t.version;
    const doc = t.id;
    if (info.column_changes || !info.indexed || row >= info.rows) {
      repairState = { doc, row, why: repairWhy(info, 0, 1) || "choose a row" };
      return;
    }
    const wait = setTimeout(() => {
      void backend()
        .rowWidth(doc, row)
        .then(([w, u]) => {
          repairState = { doc, row, why: row === 0 && t.header ? "the first row holds the column names" : repairWhy(info, w, u) };
        })
        .catch((e) => {
          repairState = { doc, row, why: errorText(e) };
        });
    }, 120);
    return () => clearTimeout(wait);
  });
  function repairWhyNow(): string {
    return !tab || repairState.doc !== tab.id || repairState.row !== tab.cell.row ? "looking at the row" : repairState.why;
  }
  function raggedReady(): boolean {
    return ready() && !tab?.info.column_changes;
  }
  function raggedWhy(): string {
    return tab?.info.column_changes ? COLUMNS_CHANGED : "";
  }

  /** Why the date radar cannot look at the active column ("" when it can). */
  function datesWhy(): string {
    const t = tab;
    return t ? dateWhy(typeAt(t.info.typed, t.cell.col)) : "";
  }
  function datesReady(): boolean {
    return ready() && datesWhy() === "";
  }

  /** Switch the radar on for the active column: count (a sample first), then light the ambiguous dates. */
  async function checkDates(all = false): Promise<void> {
    const t = tab;
    if (!t) return;
    const col = all && t.dateCheck ? t.dateCheck.col : t.cell.col;
    try {
      const scan = await runJob("Looking at the dates", () => backend().dateScan(t.id, col, t.header, all ? null : DATE_SAMPLE));
      if (!scan) return;
      t.dateCheck = { col, cols: t.info.cols, scan };
      t.dateVersion++;
      say(dateSummary(scan, columnTitle(t, col)));
    } catch (e) {
      say(errorText(e));
    }
    void tick().then(() => grid?.focus());
  }

  /** The date review screen: the tab, its column and the counts over every row. */
  let dateReview = $state<{ doc: number; col: number; scan: DateScan } | null>(null);

  async function reviewDates(): Promise<void> {
    const t = tab;
    if (!t) return;
    const col = t.dateCheck ? t.dateCheck.col : t.cell.col;
    try {
      const scan = await runJob("Looking at the dates", () => backend().dateScan(t.id, col, t.header, null));
      if (!scan) return;
      t.dateCheck = { col, cols: t.info.cols, scan };
      t.dateVersion++;
      dateReview = { doc: t.id, col, scan };
    } catch (e) {
      say(errorText(e));
    }
  }

  /** Run what the review screen chose: the clear dates and the typed ones, one undo step. */
  async function applyDateReview(done: DateReviewDone): Promise<void> {
    const open = dateReview;
    dateReview = null;
    const t = tab;
    if (!t || !open || t.id !== open.doc) return;
    const cells = done.typed.map(([row, text]): [number, number, string] => [row, open.col, text]);
    const rep = await runOp({ op: "clean", spec: done.spec, now: localNow(), typed: cells }, "Converting dates");
    if (rep) {
      const typed = cells.length;
      try {
        if (done.remember) t.update(await backend().setColumnType(t.id, open.col, { date: done.remember }), true);
      } catch (e) {
        say(errorText(e));
        return;
      }
      const left = rep.left ?? 0;
      say(
        `Dates: ${plural(rep.cells - typed, "cell")} converted` +
          (typed ? `, ${plural(typed, "typed cell")} set` : "") +
          `.${left - typed > 0 ? ` ${plural(left - typed, "cell")} left as typed; the ones that could be read two ways stay amber.` : ""} Undo with Ctrl+Z.`,
      );
    }
    void tick().then(() => grid?.focus());
  }

  async function nextDate(forward: boolean): Promise<void> {
    const t = tab;
    const check = t?.dateCheck;
    if (!t || !check) return;
    try {
      const row = await runJob("Looking for the next date", () => backend().dateFind(t.id, check.col, t.header, t.cell.row, forward));
      if (row === null || row === undefined) say("No date in this column can be read two ways.");
      else grid?.select({ row, col: check.col }, { row, col: check.col });
    } catch (e) {
      say(errorText(e));
    }
    void tick().then(() => grid?.focus());
  }

  function endDateCheck(): void {
    if (tab) {
      tab.dateCheck = null;
      tab.dateVersion++;
    }
    void tick().then(() => grid?.focus());
  }

  // A change to the data counts again (quietly); columns added or removed end the radar.
  const dateStale = $derived(tab?.dateCheck ? `${tab.id}|${tab.info.data_revision}|${tab.info.cols}|${tab.header}` : "");
  $effect(() => {
    if (!dateStale) return;
    untrack(() => {
      const t = tab;
      const check = t?.dateCheck;
      if (!t || !check) return;
      if (check.cols !== t.info.cols) {
        endDateCheck();
        return;
      }
      void backend()
        .dateScan(t.id, check.col, t.header, check.scan.looked < check.scan.rows ? DATE_SAMPLE : null)
        .then((scan) => {
          if (t.dateCheck && t.dateCheck.col === check.col) t.dateCheck = { ...t.dateCheck, scan };
        })
        .catch(() => {});
    });
  });

  async function showRagged(pick: RaggedPick): Promise<void> {
    const t = tab;
    if (!t) return;
    const rep = await runOp({ op: "show_ragged", pick, keep_first: t.header }, "Filtering");
    if (!rep) return;
    t.filter = null;
    say(raggedResult(pick, rep.rows, rep.groups ?? 0));
    if (!rep.rows) void clearFilter();
    void tick().then(() => grid?.focus());
  }

  async function sortFailing(spec: CheckSpec, scope: "all" | "shown"): Promise<void> {
    fieldCheckOpen = false;
    const rep = await runOp({ op: "sort_problems", spec, scope }, "Checking");
    if (!rep) return;
    const kept = scope === "shown" && hiddenRows.hidden ? ` Only the shown rows moved; ${plural(hiddenRows.hidden, "hidden row")} stayed in place.` : "";
    say(rep.groups ? `Put the ${plural(rep.groups, "row")} that fail first (in their order).${kept} Undo (Ctrl+Z) takes it back.` : "No row fails, so the order stays.");
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

  async function runRecipe(force: boolean, phrase: string, answers: Record<string, string>): Promise<void> {
    const t = tab;
    const r = recipeFile;
    if (!t || !r) return;
    recipeOpen = false;
    const questions = r.recipe.questions ?? [];
    // A private tab's answers are not remembered.
    if (questions.length && !(t.private.file || t.private.folder)) keepAnswers(r.path, answers);
    try {
      const info = await runJob("Running the recipe", () => backend().recipeRun(t.id, withPhrase(r.recipe, phrase), localNow(), force, answerPairs(questions, answers)));
      if (info) t.update(info, true);
      const notes = (info?.run_notes ?? []).map((n) => ` ${n}.`).join("");
      say(`Ran the recipe: ${plural(r.recipe.steps.length, "step")}.${notes} Undo each with Ctrl+Z.`);
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
      const w = await runJob("Writing the list of changes", () => backend().saveChangeReport(t.id, prefs.data.csvFormulaGuard));
      if (w) say(`Saved the list of changes (${plural(w.entries, "change")}) as a CSV report.${guardNote(w.guarded)}`);
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
    const r = oneBlock(t);
    if (!r) return;
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

  /** Which printed columns are typed Text or Id (never set as numbers). */
  function printTextCols(t: Tab, cols: number[]): boolean[] {
    const text = textCols(t.info.typed);
    return cols.map((c) => text.has(c));
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
    const html = printHtml(req.titles, rows, setup, getComputedStyle(document.body).fontFamily || "sans-serif", false, printTextCols(t, req.cols));
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
      let id: number | null = null;
      if (o.existing !== null) {
        const i = tabs.findIndex((t) => t.id === o.existing);
        if (i >= 0) active = i;
        id = o.existing;
      } else if (o.error) say(`${o.name}: ${o.error}`);
      else if (o.opened) {
        opened(o.opened);
        id = o.opened.doc?.id ?? null;
      }
      // Noted while start-up runs, so reopening last time's files
      // doesn't pull the front away from it.
      if (!sessionReady && id !== null) launchTab = tabs.find((t) => t.id === id) ?? launchTab;
    }
  }

  // ---- loading progress -------------------------------------------------------------
  onMount(() => {
    const unwatchAlt = watchAlt(window);
    // Tabs moved between windows, and (in the main window only) files from
    // Windows and a second launch.
    backend().onHandover(() => void takeHandovers());
    void takeHandovers();
    if (SECOND_WINDOW) {
      prefs.onBlockedSave = () => say("Settings are changed in the main window.");
    } else {
      backend().onOpened(() => void takeOpened());
      void takeOpened();
    }
    backend().onAgentGoto((g) => {
      const i = tabs.findIndex((t) => t.id === g.doc);
      if (i < 0) return;
      active = i;
      void tick().then(() => grid?.select({ row: Math.min(g.row, tabs[i]!.info.rows - 1), col: Math.min(g.col, tabs[i]!.info.cols - 1) }));
    });
    if (!SECOND_WINDOW && prefs.data.agentLink) void backend().setAgentLink(true).catch(() => {});
    void backend()
      .appInfo()
      .then((i) => (info = i));
    void loadRecent();
    backend().onCloseRequested(() => void quit());
    // Work left unsaved by a run that stopped is offered back once, at
    // start; otherwise the files open last time are.
    // (The second window offers nothing at start and remembers nothing: the
    // main window owns those. It reports to the app instead, which also tells
    // the app it is alive.)
    if (!SECOND_WINDOW) {
      void showRecovery(true).then(() => {
        if (recovery) sessionReady = true;
        else void startSession();
      });
    }
    const sessionSaver = window.setInterval(() => (SECOND_WINDOW ? syncSecond() : void saveSession()), SECOND_WINDOW ? 3_000 : 10_000);
    if (!SECOND_WINDOW) void backend().versionsKeep(prefs.data.keepEarlierSaves).catch(() => {});
    const autosaver = window.setInterval(() => void autosaveTick(), 15_000);
    // A stale answer (a change held the document) is followed by a fresh one: the tab takes that as its state.
    backend().onFreshInfo((info) => {
      const t = tabs.find((x) => x.id === info.id);
      if (t) t.update(info, false);
    });
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
      clearInterval(sessionSaver);
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

  /** The column a heading's right-click menu was opened on (Rename acts
   *  on it, not on the selection's focus column). */
  let contextCol: number | null = null;
  function openContext(x: number, y: number, area: "cell" | "row" | "col" | "corner" | "tab" | "pill" | "type" | "totals", col: number): void {
    contextCol = area === "col" || area === "pill" || area === "type" || (area === "totals" && col >= 0) ? col : null;
    openMenu = null;
    // The cut items show only while something is cut.
    const cutItems = new Set(["edit.moveCut", "edit.cancelCut"]);
    // "Show changes in the grid" is on the cell menu only while Find is open.
    const ghostItem = "find.ghosts";
    // The counts items (and their line) only in a counts tab.
    const counts = !!tab?.countsOf;
    let items = (contextItems[area] ?? []).filter((i) => (tab?.cut || !cutItems.has(i)) && (i !== ghostItem || findOpen) && (counts || (i !== "data.countsShow" && i !== "data.countsHide")) && (i !== "data.pivotRefresh" || (!!tab && pivotOf.has(tab.id))));
    if (!counts && items[0] === "-") items = items.slice(1);
    // Nothing is picked until the pointer or an arrow key moves onto an item.
    context = { x: Math.min(x, window.innerWidth - 260), y: Math.min(y, window.innerHeight - 420), items, index: -1 };
  }

  // ---- the totals row ------------------------------------------------------

  /** The command that gives a totals cell one choice. */
  function totalCommand(c: TotalChoice) {
    const name = TOTAL_CHOICES.find(([k]) => k === c)?.[1] ?? c;
    return { label: `Totals row: ${name}`, run: () => chooseTotal(c), enabled: totalsOn, checked: () => !!tab && tab.columns.totals.get(totalColumn()) === c };
  }

  function totalsOn(): boolean {
    return !!tab?.totalsOn;
  }
  /** The column the totals menu was opened on, else the active cell's. */
  const totalColumn = (): number => contextCol ?? tab?.cell.col ?? 0;

  function chooseTotal(choice: TotalChoice | null): void {
    const t = tab;
    if (!t) return;
    const c = totalColumn();
    if (choice === null) t.columns.totals.delete(c);
    else t.columns.totals.set(c, choice);
    t.bump();
  }

  /** Copy one column's total, or (null) every shown column's as one row of cells. */
  async function copyTotals(only: number[] | null): Promise<void> {
    const t = tab;
    if (!t || !t.info.indexed) return;
    const cols = only ?? Array.from({ length: Math.min(t.info.cols, 16_384) }, (_, c) => c).filter((c) => !t.columns.isHidden(c));
    try {
      const out: string[] = [];
      for (let at = 0; at < cols.length; at += MAX_ASKED) {
        const part = cols.slice(at, at + MAX_ASKED);
        const r = await backend().columnTotals(t.id, part, t.header, { decimal_comma: false, date_order: "mdy" });
        for (const c of part) out.push(totalCopy(r.columns.find((x) => x.col === c), t.columns.totals.get(c)));
      }
      await navigator.clipboard.writeText(out.join("\t"));
      say(only ? "Copied the total." : `Copied ${plural(out.length, "total")} as one row.`);
    } catch (e) {
      say(errorText(e));
    }
  }

  /** The title-row question, floating over the grid (it takes no room, so
   *  the table never moves); it fades out once answered in any way. */
  const headerQuestion = $derived.by(() => {
    const t = tab;
    if (!t || !t.headerAsk || t.headerTouched || t.headerAskClosed) return null;
    // Worded by how sure the guess is and what is applied now (asked again
    // after the switch was flipped, they can differ).
    return t.header
      ? t.headerAsk === "yes"
        ? "The first row looks like column names, so it is used as titles. Is that right?"
        : t.headerAsk === "no"
          ? "The first row looks like data, but it is used as titles for now. Is that right?"
          : "The first row may hold column names; it is used as titles for now. Is that right?"
      : t.headerAsk === "no"
        ? "The first row looks like data, so the columns are numbered. Does it hold column names after all?"
        : t.headerAsk === "yes"
          ? "The first row looks like column names, but it is read as data for now. Does it hold column names?"
          : "Does the first row hold column names? It is read as data for now.";
  });

  /** The floating question's height: the grid keeps the active cell clear
   *  of it (plus its gap) while it shows. */
  let headerAskH = $state(0);
  const headerAskInset = $derived(headerQuestion ? headerAskH + 28 : 0);

  /** Esc in the grid (nothing else to cancel) or the X: close the question,
   *  keeping the guess. */
  function closeHeaderAsk(): boolean {
    const t = tab;
    if (!t || !headerQuestion) return false;
    t.headerAskClosed = true;
    void tick().then(() => grid?.focus());
    return true;
  }

  const notices = $derived.by(() => {
    const t = tab;
    type Notice = { text: string; action?: string; run?: () => void; actions?: { label: string; run: () => void; title?: string }[] };
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
    const ragged = raggedNotice(d, t.header);
    if (ragged) out.push({ text: ragged, action: "Review", run: () => void showRagged("all"), actions: [{ label: "Repair…", run: () => { repairFrom = { doc: t.id, row: null }; } }] });
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
    const r = t.singleRect();
    const delim = separatorName(d);
    return {
      size: d.filtered
        ? `${Math.max(0, d.rows - t.firstRow).toLocaleString()} of ${Math.max(0, d.total_rows - t.firstRow).toLocaleString()} rows shown × ${d.cols.toLocaleString()} columns`
        : `${Math.max(0, d.rows - t.firstRow).toLocaleString()} ${d.rows - t.firstRow === 1 ? "row" : "rows"} × ${d.cols.toLocaleString()} ${d.cols === 1 ? "column" : "columns"}`,
      hidden: d.hidden_rows,
      heightsHidden: d.filtered && !t.rowHeights.empty,
      filter: d.has_filter || d.filter_on_deleted
        ? [
            ...(t.filter?.columns ?? []).map((c) => `${columnTitle(t, c.col)} ${describeRule(c.rule)}`),
            ...(d.filter_on_deleted ? ["a rule on a deleted column (Undo brings the column back)"] : []),
            ...(!t.filter?.columns.length && !d.filter_on_deleted ? ["rows with duplicates"] : []),
          ].join("; ")
        : "",
      sel: r ? (cellCount(r) > 1 ? `${r.rows.toLocaleString()} × ${r.cols.toLocaleString()} selected` : "") : `${cellCountAreas(t.areas).toLocaleString()} cells in ${t.areas.length.toLocaleString()} blocks selected`,
      format: `${d.encoding}${d.bom ? " (BOM)" : ""} · ${delim} separated · ${d.line_break}${d.mixed_line_breaks ? " (mixed)" : ""}`,
    };
  });
</script>

<svelte:window
  onkeydown={onKeyDown}
  onkeyup={onKeyUp}
  onpaste={onPaste}
  onpointerupcapture={lookBoundary}
  onpointerdowncapture={(e) => {
    if (extendMode && !(e.target as HTMLElement | null)?.closest?.("footer")) extendMode = false;
  }}
  oncontextmenucapture={(e) => {
    // Shift+F10 or the Menu key already opened the app's menu: the
    // browser's own context menu event that follows is dropped.
    if (performance.now() < ctxKeysUntil) {
      e.preventDefault();
      e.stopPropagation();
    }
  }}
/>

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
                  title={reg.enabled(id) ? undefined : c.why?.()}
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
        title="Go to: type a row, or row and column (Ctrl+G). Go to special (Ctrl+Shift+G) picks blank, number, text, changed or failing cells."
        bind:value={gotoText}
        onkeydown={(e) => {
          e.stopPropagation();
          if (e.key === "Enter") {
            // Kept from reaching the grid once it has the focus (it would
            // start editing the cell gone to).
            e.preventDefault();
            void goTo(gotoText);
          }
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
        <button type="button" class="small primary" onclick={() => void applyCellBar(true)} title={cellCountAreas(tab.areas) > 1 ? "Put the value in every selected cell (Ctrl+Enter)" : `Apply (${cellBarExpanded ? "Ctrl+Enter" : "Enter"})`}
          >{cellCountAreas(tab.areas) > 1 ? `Apply to ${cellCountAreas(tab.areas).toLocaleString()} cells` : "Apply"}</button
        >
        <button type="button" class="small" onclick={() => { cellBarDirty = false; cellBarFor = ""; }}>Cancel</button>
      {/if}
      <button type="button" class="small expand" title="Expand cell bar (Ctrl+Shift+U)" aria-label="Expand cell bar" onclick={() => (cellBarExpanded = !cellBarExpanded)}
        >{cellBarExpanded ? "▴" : "▾"}</button
      >
    </div>
    {#if prefs.data.xray && xrayNote(cellBarText)}
      <div class="invnote" role="status">X-ray: {xrayNote(cellBarText)}</div>
    {:else if prefs.data.showInvisible && invisibleNote(cellBarText)}
      <div class="invnote" role="status">Can't be seen in this cell: {invisibleNote(cellBarText)}</div>
    {/if}
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
        bind:block={findBlock}
        bind:blocks={findBlocks}
        onSelect={(row, col) => {
          // Inside the block searched, only the active cell moves and the
          // block stays selected (as in Excel). With several blocks, all of
          // them stay selected and the block holding the hit becomes the last.
          const b = findBlock;
          if (findBlocks.length > 1 && (findOptions.scope === "selection" || findOptions.scope === "column")) {
            if (tab?.selectBlocks(findBlocks, { row, col })) {
              grid?.scrollIntoView({ row, col });
              grid?.redraw();
              return;
            }
            grid?.select({ row, col });
            return;
          }
          if (b && findOptions.scope !== "all" && row >= b.row && row < b.row + b.rows && col >= b.col && col < b.col + b.cols) {
            grid?.select({ row: b.row, col: b.col }, { row: b.row + b.rows - 1, col: b.col + b.cols - 1 }, false, { row, col });
            grid?.scrollIntoView({ row, col });
          } else grid?.select({ row, col });
        }}
        {tabs}
        onChanged={(d, t) => (t ?? tab)?.update(d, true)}
        onGoTab={(t, row, col) => {
          const i = tabs.indexOf(t);
          if (i < 0) return;
          active = i;
          void tick().then(() => { grid?.select({ row, col }); grid?.scrollIntoView({ row, col }); });
        }}
        onNewTab={(d) => addTab(d)}
        onRenameColumns={async (list) => {
          const t = tab;
          if (!t) return;
          for (const [c, name] of list) await renameColumn(t.id, c, name, true, shownTitles(t));
        }}
        onBatch={() => { batchOpen = true; }}
        onCopy={async (text, from) => {
          // Every tab the list comes from is checked.
          for (const t of from) if (!(await privacyGate(`The list of matches${from.length > 1 ? ` (with ${t.info.name})` : ""} goes to the clipboard, where other programs can read it.`, undefined, t))) return false;
          try {
            await navigator.clipboard.writeText(text);
            return true;
          } catch {
            say("The clipboard could not be used.");
            return false;
          }
        }}
        onClose={() => { findOpen = false; findBlock = null; findBlocks = []; grid?.focus(); }}
      />
    {/if}
  {/if}

  <!-- Double-clicking the empty part of the bar starts a new file. -->
  <div class="tabs" class:tab-dragging={tabDrag?.moved} role="tablist" aria-label="Open files" tabindex="-1" bind:this={tabBar} ondblclick={(e) => { if (e.target === e.currentTarget) void newFile(); }}>
    {#each tabs as t, i (t.id)}
      <div class="tab" class:active={i === active} class:dragging={tabDrag?.moved && tabs[tabDrag.from] === t} class:drop={cellDropTab === t.id} style:transform={tabShift(i)} role="tab" aria-selected={i === active} tabindex="-1" title={t.info.path ?? t.info.name}
        data-tab-index={i}
        onmousedown={(e) => { if (e.button === 1) { e.preventDefault(); void closeTab(i); } }}
        oncontextmenu={(e) => tabMenu(e, i)}
        onpointerdown={(e) => tabPointerDown(e, i)}
        onpointermove={tabPointerMove}
        onpointerup={tabPointerUp}
        onpointercancel={tabPointerUp}
        onclick={() => { if (tabDragged) { tabDragged = false; return; } active = i; void tick().then(() => grid?.focus()); }}
        onkeydown={(e) => { if (e.key === "F2") { e.preventDefault(); active = i; void renameFile(); } }}
        ondblclick={(e) => { e.stopPropagation(); active = i; if (!t.info.path) void renameFile(); }}
        ondragstart={(e) => e.preventDefault()}
        onlostpointercapture={(e) => { if (tabDrag && e.pointerId === tabDrag.pointer) endTabDrag(false); }}
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
          {#each n.actions ?? [] as a (a.label)}<button type="button" class="small" title={a.title} aria-keyshortcuts={a.title} onclick={a.run}>{a.label}</button>{/each}
        </div>
      {/each}
      <button type="button" class="small dismiss" aria-label="Dismiss notices" onclick={() => { if (tab) tab.dismissed = true; }}><Icon name="close" size={12} /></button>
    </div>
  {/if}

  {#if tab}
    {@const dn = diskBanner(tab)}
    {#if dn}
      <DiskBanner
        name={tab.info.name}
        note={dn}
        onReload={() => { if (tab) void reloadFromDisk(tab); }}
        onCompare={() => { if (tab) void compareWithDisk(tab); }}
        onSaveAs={() => void runCmd("file.saveAs")}
        onIgnore={() => { if (tab) tab.diskIgnored = dn.token; }}
        onMute={() => { if (tab) tab.diskMuted = true; }}
      />
    {/if}
  {/if}

  <main class="body" class:with-panel={!!prefs.data.panel}>
    {#if tab}
      {#key tab.id}
        {#if tab.split}
          {@const sp = tab.split}
          <div class="splitview" class:cols={sp.dir === "cols"} bind:this={splitEl} style={sp.dir === "cols" ? `grid-template-columns: minmax(0, ${sp.at}fr) 6px minmax(0, ${1 - sp.at}fr)` : `grid-template-rows: minmax(0, ${sp.at}fr) 6px minmax(0, ${1 - sp.at}fr)`}>
            <div class="pane">
            <Grid
                  onSeam={(row, x, y) => void openSeam(row, x, y)}
                  bind:this={grid}
                  {tab}
                  bottomInset={headerAskInset}
                  readOnly={!tab.info.indexed || tab.peeking}
                  locked={tab.info.read_only}
                  onCommit={commitCell}
                  onContext={openContext}
                  onTotals={(c, x, y) => openContext(x, y, "totals", c)}
                  onRenameCol={(c) => void askRename(c)}
                  onInsight={(c) => (profileCol = c)}
                  onXray={(cells, cols) => { xrayNow = cells || cols ? { cells, cols } : null; }}
                  onPill={(c, part, px, py) => {
                    // The leftover part filters at once; the shape part opens the pill's menu.
                    if (part === "rest") void showShape(c, true, false);
                    else openContext(px, py, "pill", c);
                  }}
                  onActivate={() => {
                    if (!tab?.countsOf) return false;
                    void countsShow(false);
                    return true;
                  }}
                  onMessage={say}
                  onFilterButton={(col, x, y) => (filterMenu = { col, x, y })}
                  onFill={(dir, count, swap) => void fillHandle(dir, count, swap)}
                  onFillClear={(r) => void clearRect(r)}
                  onFillAuto={() => void fillHandleAuto()}
                  onOpenLink={(url) => void openLink(url)}
                  onMoveCols={(at, count, to) => void dragCols(at, count, to)}
                  onMoveRows={(at, count, to) => void dragRows(at, count, to)}
                  onMoveCells={(from, row, col, copy) => void dragCells(from, row, col, copy)}
                  onCellsOver={cellsOver}
                  onCellsDropOutside={cellsDropOutside}
                  onJump={(dir, extend) => void jump(dir, extend)}
                />
            </div>
            <div class="divider" role="separator" aria-orientation={sp.dir === "cols" ? "vertical" : "horizontal"} aria-label="Split: drag to resize, or to the edge to close" title="Drag to resize; drag to the edge to close the split" onpointerdown={splitDrag}></div>
            <div class="pane">
            <Grid
                  onSeam={(row, x, y) => void openSeam(row, x, y)}
                  pane="split"
                  bottomInset={headerAskInset}
                  {tab}
                  readOnly={!tab.info.indexed || tab.peeking}
                  locked={tab.info.read_only}
                  onCommit={commitCell}
                  onContext={openContext}
                  onTotals={(c, x, y) => openContext(x, y, "totals", c)}
                  onRenameCol={(c) => void askRename(c)}
                  onInsight={(c) => (profileCol = c)}
                  onXray={(cells, cols) => { xrayNow = cells || cols ? { cells, cols } : null; }}
                  onPill={(c, part, px, py) => {
                    // The leftover part filters at once; the shape part opens the pill's menu.
                    if (part === "rest") void showShape(c, true, false);
                    else openContext(px, py, "pill", c);
                  }}
                  onActivate={() => {
                    if (!tab?.countsOf) return false;
                    void countsShow(false);
                    return true;
                  }}
                  onMessage={say}
                  onFilterButton={(col, x, y) => (filterMenu = { col, x, y })}
                  onFill={(dir, count, swap) => void fillHandle(dir, count, swap)}
                  onFillClear={(r) => void clearRect(r)}
                  onFillAuto={() => void fillHandleAuto()}
                  onOpenLink={(url) => void openLink(url)}
                  onMoveCols={(at, count, to) => void dragCols(at, count, to)}
                  onMoveRows={(at, count, to) => void dragRows(at, count, to)}
                  onMoveCells={(from, row, col, copy) => void dragCells(from, row, col, copy)}
                  onCellsOver={cellsOver}
                  onCellsDropOutside={cellsDropOutside}
                  onJump={(dir, extend) => void jump(dir, extend)}
                />
            </div>
          </div>
        {:else}
        <Grid
          onSeam={(row, x, y) => void openSeam(row, x, y)}
          bind:this={grid}
          {tab}
          bottomInset={headerAskInset}
          readOnly={!tab.info.indexed || tab.peeking}
          locked={tab.info.read_only}
          onCommit={commitCell}
          onContext={openContext}
          onTotals={(c, x, y) => openContext(x, y, "totals", c)}
          onRenameCol={(c) => void askRename(c)}
          onInsight={(c) => (profileCol = c)}
                  onXray={(cells, cols) => { xrayNow = cells || cols ? { cells, cols } : null; }}
                  onPill={(c, part, px, py) => {
                    // The leftover part filters at once; the shape part opens the pill's menu.
                    if (part === "rest") void showShape(c, true, false);
                    else openContext(px, py, "pill", c);
                  }}
          onActivate={() => {
            if (!tab?.countsOf) return false;
            void countsShow(false);
            return true;
          }}
          onMessage={say}
          onFilterButton={(col, x, y) => (filterMenu = { col, x, y })}
          onFill={(dir, count, swap) => void fillHandle(dir, count, swap)}
          onFillClear={(r) => void clearRect(r)}
          onFillAuto={() => void fillHandleAuto()}
          onOpenLink={(url) => void openLink(url)}
          onMoveCols={(at, count, to) => void dragCols(at, count, to)}
          onMoveRows={(at, count, to) => void dragRows(at, count, to)}
          onMoveCells={(from, row, col, copy) => void dragCells(from, row, col, copy)}
                  onCellsOver={cellsOver}
                  onCellsDropOutside={cellsDropOutside}
          onJump={(dir, extend) => void jump(dir, extend)}
        />
        {/if}
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
        onSetRule={(col, rule) => setRule(col, rule)}
        record={{ onGo: (row, col) => grid?.select({ row, col }) }}
        notes={{ onPutBack: () => void putBackNotes(), onKeep: (keep) => void setKeepNotes(keep) }}
        steps={{
          onInfo: (info) => {
            const t = tab;
            if (t) {
              t.update(info, true);
              followHeaderEdits(t);
            }
          },
          run: runJob,
          confirmLoss: async (what) =>
            (await ask("Run the steps again?", `${what[0]?.toUpperCase() ?? ""}${what.slice(1)}. Go ahead? Undo (Ctrl+Z) takes the new steps back one by one.`, [
              { id: "go", label: "Go ahead", primary: true },
              { id: "cancel", label: "Cancel" },
            ], "cancel")) === "go",
          onSave: () => void openRecipes(),
          now: localNow,
          restoreCheckpoint,
          showSince: (c) => { if (tab) sinceFor = { doc: tab.id, checkpoint: c }; },
        }}
        onFilterValue={(col, value, cut) => void setRule(col, cut ? { type: "conditions", first: { type: "text", op: "begins_with", text: value.replace(/…$/, ""), match_case: true } } : value === "" ? { type: "values", values: [], blanks: true } : { type: "values", values: [value], blanks: false })}
        onClose={() => showPanel(null)}
      />
    {/if}
    {#if headerQuestion && tab}
      <div class="header-ask" role="group" aria-label="Title row question" style:margin-bottom={tab?.totalsOn ? `${grid?.totalsHeight() ?? 0}px` : null} bind:clientHeight={headerAskH} transition:fade={{ duration: 250 }}>
        <span role="status" aria-live="polite">{headerQuestion}</span>
        <button type="button" class="small" title="Alt+Shift+Y" aria-keyshortcuts="Alt+Shift+Y" onclick={() => void answerHeader(true)}>Yes, column names</button>
        <button type="button" class="small" title="Alt+Shift+N" aria-keyshortcuts="Alt+Shift+N" onclick={() => void answerHeader(false)}>No, it's data</button>
        {#if tab.titleLines > 0 && !tab.info.filtered}
          <button type="button" class="small" title="Set the {tab.titleLines === 1 ? 'line' : `${tab.titleLines} lines`} above aside as report notes (saved back above the table)" onclick={() => void headersStartHere(tab?.titleLines)}>Headers start at row {tab.titleLines + 1}</button>
        {/if}
        <button type="button" class="small close" aria-label="Close: keep the guess" title="Keep the guess (Esc)" onclick={() => void closeHeaderAsk()}><Icon name="close" size={12} /></button>
      </div>
    {/if}
  </main>

  {#if tab && repairFrom && repairFrom.doc === tab.id}
    {#key `${repairFrom.doc}:${repairFrom.row}`}
      <RepairBar
        {tab}
        titles={Array.from({ length: Math.min(tab.info.cols, 200) }, (_, c) => columnTitle(tab!, c))}
        start={repairFrom.row}
        onGo={(row) => grid?.select({ row, col: tab!.cell.col }, { row, col: tab!.cell.col })}
        onDone={(info, what) => { tab!.update(info, true); say(`${what} Undo (Ctrl+Z) takes it back.`); }}
        job={(label, call) => runJob(label, call)}
        onReopenLines={() => { repairFrom = null; void reopenLineByLine(); }}
        onClose={() => { repairFrom = null; void tick().then(() => grid?.focus()); }}
      />
    {/key}
  {/if}

  <footer class="status">
    {#if message}
      <span class="msg">{message}</span>
      {#if messageAction}<button type="button" class="small" onclick={() => { const run = messageAction?.run; messageAction = null; run?.(); }}>{messageAction.label}</button>{/if}
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
    {#if findOpen && tab && tab.findCount !== null}<span class="muted" title="Cells that match the find bar's search">Find: {tab.findCount.toLocaleString()}</span>{/if}
    {#if extendMode}<button type="button" class="small" title="Arrow keys grow the selection. Click to end (or F8, or Esc)." onclick={() => toggleExtend()}>Extend selection</button>{/if}
    {#if tab?.diffOn}
      <span class={tab.diffOutOfDate ? "msg" : "muted"} title="Compared by key (Tools > Compare). Alt+F7 next difference, Alt+Shift+F7 previous.">{tab.diffOutOfDate ? "Differences out of date: compare again" : "Differences marked"}</span>
      <button type="button" class="small" title="Take the colours off the compared tabs" onclick={() => void clearDiff()}>Clear</button>
    {/if}
    {#if tab && (tab.checks.length || tab.tableChecks.length)}
      {@const note = problemNote(tab)}
      {#if note}<span class="msg" title="The rule this cell breaks (F7 jumps to the next problem)">{note}</span>{/if}
      <span class="muted" title={tab.rulesNote || "How many rules are set and how many problems the last full check found. F7 jumps to the next problem."}>
        {#if tab.checks.length}{plural(tab.checks.length, "rule")}{:else}{plural(tab.tableChecks.length, "table check")}{/if}{#if tab.lastRun}, {plural(tab.lastRun.problems, "problem")}{#if tab.lastRun.tableFailed}, {plural(tab.lastRun.tableFailed, "table check")} not passed{/if}{#if tab.lastRun.revision !== tab.info.data_revision} (not current){/if}{/if}
      </span>
      {#if !tab.lastRun || tab.lastRun.revision !== tab.info.data_revision}<button type="button" class="small" title="Run the rules over the whole table" onclick={() => { const t = tab; if (t) void checkNow(t); }}>Check now</button>{/if}
    {/if}
    {#if status}
      {#if tab?.dateCheck}
        {@const radar = tab.dateCheck}
        {@const radarColumn = columnTitle(tab, radar.col)}
        <span class="date-pill" data-testid="date-pill">
          <button type="button" class="small" title={dateSummary(radar.scan, radarColumn)} onclick={() => void nextDate(true)}>{datePill(radar.scan)}{dateSampled(radar.scan) ? " (first rows)" : ""}</button>
          <button type="button" class="small" aria-label="Previous ambiguous date" onclick={() => void nextDate(false)}>▲</button>
          <button type="button" class="small" aria-label="Next ambiguous date" onclick={() => void nextDate(true)}>▼</button>
          {#if dateSampled(radar.scan)}<button type="button" class="small" onclick={() => void checkDates(true)}>Count all rows</button>{/if}
          <button type="button" class="small" onclick={() => void reviewDates()}>Review…</button>
          <button type="button" class="small" aria-label="Remove the ambiguous date marks" onclick={endDateCheck}>✕</button>
        </span>
      {/if}
      {#if prefs.data.xray}<button type="button" class="small" data-testid="xray-status" title="X-ray: what looks right but is not, on this screen. Click for a report of the selected columns or the whole file." onclick={() => (xrayOpen = true)}>X-ray: {xrayNow ? `${plural(xrayNow.cells, "cell")}, ${plural(xrayNow.cols, "column")}` : "nothing on this screen"}</button>{/if}
      {#if tab}<QuickTotals {tab} />{/if}
      {#if status.sel}<span>{status.sel}</span>{/if}
      {#if status.heightsHidden}<span title="The rows you gave a height keep it and get it back when the filter or the hidden rows are cleared.">Custom row heights hidden while filtered</span>{/if}
      {#if tab && hiddenRows.hidden}
        <span class="acting" class:including={hiddenRows.including > 0} data-testid="acting-pill">{actingWords(hiddenRows.hidden, hiddenRows.total, hiddenRows.including > 0)}</span>
      {/if}
      {#if status.hidden}
        <button type="button" class="small filtered" title="Hidden rows are still saved. Click to show them all again." onclick={() => void showRows(true)}>{plural(status.hidden, "hidden row")} ✕</button>
      {/if}
      {#if status.filter}
        <button type="button" class="small filtered" title={`Filter: ${status.filter}. Click to show all rows.`} onclick={() => void clearFilter()}>Filtered ✕</button>
      {/if}
      {#if tab?.info.read_only}<button type="button" class="small" title="Locked: cells cannot change. Click to unlock." onclick={() => void toggleReadOnly()}>Locked</button>{/if}
      <span>{status.size}</span>
      <button type="button" class="small" data-testid="encoding-button" title="Change how the file is read (encoding). Click, then use the arrow keys to see the first rows in each one." onclick={() => (encodingOpen = !!tab?.info.path)}>{status.format}</button>
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
          <button
            type="button"
            role="menuitem"
            class="item"
            class:hot={context.index === pos}
            disabled={!reg.enabled(id)}
            title={reg.enabled(id) ? undefined : c.why?.()}
            onmouseenter={() => {
              if (context) context.index = pos;
            }}
            onclick={() => void runCmd(id)}
          >
            <span class="check">{c.checked?.() ? "✓" : ""}</span>
            <span class="label">{contextLabel(id) ?? plain(c.label)}</span>
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
    sel={blockOr(tab)}
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
    onClose={(row) => { formRow = null; grid?.select({ row, col: tab?.cell.col ?? 0 }); grid?.focus(); }}
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
  <SaveAsDialog info={tab.info} sample={req.sample} privateFile={tab.private.file || tab.private.folder} onSave={(o, r) => req.done(o, r)} onClose={() => { req.done(null); grid?.focus(); }} />
{/if}

{#if exportOpen && tab}
  <ExportDialog
    doc={tab.id}
    name={tab.info.name}
    header={tab.header}
    filtered={tab.info.filtered}
    titles={Array.from({ length: Math.min(tab.info.cols, 200) }, (_, c) => columnTitle(tab!, c))}
    rows={tab.info.rows}
    tabList={exportTabs}
    widths={Array.from({ length: Math.min(tab.info.cols, 16_384) }, (_, c) => tab!.columns.customWidth(c) ?? 0)}
    rowHeights={tab.info.filtered ? [] : tab.rowHeights.toView()}
    onExport={(o, to, ids) => void exportRun(o, to, ids)}
    onClose={() => { exportOpen = false; grid?.focus(); }}
  />
{/if}

{#if importing}
  {@const start = importing}
  <ImportDialog {start} onImport={(o, header, notes) => void finishImport(start, o, header, notes)} onClose={closeImport} />
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

{#if encodingOpen && tab}
  <EncodingPicker
    doc={tab.id}
    current={tab.info.encoding}
    header={tab.header}
    onApply={(c, now) => void changeEncoding(c, now)}
    onShow={(info) => { encodingOpen = false; tab?.update(info, true); say("Showing only the rows in another encoding. Data > Show all rows brings the rest back."); grid?.focus(); }}
    onFixMojibake={() => { encodingOpen = false; cleanTool = "fix_mojibake"; }}
    onClose={() => { encodingOpen = false; grid?.focus(); }}
  />
{/if}

{#if xrayOpen && tab}
  <XrayDialog
    doc={tab.id}
    titles={Array.from({ length: Math.min(tab.info.cols, 2000) }, (_, c) => columnTitle(tab!, c))}
    cols={tab.areas.length && cellCountAreas(tab.areas) > 1 ? selectedCols(tab) : []}
    header={tab.header}
    onShow={(info) => { xrayOpen = false; tab?.update(info, true); say("Showing only the rows with look-alike or invisible characters. Data > Show all rows brings the rest back."); grid?.focus(); }}
    onSanitize={() => { xrayOpen = false; cleanTool = "sanitize"; }}
    onClose={() => { xrayOpen = false; grid?.focus(); }}
  />
{/if}

{#if dateReview && tab && tab.id === dateReview.doc}
  <DateReviewDialog
    doc={dateReview.doc}
    col={dateReview.col}
    column={columnTitle(tab, dateReview.col)}
    header={tab.header}
    scan={dateReview.scan}
    onApply={(done) => void applyDateReview(done)}
    onClose={() => { dateReview = null; grid?.focus(); }}
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
    onUnavailable={(name) => say(!ready() ? `“${name}” needs a file: open one first.` : tab && tab.extra.length > 0 ? ONE_BLOCK : `“${name}” isn't available here right now.`)}
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
    onRun={(f, p, a) => void runRecipe(f, p, a)}
    onClose={() => { recipeOpen = false; recipeFile = null; grid?.focus(); }}
  />
{/if}

{#if sinceFor && tab && tab.id === sinceFor.doc}
  <SinceDialog
    doc={sinceFor.doc}
    checkpoint={sinceFor.checkpoint}
    header={tab.header}
    titles={shownTitles(tab)}
    onGo={(row, col) => { sinceFor = null; goToChange(row, col); }}
    onStop={cancelJob}
    onClose={() => { sinceFor = null; grid?.focus(); }}
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
    col={tab.cell.col}
    rules={tab.highlights}
    onSave={(rules) => { highlightOpen = false; setHighlights(rules); grid?.focus(); }}
    onClose={() => { highlightOpen = false; grid?.focus(); }}
  />
{/if}

{#if valuesOpen && tab}
  <ValueEditor
    doc={tab.id}
    titles={shownTitles(tab)}
    col={tab.cell.col}
    header={tab.header}
    startMethod={valuesMethod}
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
    col={tab.cell.col}
    header={tab.header}
    onRun={(op, header) => { editKind = null; void runDialog({ data: op }, header); grid?.focus(); }}
    onClose={() => { editKind = null; grid?.focus(); }}
  />
{/if}

{#if profileCol !== null && tab}
  <ProfileDialog
    doc={tab.id}
    col={profileCol}
    title={`“${columnTitle(tab, profileCol)}”`}
    colRef={columnRef(tab.header ? columnTitle(tab, profileCol) : "", profileCol)}
    header={tab.header}
    run={runJob}
    onShow={(expr, what) => {
      const t = tab;
      profileCol = null;
      if (!t) return;
      computeMode = "filter";
      void runCompute({ expr, title: "", target: null, at: null, keep_first: t.header, read: { decimal_comma: false, date_order: "mdy" }, now: localDays() }).then(() => say(`Showing the rows with ${what}. Data › Clear filter shows every row.`));
    }}
    onClose={() => { profileCol = null; grid?.focus(); }}
  />
{/if}

{#if computeOpen && tab}
  <ComputeDialog
    doc={tab.id}
    titles={shownTitles(tab)}
    col={tab.cell.col}
    header={tab.header}
    mode={computeMode}
    onRun={(s) => void runCompute(s)}
    onClose={() => { computeOpen = false; grid?.focus(); }}
  />
{/if}

{#if recipeFilesOpen}
  <BatchDialog onClose={() => { recipeFilesOpen = false; grid?.focus(); }} />
{/if}

{#if watchOpen}
  <WatchDialog onClose={() => { watchOpen = false; grid?.focus(); }} />
{/if}

{#if receiptOpen}
  <ReceiptDialog onClose={() => { receiptOpen = false; grid?.focus(); }} />
{/if}

{#if notesRow !== null && tab}
  <NotesDialog
    doc={tab.id}
    row={notesRow}
    titles={Array.from({ length: Math.min(tab.info.cols, 2000) }, (_, c) => columnTitle(tab!, c))}
    header={tab.header}
    current={tab.rowNotes.get(notesRow) ?? null}
    onChanged={() => { const t = tab; if (t) { t.notesVersion++; t.bump(); } }}
    onGo={(row) => { notesRow = null; grid?.select({ row, col: tab?.cell.col ?? 0 }); grid?.focus(); }}
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
    start={sqliteStart}
    doc={tab?.id ?? null}
    docName={tab?.info.name ?? ""}
    header={tab?.header ?? true}
    filtered={!!tab && tab.info.rows < tab.info.total_rows}
    onSaved={(n) => { sqliteMode = null; say(`Saved ${plural(n, "row")} to the database.`); grid?.focus(); }}
    onOpened={(info, note) => { sqliteMode = null; sqliteStart = null; addTab(info, true); say(`Opened the table in a new tab.${note ? ` ${note}` : ""} Save it as a file with File ▸ Save as, or back to a database with File ▸ Save as a table in a database.`); }}
    onClose={() => { sqliteMode = null; sqliteStart = null; grid?.focus(); }}
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
    col={tab.cell.col}
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
    col={tab.cell.col}
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
    onGolden={goldenMade}
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
    onSortFailing={(s, scope) => void sortFailing(s, scope)}
    onResultColumn={(s) => void addCheckColumn(s)}
    onClose={() => { fieldCheckOpen = false; grid?.focus(); }}
  />
{/if}

{#if checksOpen && tab}
  <ChecksDialog
    doc={tab.id}
    titles={shownTitles(tab)}
    col={tab.cell.col}
    header={tab.header}
    checks={tab.checks}
    tableChecks={tab.tableChecks}
    focusTable={checksAtTable}
    marked={tab.checksMarked}
    onApply={keepChecks}
    rulesFile={tab.rulesNote ? tab.rulesNote.replace(/^Loaded .* from /, "").replace(/ \(.*$/, "") : ""}
    others={tabs.map((o) => ({ id: o.id, name: o.info.name, titles: shownTitles(o), data_revision: o.info.data_revision ?? 0 }))}
    onGoto={(row, col) => { const t = tab; if (t) grid?.select({ row, col: col ?? t.cell.col }); }}
    onShowProblems={(s) => void showProblems(s)}
    onClose={() => { checksOpen = false; checksAtTable = false; grid?.focus(); }}
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
    textCols={printTextCols(tab, printReq.cols)}
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
    col={t.cell.col}
    header={t.header}
    others={tabs.filter((x) => x !== t).map((x) => ({ id: x.id, name: x.info.name }))}
    onRun={(r, header) => { toolsKind = null; void runTools(r, header); }}
    onClose={() => { toolsKind = null; grid?.focus(); }}
    onPickFile={async () => {
      const o = await openOther();
      return o ? { id: o.id, name: o.name } : null;
    }}
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

{#if batchOpen && tab}
  <BatchReplaceDialog
    {tab}
    {tabs}
    titles={shownTitles(tab)}
    sel={{ col: blockOr(tab).col, cols: blockOr(tab).cols }}
    run={runJob}
    onDone={(r) => {
      batchOpen = false;
      if (r.info) tab?.update(r.info, true);
      say(`Replaced ${plural(r.cells, "cell")} from the list. Undo (Ctrl+Z) reverts it.`);
      grid?.focus();
    }}
    onClose={() => { batchOpen = false; grid?.focus(); }}
  />
{/if}

{#if subtotalOpen && tab}
  <SubtotalDialog
    titles={shownTitles(tab)}
    col={tab.cell.col}
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
    col={tab.cell.col}
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
    count={kind === "rows" ? blockOr(tab).rows : blockOr(tab).cols}
    onRun={(n, after) => { const rows = kind === "rows"; insertAsk = null; void (rows ? insertRows(after, n) : insertCols(after, n)).then(() => grid?.focus()); }}
    onClose={() => { insertAsk = null; void tick().then(() => grid?.focus()); }}
  />
{/if}

{#if compareAsk && tab}
  {@const c = compareAsk}
  <CompareColsDialog
    titles={c.titles}
    hidden={hiddenColsOf(tab)}
    first={c.first}
    second={c.second}
    header={tab.header && tab.info.rows > 0}
    readOnly={!!tab.info.read_only}
    onRun={(spec, action) => void runCompareCols(spec, action).then(() => grid?.focus())}
    onClose={() => { compareAsk = null; grid?.focus(); }}
  />
{/if}
{#if setsAsk && tab}
  {@const s = setsAsk}
  <SetsDialog
    tabs={s.tabs}
    current={s.current}
    {titlesOf}
    onRun={runSets}
    onCancelRun={() => { if (tab) void backend().cancelJob(tab.id); }}
    onOpen={(id, mask, list, keys, header) => openSetsTab(() => backend().setsOpen(id, mask, list, keys), keys || header, id)}
    onTable={(id, saveIt) => openSetsTab(() => backend().setsTable(id), true, id, saveIt)}
    onCopy={(t) => void copySets(t)}
    onClose={(id) => { setsAsk = null; if (id !== null) void backend().setsClose(id).catch(() => {}); grid?.focus(); }}
  />
{/if}
{#if patchAsk && tab}
  {@const p = patchAsk}
  <PatchDialog
    save={p.save}
    apply={p.apply}
    run={runJob}
    onCancelRun={cancelJob}
    onSaved={(m) => { patchAsk = null; say(m); grid?.focus(); }}
    onApply={applyPatch}
    onSaveReport={async () => { const to = p.apply ? await backend().patchReportSave(p.apply.doc) : null; return to ? `Report saved to ${to}.` : ""; }}
    onClose={closePatch}
  />
{/if}
{#if relationsAsk && tab}
  {@const r = relationsAsk}
  <RelationsDialog
    titles={r.titles}
    hidden={hiddenColsOf(tab)}
    header={tab.header && tab.info.rows > 0}
    onRun={runRelations}
    onCancelRun={() => { if (tab) void backend().cancelJob(tab.id); }}
    onExceptions={showExceptions}
    onCopy={(t) => void copyRelations(t)}
    onClose={() => { relationsAsk = null; grid?.focus(); }}
  />
{/if}
{#if matrixAsk && tab}
  {@const m = matrixAsk}
  <CleanMatrixDialog
    titles={m.titles}
    picked={m.picked}
    hidden={hiddenColsOf(tab)}
    protectedCols={tab.info.protected ?? []}
    textCols={[...textCols(tab.info.typed)]}
    header={tab.header && tab.info.rows > 0}
    revision={`${tab.info.data_revision ?? 0}:${tab.info.rows}`}
    readOnly={!!tab.info.read_only}
    onPreview={previewMatrix}
    onCancelRun={() => { if (tab) void backend().cancelJob(tab.id); }}
    onOpenCleaner={() => backend().cleanerOpenNamed()}
    onApply={(s, rev) => void applyMatrix(s, rev).then(() => grid?.focus())}
    onClose={() => { matrixAsk = null; grid?.focus(); }}
  />
{/if}
{#if gapsAsk && tab}
  {@const g = gapsAsk}
  <GapsDialog
    titles={g.titles}
    hidden={hiddenColsOf(tab)}
    col={g.col}
    header={tab.header && tab.info.rows > 0}
    rows={tab.info.rows}
    onRun={runGaps}
    onCancelRun={() => { if (tab) void backend().cancelJob(tab.id); }}
    onShowRows={(s) => void showGapRows(s).then(() => grid?.focus())}
    onOpenTab={(s, saveIt) => void openGapsTab(s, saveIt).then(() => grid?.focus())}
    onCopy={(t) => void copyGaps(t)}
    onClose={() => { gapsAsk = null; grid?.focus(); }}
  />
{/if}
{#if tidyAsk && tab}
  <TidyTitlesDialog doc={tab.id} last={lastTidy} onApply={(s) => void runTidyTitles(s).then(() => grid?.focus())} onClose={() => { tidyAsk = false; grid?.focus(); }} />
{/if}
{#if zipPick}
  {@const z = zipPick}
  <ZipPickDialog list={z} onOpen={(names) => void openFromZip(z, names)} onClose={() => (zipPick = null)} />
{/if}
{#if specialAsk && tab}
  {@const t = tab}
  <GoToSpecialDialog
    selected={cellCount(blockOr(t))}
    against={t.header && t.info.rows > 0 ? t.cache.get(0, t.cell.col) || `column ${t.cell.col + 1}` : `column ${t.cell.col + 1}`}
    hasChecks={t.checks.length > 0}
    readOnly={!!t.info.read_only}
    last={lastSpecial}
    onRun={(c) => { specialAsk = false; void runSpecial(c).then(() => grid?.focus()); }}
    onClose={() => { specialAsk = false; grid?.focus(); }}
  />
{/if}
{#if shiftAsk && tab}
  {@const s = shiftAsk}
  <ShiftCellsDialog
    insert={s.insert}
    rows={blockOr(tab).rows}
    cols={blockOr(tab).cols}
    onRun={(choice) => { const insert = s.insert; shiftAsk = null; void runShift(insert, choice).then(() => grid?.focus()); }}
    onClose={() => { shiftAsk = null; void tick().then(() => grid?.focus()); }}
  />
{/if}

{#if renameAsk}
  {@const r = renameAsk}
  <RenameColumn
    number={r.col + 1}
    name={r.name}
    header={r.header}
    others={r.others}
    onRun={(name) => { const { id, col, header, titles } = r; renameAsk = null; void renameColumn(id, col, name, header, titles).then(() => grid?.focus()); }}
    onClose={() => { renameAsk = null; void tick().then(() => grid?.focus()); }}
  />
{/if}

{#if dialog}
  <Dialog request={dialog} onDone={() => { dialog = null; void tick().then(() => grid?.focus()); }} />
{/if}

<style>
  .acting {
    white-space: nowrap;
  }
  .acting.including {
    padding: 0 0.4rem;
    border-radius: 4px;
    background: rgb(217 142 4 / 0.28);
    font-weight: 600;
  }
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
  /* The item under the pointer (or picked with the arrow keys): a quiet
     tint of the accent colour, readable in the light and dark themes. */
  .item.hot:not(:disabled),
  .item:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    color: var(--text);
  }
  .item:focus-visible {
    outline: 2px solid color-mix(in srgb, var(--accent) 55%, transparent);
    outline-offset: -2px;
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
  .invnote {
    padding: 2px 8px 3px 136px;
    font-size: 12px;
    color: #c2410c;
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
    position: relative;
    display: flex;
    gap: 2px;
    padding: 4px 8px 0;
    background: var(--bg);
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    user-select: none;
    -webkit-user-select: none;
  }
  .tabs.tab-dragging {
    cursor: grabbing;
  }
  .tabs.tab-dragging .tab:not(.dragging) {
    transition: transform 0.12s ease;
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
  .tab.drop {
    background: var(--surface);
    color: var(--text);
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .tab.dragging {
    position: relative;
    z-index: 2;
    background: var(--surface);
    color: var(--text);
    cursor: grabbing;
    box-shadow:
      inset 0 2px 0 var(--accent),
      0 4px 14px rgba(0, 0, 0, 0.25);
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
    /* As tall as a line with buttons: a line that loses its buttons (the
       title-row question answered) doesn't move the table. */
    min-height: 24px;
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
    position: relative;
  }
  /* The title-row question floats over the bottom of the table: it takes no
     room, so nothing moves when it comes or goes. */
  .header-ask {
    position: absolute;
    left: 50%;
    bottom: 28px;
    transform: translateX(-50%);
    z-index: 20;
    width: max-content;
    max-width: calc(100% - 32px);
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    padding: 8px 10px 8px 14px;
    font-size: 13px;
    background: color-mix(in srgb, #e3a008 16%, var(--surface));
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 4px 16px rgb(0 0 0 / 0.18);
  }
  /* Centred over the table, not the table and the side panel. */
  .body.with-panel .header-ask {
    left: calc((100% - 320px) / 2);
    max-width: calc(100% - 352px);
  }
  .header-ask .close {
    padding: 3px 5px;
  }
  .body.with-panel {
    grid-template-columns: minmax(0, 1fr) 320px;
  }
  .splitview {
    display: grid;
    min-height: 0;
    min-width: 0;
  }
  .splitview .pane {
    display: grid;
    min-height: 0;
    min-width: 0;
  }
  .splitview .divider {
    background: var(--border, #c8c8c8);
    cursor: row-resize;
    touch-action: none;
  }
  .splitview.cols .divider {
    cursor: col-resize;
  }
  .splitview .divider:hover {
    background: var(--accent, #2f6fde);
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
