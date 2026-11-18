// The feature descriptions that command search (Alt+Q) matches against.
// They come straight from README.md (bundled at build time), so the words
// the search knows are always the words the README uses. Each README bullet
// is linked to the commands it describes in FEATURE_LINKS; a unit test fails
// when a bullet is added or reworded without a link, or a link names a
// command that does not exist.

/** One bullet of the README's feature sections. */
export interface FeatureBullet {
  /** Lookup key: the bold title, or the first words of the bullet. */
  key: string;
  /** Heading the bullet sits under, e.g. "Cleaning tools". */
  section: string;
  /** The bullet as plain text (no markdown). */
  text: string;
}

/** What command search knows about one command from the README. */
export interface FeatureDoc {
  section: string;
  /** All linked README text, for matching. */
  text: string;
  /** One short line for the result list. */
  blurb: string;
}

/** Plain text from a line of markdown. */
export function plainMarkdown(s: string): string {
  return s
    .replace(/!\[[^\]]*\]\([^)]*\)/g, "")
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/<[^>]+>/g, "")
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .replace(/\*([^*]+)\*/g, "$1")
    .replace(/`([^`]*)`/g, "$1")
    .replace(/\s+/g, " ")
    .trim();
}

/** Key for a bullet: its bold title, or its first five words, lower case. */
export function bulletKey(markdown: string): string {
  const bold = /^\*\*([^*]+)\*\*/.exec(markdown.trim());
  const base = bold ? bold[1]! : plainMarkdown(markdown).split(" ").slice(0, 5).join(" ");
  return base
    .toLowerCase()
    .replace(/[^a-z0-9 ]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

/** Sections of the README that describe features. */
const FIRST = "## Features";
const AFTER = "## Build from source";

/** Every bullet in the README's feature sections. */
export function readmeBullets(readme: string): FeatureBullet[] {
  const lines = readme.replace(/\r\n?/g, "\n").split("\n");
  const start = lines.findIndex((l) => l.trim() === FIRST);
  if (start < 0) return [];
  const out: FeatureBullet[] = [];
  let section = "Features";
  let cur: string[] | null = null;
  const flush = (): void => {
    if (cur) {
      const md = cur.join(" ");
      out.push({ key: bulletKey(md), section, text: plainMarkdown(md) });
    }
    cur = null;
  };
  for (let i = start + 1; i < lines.length; i++) {
    const l = lines[i]!;
    if (l.trim() === AFTER) break;
    const h = /^(#{2,3})\s+(.*)$/.exec(l);
    if (h) {
      flush();
      section = plainMarkdown(h[2]!);
      continue;
    }
    if (/^- /.test(l)) {
      flush();
      cur = [l.slice(2)];
    } else if (cur && /^\s+\S/.test(l)) cur.push(l.trim());
    else flush();
  }
  flush();
  return out;
}

/** README bullet key → the commands it describes. Keys are the bold title of
 *  the bullet, or its first five words (lower case, punctuation removed). */
export const FEATURE_LINKS: Record<string, string[]> = {
  // Opening and saving
  "works out the file on its own": ["file.open", "file.layout", "file.reopenLines"],
  "text safe saving": ["file.save"],
  "repairs broken files": ["file.open"],
  "title row or not": ["format.header", "format.makeHeader", "format.renameCol", "format.forgetHeader", "format.forgetHeaders"],
  "save as options": ["file.saveAs"],
  "never lose work": ["file.recover", "file.versions", "file.revert"],
  "big files": ["file.open"],
  "zipped and gzipped files": ["file.open", "file.recent"],
  "pick up where you left off": ["file.options", "file.recent"],
  // The grid
  "a spreadsheet style grid that stays": ["help.keys", "edit.repeat", "data.filter"],
  "tabs for each open file": [
    "window.next", "window.prev", "window.moveTabLeft", "window.moveTabRight", "view.freeze", "view.freezeRow", "view.freezeCol", "view.unfreeze",
    "insert.colsLeft", "insert.colsRight", "edit.deleteCols", "edit.moveColsLeft", "edit.moveColsRight",
    "format.hideCols", "format.showCols", "format.autofit", "format.hideRows", "format.showRows", "format.showAllRows",
  ],
  "drag to move and arrange": [
    "help.mouse", "edit.moveColsLeft", "edit.moveColsRight", "edit.moveRowsUp", "edit.moveRowsDown",
    "format.autofit", "format.showCols", "view.freeze",
  ],
  "light dark or system theme": ["view.themeSystem", "view.themeLight", "view.themeDark", "view.fontOther", "view.zoomIn", "view.zoomOut", "view.zoomReset"],
  "alternating row colours wrapped text": ["view.banded", "format.wrap", "format.rowHeight", "format.fitRows"],
  "display formats": ["format.display", "format.clearDisplay"],
  "value types shown in colour": ["view.valueTypes", "view.scale3", "view.scale2", "view.bars"],
  "ctrl click opens web and e mail": [],
  "command search": ["help.search"],
  "custom shortcuts ctrl for the": ["help.shortcuts", "help.keys", "view.panelFiles", "view.panelActivity", "file.recent"],
  "show invisible characters": ["view.invisible", "view.cellBar"],
  "protect columns": ["format.protectCols"],
  // Editing
  "edit in the cell or": ["edit.cell", "view.cellBar", "edit.goto"],
  "go to special": ["edit.gotoSpecial", "edit.goto"],
  "tidy column titles": ["format.tidyTitles", "format.renameCol"],
  "formulas in cells": ["edit.cell", "view.cellBar", "data.compute"],
  "fill handle fill series ctrl enter": ["edit.fillSeries", "edit.fillDown", "edit.fillRight", "edit.pasteSpecial", "insert.symbol"],
  "type ahead suggestions": ["view.typeahead", "edit.pickList", "view.listArrows", "edit.pickDate", "view.spellcheck"],
  "cut and paste that moves": ["edit.cut", "edit.paste", "edit.moveCut", "edit.cancelCut"],
  "undo and redo for everything": ["edit.undo", "edit.redo"],
  "the active cell stays put": [],
  "typing moves like excel": ["edit.cell"],
  "insert and delete cells": ["insert.cellsAsk", "edit.deleteCellsAsk", "insert.smart", "edit.deleteSmart"],
  "clear or fill big selections": ["edit.clear"],
  "looks like a spreadsheet": ["view.alignNumbers", "format.alignLeft", "format.alignCenter", "format.alignRight", "format.alignAuto", "format.clip", "view.colLetters"],
  "status bar totals": ["view.panelSummary"],
  "go to and the name box": ["edit.goto"],
  "more excel and google sheets keys": ["edit.contextMenu", "format.display", "format.asNumber", "format.asDate", "format.asCurrency", "format.asPercent", "format.clearDisplay", "edit.copyAbove", "edit.today", "edit.now", "edit.selectAll", "edit.selectRegion", "view.toActive", "edit.shrinkSel", "edit.nextCorner", "edit.extendMode"],
  "right click menus say how many": ["edit.contextMenu"],
  "split view": ["view.splitRows", "view.splitCols"],
  "name your tabs": ["file.rename"],
  "paste into filtered rows": ["edit.paste", "edit.pasteSpecial", "data.filterBySel"],
  "file tabs": ["file.closeOthers", "file.closeRight", "file.reopenClosed", "file.closeUnchanged", "window.next", "window.prev"],
  "set column width types an": ["format.colWidth", "format.autofit", "format.showCols", "format.showAllCols"],
  "where enter goes": ["file.options"],
  "copy and paste like excel": ["edit.copy", "edit.paste"],
  "every dialog answers enter with": ["file.options"],
  "one row per page form view": ["data.form"],
  "list of my changes": ["edit.changes", "view.showEdits"],
  "notes and coloured flags on rows": ["edit.note"],
  "read only lock for a file": ["file.readOnly"],
  // Find, sort and filter
  "find and replace with plain": ["find.find", "find.replace", "tools.nextChange", "tools.largest", "tools.smallest"],
  "whole word ignore accents and special characters": ["find.find", "find.replace"],
  "match count and matches lit up": ["find.find"],
  "find all list": ["find.find"],
  "replace all preview of every change": ["find.replace"],
  "right click a cell to find its value": ["find.thisValue", "find.replaceValue", "find.allInColumn"],
  "find tips and recent searches": ["find.find"],
  "find blanks special characters column titles and notes": ["find.find", "find.replace"],
  "search all open tabs or chosen columns": ["find.find", "find.replace"],
  "act on the rows find all found": ["find.find"],
  "replace from a list": ["find.batch", "find.replace"],
  "untick changes and show changed rows": ["find.replace"],
  "find numbers dates and similar spellings by value": ["find.find", "find.replace"],
  "preserve case hidden rows and search by columns": ["find.find", "find.replace"],
  "copy the list and saved searches": ["find.find"],
  "search a whole folder": ["tools.searchFolder"],
  "sort by several columns numbers": ["data.sort", "data.sortAsc", "data.sortDesc", "tools.flip", "data.shuffle", "data.sortColsAsc", "data.sortColsDesc"],
  "quick filter a filter builder": ["data.filter", "data.filterMenu", "data.filterBySel", "data.clearFilter"],
  "saved views": ["view.saveView", "view.applyView", "view.saveViewFile", "view.openViewFile"],
  "find duplicate rows and remove": ["data.removeDuplicates", "data.showDuplicates"],
  "column lists start empty": ["data.removeDuplicates", "data.showDuplicates", "data.mergeRows", "data.subtotals", "data.group", "tools.fillBlanks"],
  // Seeing what is in the data
  "summary of each column": ["view.panelSummary"],
  "counts tab": ["data.counts"],
  "column profile": ["data.profile"],
  "filter panel": ["view.panelFilters"],
  "column insights": ["view.insights", "data.profile"],
  "quick totals": ["view.panel"],
  "charts line bar pie counts": ["view.panelChart", "data.subtotals", "view.highlights"],
  "compare two open files": ["tools.compare"],
  "compare two columns": ["data.compareCols"],
  "random and audit samples": ["tools.sample"],
  "value editor": ["data.values"],
  // Checking data
  "data checks": ["data.checks", "data.unmarkChecks", "data.clearChecks"],
  "check fields with patterns": ["data.fieldCheck"],
  "clean up suggestions": ["clean.suggest"],
  // Import and export
  "open": ["file.import", "file.importClipboard"],
  "sqlite": ["file.openSqlite", "file.saveSqlite"],
  "export": ["file.export"],
  "json lines": ["file.import", "file.export"],
  "split one file into many": ["tools.split", "tools.combine"],
  "excel files made for excel users": ["file.export"],
  "more rows than excel holds": ["file.export", "tools.split"],
  "several open tabs as one workbook": ["file.export"],
  "append": ["data.append"],
  "encrypted copies": ["file.saveEncrypted", "file.openEncrypted"],
  // Joining two lists
  "add new columns or update": ["data.join"],
  "keep every row only matched": ["data.join"],
  "repeated keys": ["data.join"],
  "loose keys": ["data.join"],
  "nearest key": ["data.join"],
  "text for rows not found": ["data.join"],
  "lookup inside computed columns and": ["data.compute", "data.join"],
  // Matching people and addresses
  "match people": ["data.match"],
  "street range lookup": ["data.street"],
  "take changes from a newer file": ["tools.updates"],
  // Reshaping tables
  "split and merge columns and": ["data.splitColumn", "data.mergeColumns", "data.splitRows", "data.mergeRows", "data.unpivot", "tools.reshape", "tools.stack"],
  "smarter split": ["data.splitColumn"],
  "extract": ["data.extract"],
  "choose and order columns": ["data.arrange"],
  "computed columns": ["data.compute", "data.changeFormula"],
  "change a column with a formula": ["data.changeFormula", "data.compute"],
  "filter by formula": ["data.filterFormula", "data.clearFilter"],
  "build a column without a formula": ["data.build"],
  "group figures": ["data.group"],
  "remove blank rows and columns": ["data.removeBlankRows", "data.removeBlankCols", "tools.deleteHidden", "tools.deleteHiddenCols", "tools.breaks", "view.divider"],
  "fill blank cells from above": ["tools.fillBlanks", "tools.countRepeats", "tools.blankRepeats"],
  // Pivot tables
  "pivot table builder": ["data.pivot"],
  "refresh": ["data.pivotRefresh"],
  "count or add up any combination": ["data.pivot"],
  "group dates and numbers": ["data.pivot"],
  "filter and sort items": ["data.pivot"],
  "totals and show values as": ["data.pivot"],
  "expand collapse and drill down": ["data.pivot"],
  "use the result": ["data.pivot"],
  // Cleaning tools
  "replace text dates and times": [
    "clean.tools", "clean.tool.replace", "clean.tool.dates", "clean.tool.numbers", "clean.tool.add", "clean.tool.pad",
    "clean.tool.remove", "clean.tool.reverse", "clean.tool.random", "clean.tool.spaces", "clean.tool.digits", "clean.tool.now",
  ],
  "change case including proper name": ["clean.tool.case", "clean.tool.name", "data.splitName"],
  "remove accents invisible characters and": ["clean.tool.accents", "clean.tool.html_encode", "clean.tool.html_decode", "clean.tool.html_text", "clean.tool.url_encode", "clean.tool.url_decode"],
  "fix numbers stored as text": [
    "clean.tool.fix_number", "clean.tool.calc", "clean.tool.sort_parts", "clean.tool.words", "clean.tool.random_number",
    "clean.tool.number", "format.bakeDisplay",
  ],
  "dates read in a layout": ["clean.tool.dates"],
  "address zip phone and e mail tidy": ["clean.tool.address", "clean.tool.zip", "clean.tool.phone", "clean.tool.email"],
  "fill by example": ["data.fillExample"],
  "masking": ["clean.tool.mask", "clean.tool.date_band", "clean.tool.scramble"],
  // Repeating your work
  "cleaners": ["clean.tools"],
  "recipes": ["tools.recipe"],
  "recipe steps panel": ["view.panelSteps", "tools.recipe"],
  "recipe questions": ["view.panelSteps", "tools.recipe", "tools.recipeFiles"],
  "run recipe on files": ["tools.recipeFiles", "tools.watch"],
  "watched folder": ["tools.watch"],
  // Printing, PDF and labels
  "print with page setup and": ["file.print"],
  "save as pdf": ["file.pdf"],
  "mailing labels": ["file.mail"],
  // Working with files and folders
  "opens files from double click open": ["file.open"],
  "can be the default app": ["file.options"],
  "save all revert rename show": ["file.saveAll", "file.revert", "file.rename", "file.showFolder", "file.copyPath", "file.closeUnchanged"],
  // Privacy-safe defaults
  "no internet": [],
  "agent link is off": ["file.options"],
  "log files never hold cell contents": ["help.logs"],
  "private data warnings are on": ["tools.findPrivate"],
  "share safe copy": ["file.safeCopy"],
  "private file mode": ["file.private", "file.privateFolder"],
  "encrypted files": ["file.openEncrypted", "file.saveEncrypted"],
};

/** Command whose README bullet should give its description when several
 *  bullets describe it. */
export const LEAD: Record<string, string> = {
  "data.join": "add new columns or update",
};

/** Commands whose README text is shown but not searched (command search's
 *  own bullet lists example words that belong to other tools). */
export const SHOW_ONLY = new Set(["help.search"]);

/** Split into sentences at ". ", "! " or "? ", never inside brackets. */
export function sentences(t: string): string[] {
  const out: string[] = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < t.length; i++) {
    const c = t[i]!;
    if (c === "(") depth++;
    else if (c === ")") depth = Math.max(0, depth - 1);
    else if (depth === 0 && (c === "." || c === "!" || c === "?") && t[i + 1] === " ") {
      out.push(t.slice(start, i + 1).trim());
      start = i + 2;
    }
  }
  out.push(t.slice(start).trim());
  return out.filter(Boolean);
}
const wordsOf = (t: string): string[] => t.toLowerCase().match(/[a-z0-9]+/g) ?? [];

/** Shorten to at most `max` characters at a word break. */
export function clip(t: string, max = 120): string {
  if (t.length <= max) return t;
  const cut = t.slice(0, max - 1);
  const sp = cut.lastIndexOf(" ");
  return `${(sp > max / 2 ? cut.slice(0, sp) : cut).replace(/[,;:\s]+$/, "")}…`;
}

/** The sentence of `text` that says most about a command called `label`. */
export function bestSentence(text: string, label: string): string {
  const want = new Set(wordsOf(label).filter((w) => w.length > 2));
  let best = "";
  let bestScore = -1;
  for (const s of sentences(text)) {
    const score = wordsOf(s).filter((w) => want.has(w)).length;
    if (score > bestScore) {
      best = s;
      bestScore = score;
    }
  }
  return best;
}

/** Command id → what the README says about it. `labelOf` gives the
 *  command's own name, used to pick the most fitting sentence. */
export function featureDocs(bullets: FeatureBullet[], links: Record<string, string[]>, labelOf: (id: string) => string): Map<string, FeatureDoc> {
  const parts = new Map<string, FeatureBullet[]>();
  for (const b of bullets)
    for (const id of links[b.key] ?? []) {
      const list = parts.get(id) ?? [];
      list.push(b);
      parts.set(id, list);
    }
  const out = new Map<string, FeatureDoc>();
  for (const [id, bs] of parts) {
    const label = labelOf(id);
    // The bullet that names the command best gives the short description.
    let lead = bs.find((b) => b.key === LEAD[id]) ?? bs[0]!;
    let leadScore = LEAD[id] && lead.key === LEAD[id] ? Infinity : -1;
    const want = new Set(wordsOf(label).filter((w) => w.length > 2));
    for (const b of bs) {
      const score = wordsOf(bestSentence(b.text, label)).filter((w) => want.has(w)).length;
      if (score > leadScore) {
        lead = b;
        leadScore = score;
      }
    }
    out.set(id, {
      section: lead.section,
      text: SHOW_ONLY.has(id) ? "" : bs.map((b) => `${b.section} ${b.text}`).join(" "),
      blurb: clip(bestSentence(lead.text, label)),
    });
  }
  return out;
}
