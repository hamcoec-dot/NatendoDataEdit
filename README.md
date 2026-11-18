# Natendo Data Edit

A Windows desktop editor for table files: CSV, TSV and files split by any
other character, plus fixed-width text, log files, spreadsheets and SQLite
tables. It opens a 10-million-row file in about a second, lets you edit it
like a spreadsheet, and saves it back without disturbing the rows you did not
touch.

- **No row limits, no ads, no editions, no account.**
- **Works offline.** The app never connects to the internet.
- **Fast on huge files.** 96 speed checks on a 10-million-row file all pass
  (see [Speed](#speed)).
- Built with a Rust engine and a web-style screen on Tauri 2. All code, names,
  icons and wording are original.

![Natendo Data Edit showing a customer list with data bars and the column summary panel](docs/screenshots/main-window.png)
*A 5,000-row customer list with data bars on Monthly spend and the column summary beside it. All screenshots use made-up sample data.*

## Contents

- [Install](#install)
- [Speed](#speed)
- [Features](#features)
  - [Opening and saving](#opening-and-saving)
  - [The grid](#the-grid)
  - [Editing](#editing)
  - [Find, sort and filter](#find-sort-and-filter)
  - [Seeing what is in the data](#seeing-what-is-in-the-data)
  - [Checking data](#checking-data)
  - [Import and export](#import-and-export)
  - [Joining two lists](#joining-two-lists)
  - [Matching people and addresses](#matching-people-and-addresses)
  - [Reshaping tables](#reshaping-tables)
  - [Pivot tables](#pivot-tables)
  - [Cleaning tools](#cleaning-tools)
  - [Repeating your work](#repeating-your-work)
  - [Printing, PDF and labels](#printing-pdf-and-labels)
  - [Working with files and folders](#working-with-files-and-folders)
- [Privacy-safe defaults](#privacy-safe-defaults)
- [Build from source](#build-from-source)
- [Command line (`natendo-cli`)](#command-line-natendo-cli)
- [Agent link](#agent-link)
- [Known limits](#known-limits)

## Install

1. Download `Natendo.Data.Edit_0.4.1_x64-setup.exe` from the
   [Releases page](https://github.com/hamcoec-dot/NatendoDataEdit/releases).
2. Run it. It installs for your Windows account only, so it does not need
   administrator rights.
3. Setup asks whether table files (`.csv`, `.tsv`, `.tab`, `.psv`, `.dat` and
   similar) should open in Natendo Data Edit, and separately whether plain
   `.txt` files should. You can change this later in File > Options.

The installer is not code-signed yet, so Windows SmartScreen shows "Windows
protected your PC" the first time. Click **More info**, then **Run anyway**.

Needs 64-bit Windows 10 or 11. The screen uses Microsoft Edge WebView2, which
Windows 11 already has; setup fetches it on older systems if it is missing.

## Speed

Every build is timed against a generated test file of **10 million rows,
1.59 GB**, with a time limit for each job. Results measured on 13 November
2026, on a PC with an AMD Ryzen 9 9955HX (16 cores)
and 32 GB of memory: **96 of 96 checks passed.**

Opening and moving around

| Job (10 million rows) | Time |
|---|---|
| First rows on screen | 0.03 s |
| Whole file indexed and ready | 1.3 s |
| Scrolling: fetching a screenful of cells | 0.1 ms (slowest 1 in 100) |
| Memory for the index | 40 MB (2.5% of the file) |
| Jump to the end of a column (Ctrl+Down) | 0.07 s |

Sorting, filtering, finding

| Job (10 million rows) | Time |
|---|---|
| Sort by a number column | 0.24 s |
| Sort by a text column (natural order, so 2 before 10) | 1.3 s |
| Filter one column | 0.13 s |
| Find every match of a pattern in a column | 0.10 s |
| Replace all in a column | 0.57 s |
| Find duplicate rows by one column | 0.61 s |
| Remove those duplicates | 0.93 s |
| Shuffle every row | 0.11 s |
| Totals of a whole column | 0.14 s |

Saving

| Job (10 million rows) | Time |
|---|---|
| Save after editing 1,000 rows | 3.2 s |
| Save after sorting and Replace All | 3.7 s |
| Save after adding and moving columns | 6.0 s |
| Save after a 3-column join | 8.2 s |
| Save As with every value quoted | 9.2 s |

Bigger jobs

| Job | Time |
|---|---|
| Compare two 10M-row files by key | 10.0 s |
| Join a 10M-row file to another 10M-row file, 3 columns: plan and apply | 12.4 s |
| Join 1M rows to a 100,000-row list by one key | 0.12 s |
| Find near-duplicate people within 10M rows | 29.1 s |
| Match 1M people against a 10,000-person list | 0.96 s |
| Street range lookup for 10M addresses (31,100 ranges) | 3.0 s |
| Take changes from a newer copy, 10M rows | 4.7 s |
| Pivot 10M rows: city and flag by month, sum + row count + distinct count | 1.5 s |
| Pivot quick preview (first 200,000 rows) | 0.05 s |
| Pivot: change sorting, totals, show-as or formats | 0.01 s |
| Pivot with 3 million items (name by city), sum + median | 4.6 s |
| Pivot drill-down to one city's 32,000 rows | 0.26 s |
| Computed column on every one of 10M rows | 6.2 s |
| Share-safe copy of 10M rows (mask, scramble, age band) | 2.5 s |
| Search a folder holding the 10M- and 1M-row files | 10.3 s |
| Recipe of 3 column steps on 10M rows | 1.1 s |
| Export 1M rows to Excel / read them back | 4.4 s / 9.1 s |
| Save 1M rows to SQLite and open them again | 3.0 s |
| Encrypt 1M rows and open them again | 1.3 s |
| PDF of 100,000 rows | 1.8 s |
| Mailing labels for 1M rows (30 per page) | 7.5 s |

The speed checks live in `crates/engine/examples/perf.rs`; run them with
`cargo run --release -p natendo-engine --example perf`.

## Features

A look at three of the menus: almost every tool is one click away, and
everything is also in command search.

<table>
<tr>
<td width="33%"><img src="docs/screenshots/menu-data.png" alt="The Data menu: sorting, filters, duplicates, split and merge, pivot, joins, match people, street range lookup and more"></td>
<td width="33%"><img src="docs/screenshots/menu-clean.png" alt="The Clean menu: dozens of cleaning steps from replace text to phone, ZIP and e-mail tidy"></td>
<td width="33%"><img src="docs/screenshots/menu-tools.png" alt="The Tools menu: fill blanks, compare files, newer-file changes, split and combine files, folder search, recipes, watched folder, private data"></td>
</tr>
<tr><td align="center"><b>Data</b></td><td align="center"><b>Clean</b></td><td align="center"><b>Tools</b></td></tr>
</table>

### Opening and saving

- **Works out the file on its own:** the separator (comma, tab, semicolon,
  pipe, runs of spaces, control characters, even several-character
  separators), the quoting style, the text encoding (UTF-7, UTF-8, UTF-16,
  UTF-32 and older Windows code pages) and the line breaks (Windows, Unix,
  old Mac, mixed). It skips notes and `sep=` lines at the top, and when it is
  unsure it shows its confidence and the runner-up choices with a preview.
- **Text-safe saving:** rows you did not edit are written back byte for byte,
  so a save never quietly changes dates, leading zeros or quoting elsewhere in
  the file.
- **Repairs broken files:** stray or unclosed quotes and rows with too many or
  too few fields are fixed, and each repair is listed.
- **Title row or not:** the app guesses whether the first row holds titles;
  one switch changes it, and files without titles get column numbers.
  Rename a column by double-clicking its heading, right-clicking it, or
  Format > Rename column (a file without titles gets a title row). Data
  check rules that name the column ([first_name]) follow the new name (or
  its number, [#2], when another column has that name too), and undo puts
  both back. A saved pivot layout or view that names a column this file no
  longer has asks before using the column in the same place, and the view's
  rules follow. A rename is a recipe step, so recipes replay it.

![Renaming the first_name column after double-clicking its heading](docs/screenshots/rename-column.png)
*Double-click a heading to rename the column.*

- **Save As options** with a live preview: separator, quoting (as is, only
  where needed, every value, text but not numbers), quote character, encoding
  and line breaks.
- **Never lose work:** automatic recovery copies after a crash, a list of
  earlier saves to go back to, Revert, and saves written to a temporary file
  first and then moved into place.
- **Big files:** opens files of several gigabytes; every long job shows
  progress and can be stopped.
- **Zipped and gzipped files:** open `voters.csv.gz` or `export.zip` without
  unpacking them first. A zip holding several tables lists them to pick from
  (or open them all, one tab each). Everything is unpacked in memory only, so
  no unpacked copy of the data is left on disk (as with any file, unsaved
  changes are kept in a recovery copy until you save or close; private file
  mode turns that off). A `.gz` file saves back gzipped, fast, using every
  core; a table from a zip opens as a new tab and Save asks where to save
  it (the zip itself is never changed). Sizes are capped, and a file that
  would unpack to hundreds of times its size is refused.
- **Pick up where you left off:** at start, the app offers to reopen the
  files that were open last time, each at the same cell with its hidden
  columns, widths and protected columns. A file changed since opens fresh
  and says so. Only where each file is and how it was laid out are kept,
  never anything from the cells (so filters and highlight rules, which can
  hold names from the file, are not kept either); files in private file mode, encrypted files
  and tabs never saved are not remembered. File > Options sets it to offer
  (the default), reopen without asking, or never remember.

### The grid

![The grid in the dark theme with value types shown in colour](docs/screenshots/dark-theme.png)
*Dark theme with a violet accent; numbers show in blue and dates in green (Ctrl+F8).*

- A spreadsheet-style grid that stays smooth at millions of rows, with
  **Excel's usual keyboard shortcuts** (F2, F4 to repeat, Ctrl+arrows,
  Ctrl+Shift+L for filters, Alt menu keys and the rest).
- **The active cell stays put**, as in Excel and Google Sheets: after
  Shift+arrows, Ctrl+Shift+arrows, Shift+click, Shift+Space or Ctrl+Space
  the cell you started from stays active (typing goes there), and a plain
  arrow key then moves on from it. Ctrl+Shift+Down then Down lands one row
  below where you started, not at the bottom.

![A block of cells selected with Shift+arrows; the active cell is still the one it started from](docs/screenshots/active-cell-stays.png)
*Shift+Down three times and Shift+Right twice: Jo, where it started, is still the active cell (R11 C2).*

- **Looks like a spreadsheet:** numbers and dates sit on the right (View >
  Numbers on the right), and Format > Align left, center, right or
  automatically sets any column. Long text spills into the empty cells beside
  it instead of being cut off; Format > Cut off long text keeps a column's text
  inside it, and Wrap text wraps it. View > Column letters adds A, B, C to the
  headings. A small arrow marks the column the rows are sorted by (until the
  sort is undone); with the filter buttons off, pointing at a heading shows an
  arrow that opens the column's menu; pointing at a link shows its address.

![Numbers lined up on the right, and a long note spilling across the empty cells beside it](docs/screenshots/text-spill.png)
*Amounts and dates on the right; a long note in row 4 runs on over the empty cells next to it.*

- **Status bar totals:** select cells and the status bar shows Sum and Count
  side by side (just Count when there are no numbers). Click it to tick
  Average, Min, Max, Numbers, Distinct or Characters as well, or to copy any
  one of them.

![The status bar showing Sum, Average and Count of a selection](docs/screenshots/status-totals.png)
*Some amounts selected (one cell is empty), with Average ticked as well as Sum and Count.*

- **Go To and the name box** take a row ("250"), a row and column ("R250 C3"
  or "250, 3"), Excel's letters ("F250") or a column title with or without a
  row ("city 250"). While a filter is on, rows are found by the numbers shown
  on screen (the whole file's), and the name box shows them too.
- **More Excel and Google Sheets keys:** Shift+F10 or the Menu key opens the
  right-click menu; Ctrl+1 opens Display format; Ctrl+Shift+1, 3, 4 and 5 show
  columns as number, date, currency or percent, and Ctrl+\ removes the
  format; Ctrl+' starts an edit with the value of the cell above; Ctrl+; and
  Ctrl+Shift+; start an edit with today's date or the time (add to it, or Esc);
  Ctrl+A selects the block of data around the cell first and everything on
  the second press (Ctrl+Shift+* selects the block); Ctrl+Backspace scrolls
  back to the active cell, Shift+Backspace selects just it, and Ctrl+. moves it
  round the corners of the selection; F8 turns on extend mode (arrows grow the
  selection); Alt+PageDown and Alt+PageUp move a screen sideways; Space ticks
  or unticks a tick box; Esc keeps the selection; Alt+/ opens command search
  as well as Alt+Q. Home, End and Ctrl+arrows skip hidden columns, and
  double-clicking the edge of the selection jumps that way like Ctrl+arrow.
- **Right-click menus say how many** ("Insert 3 rows above", "Delete rows
  5–7"). The cell menu also has Edit cell, Pick from a list, Note and flag,
  Hide rows and columns, Show my edits and List of my changes; the column
  menu has Set column width… and Column summary. In every menu, the item
  under the pointer (or picked with the arrow keys) is lightly highlighted.

- Tabs for each open file, frozen rows and columns, insert, delete, move,
  hide, resize and auto-fit columns, hide and show rows (Ctrl+9,
  Ctrl+Shift+9).
- **Drag to move and arrange**, as in Google Sheets: reorder or rearrange
  columns by dragging their selected headings, or rows by their selected
  numbers; drag a selected block of cells by its outline to move it (Ctrl+drag
  copies it, Esc stops a drag). Resize or double-click-fit several selected
  columns at once, click the small arrow beside hidden columns to show them
  again, drag the edges of the top-left corner to freeze rows or columns, and
  drag file tabs into another order (the tab follows the pointer and the
  others slide aside to show where it will land; Esc puts it back;
  Ctrl+Alt+Shift+PageUp/PageDown moves the shown tab from the keyboard). Each move is one undo step, and colour
  scales, highlight rules and checks move with their columns. Help > Mouse
  and drag moves lists them all.
- **Name your tabs.** Tabs made by a tool are named after what made them,
  for example "Pivot – city by party (voters)", "Subtotals (voters)" or
  "Compare voters with voters-2024" (a drill-down tab is named after the
  fields, such as "Rows for one city (voters)", never a value). Double-click a
  tab that is not saved yet (or right-click › Rename, or F2 on it) to give it
  your own name; Save then suggests that name for the file.
- **File tabs** have a right-click menu (Save, Rename, Copy path, Show in
  File Explorer, move left or right, Close, Close other tabs, Close tabs to
  the right, Close unchanged tabs). Double-click the empty part of the tab
  bar for a new file, and Ctrl+Shift+T opens the last closed file again.
  Ctrl+PageDown and Ctrl+PageUp switch tabs (Ctrl+Shift+PageDown and
  Ctrl+Shift+PageUp too, as in Google Sheets).
- Set column width… types an exact width; AutoFit (or a double-click on a
  column's edge) also measures the first and last 2,000 rows of the file, so
  a long value off screen fits too. Ctrl+Shift+0 shows the hidden columns
  inside the selection (Unhide all columns shows every one).
- Light, dark or system theme with an accent colour, font and font size, zoom.
- Alternating row colours, wrapped text, row height in lines.
- **Display formats** per column (numbers, percent, currency, dates, tick
  boxes) that change only how cells look, never the file.
- Value types shown in colour (Ctrl+F8), colour scales and data bars.
- Ctrl+click opens web and e-mail addresses.
- **Command search** (Alt+Q, the same key as Excel's search box; change it
  under Help > Customize shortcuts): type a few letters or words and every
  matching tool shows at once, with the menu it lives in and a line from this
  page saying what it does. Everyday words work too ("dedupe", "vlookup",
  "trim", "mail merge"), and small typing slips are forgiven. Arrows and Enter
  or a click open the tool; Esc or Alt+Q again closes.
- Custom shortcuts, Ctrl+/ for the list of shortcuts (with a Mouse section:
  every drag and click move), a file browser panel, an activity panel, and a
  start page with recent files. File > Open recent (Ctrl+Shift+O) shows the
  same list at any time: tick two or three files and each opens in its own
  tab, or middle-click files one after another to open each in a new tab
  while the list stays open (middle-clicking a file on the start page works
  the same way). Up and Down move through the list and Enter opens. Files
  that are already open are marked "open" and switched to, never opened
  twice; two files with the same name show enough of their folders to tell
  them apart (2024\Data, 2025\Data), in the list and on their tabs.

![The Open recent list with three files open and two voters.csv files told apart by folder](docs/screenshots/open-recent.png)
*File > Open recent after a few middle-clicks: each opened in its own tab and is marked "open".*

![Dragging the city column by its heading to a new place](docs/screenshots/drag-column.png)
*Dragging "city" by its heading: a see-through band follows the pointer and the bar shows where it will land.*

- **Show invisible characters** (View > Show invisible characters,
  Ctrl+Shift+H): spaces show as a faint `·`, tabs as `→` and line breaks as
  `⏎`; non-breaking, zero-width and other odd spaces show in orange
  (`°`, `¦`), the usual reason two names that look the same won't match.
  Under the cell bar a line says what the cell holds that can't be seen
  ("2 spaces at the end, 1 non-breaking space"). Display only: nothing in the
  file changes.

![Show invisible characters: dots for spaces and an orange mark for a non-breaking space](docs/screenshots/invisible.png)
*Show invisible characters: a double space, a space at the end and a non-breaking space, which the line under the cell bar names.*

- **Protect columns** (right-click a heading > Protect column, or Format >
  Protect column): typing, paste, fill, Replace, Go to special, cleaning
  tools and other changes pass a protected column over, and the status bar
  says how many cells were left as they were; a change of only protected
  cells is refused. Sorting, filtering and renaming still work. A small
  padlock shows in the heading; protection follows the column when columns
  move and is saved with views.

### Editing

![The first_name column cut (dashed outline) and the right-click menu offering Move what was cut here](docs/screenshots/cut-move.png)
*first_name is cut (dashed outline); right-clicking city offers "Move what was cut here", which puts it before city in one step.*

- **Cut and paste that moves**, as in Excel and Google Sheets: Ctrl+X marks
  whole columns, whole rows or a block of cells with a dashed outline and
  changes nothing yet; pick a column (or row, or cell) and Ctrl+V moves them
  there in one undo step. Cut columns go in before the column picked and cut
  rows above the row picked, and the gap they leave closes up, so there is no
  empty column to insert or delete by hand. Esc cancels. The right-click menus
  of cells, row numbers and column headings have Cut, Copy, Paste, Move what
  was cut here and Cancel cut.
- **Go to special** (Edit > Go to special, Ctrl+Shift+G, or right-click a
  cell): pick the blank cells, the cells with numbers or text, the cells that
  differ from the active cell's column in each row ("row differences"), the
  cells changed since the file was opened or saved, or the cells that fail a
  data check, in the selection or the whole table. Then light them up (and go
  to the first), fill them all with one value ("fill every blank in these
  three columns with UNKNOWN"), or clear them, in one undo step.

![Go to special picking the blank cells of the selection to fill them with UNKNOWN](docs/screenshots/go-to-special.png)
*Go to special: every blank cell in the selection gets UNKNOWN, as one undo step.*

- **Tidy column titles** (Format > Tidy column titles, or right-click a
  heading): trim and collapse spaces, one case style (Title Case,
  lower_with_underscores, UPPER, lower), remove odd characters, and make
  repeated titles unique (Phone, Phone 2) and empty ones named. A preview
  lists every title that changes; data check rules follow the new names; one
  undo step, and a recipe step.
- **Paste into filtered rows**: while a filter is on or rows are hidden,
  Paste, Paste Special, fill, Ctrl+Enter, Delete and Copy work on the rows you
  can see, and hidden rows are left as they were (the status line says how
  many). A paste taller than the rows showing from there down asks first:
  fill the shown rows and stop, add the rest as new rows at the end, or
  cancel. No choice ever writes over a hidden row. Hidden columns are passed
  over the same way: a paste skips them, clear and fill leave them alone, and
  Copy leaves them out, so a hidden ID or birth date column never reaches the
  clipboard.

![The question when pasting 5 rows with only 3 rows showing: fill the shown rows and stop, add the rest at the end, or cancel](docs/screenshots/paste-taller-than-filter.png)
*Pasting more rows than are showing asks first, with the real numbers.*

- Edit in the cell or in a larger, multi-line edit box; a cell bar with a
  "Go to" box. Enter in the cell bar puts the value in and moves down, as in
  Excel's formula bar (Ctrl+Enter puts it into every selected cell).
- **Where Enter goes:** File > Options > Editing chooses down, right, up,
  left or stay put after Enter (Shift+Enter goes the other way).
- **Typing moves like Excel:** after typing in a cell, an arrow key keeps
  what you typed and moves on (F2, or a double-click, edits inside the text
  instead, and F2 switches between the two). Enter and Tab move through a
  selected block without losing it, wrapping at its edges, and Tab, Tab,
  Enter goes to the next row in the column you started from.

![A block of cells selected; Enter moved the active cell down inside it](docs/screenshots/enter-in-block.png)
*Typing into a selected block: Enter moves to the next cell of the block, which stays selected.*

- **Insert and delete cells** as in Excel: Ctrl+Plus or Ctrl+Minus on part
  of a row asks whether to shift the other cells (right or down when
  inserting, left or up when deleting) or take the entire rows or columns;
  whole rows or columns selected go straight away. The cell right-click menu
  has Insert cells… and Delete cells… too.

![The Delete box offering Shift cells left, Shift cells up, Entire rows and Entire columns, with a warning under Shift cells up](docs/screenshots/delete-cells.png)
*Ctrl+Minus on two rows of two columns, with Shift cells up picked: the box warns that values move into other rows.*

- **Clear or fill big selections:** Delete clears and Ctrl+Enter fills a
  selection of up to 5 million cells, or whole columns of any length, as one
  undo step with a progress bar; whole columns down to the last row (with no
  filter on) clear at once however many rows there are, and List of my
  changes shows them as cleared.
- **Copy and paste like Excel:** Copy marks the copied cells with a dashed
  outline until Esc or the next change, and a copied block pasted into a
  bigger selection that it fits a whole number of times repeats to fill it
  (two cells into ten, a 2×2 block into 4×4).
- Fill handle, fill series, Ctrl+Enter to fill a whole selection, Paste
  Special (values only, swap rows and columns), special characters. Dragging
  the fill handle back inside the selection clears the cells it passes over.
- **Type-ahead suggestions** from the column's own values, **pick lists**
  (Alt+Down) of the values already in a column, as Excel's "Pick from
  drop-down list" (columns with a list check offer their list instead; View >
  List arrows on every cell shows its arrow on each cell), a calendar pop-up
  for dates, spell check.

![Alt+Down on a city: a list of the cities already in the column](docs/screenshots/pick-values.png)
*Alt+Down in the city column lists the cities already in it; arrows and Enter pick one.*
- Undo and redo for everything, including big jobs on millions of rows,
  in the order you did things: edits and every tool, sorts, **filters**
  (Filter, Show duplicates, show rows with problems or notes, Clear filter),
  hiding rows and columns, column widths and AutoFit, display formats and
  wrapping, row height, highlight rules, colour scales, banding, the group
  line and data checks. Applying a saved view is one step. A deleted column
  comes back with its width. Filters
  and the look of the table do not count as unsaved changes. Undo and Redo
  select the cells that changed and say what was undone or redone ("Undid
  typing in R9 C3", "Undid “Paste”").
- One-row-per-page form view.
- **List of my changes** (Edit menu): every cell that differs from the saved
  file (was / now), rows and columns added, deleted or moved; click to jump
  there, or save the list as a report. View > Show my edits marks changed
  cells.
- **Notes and coloured flags on rows** (Shift+F2). They are kept in a small
  file beside the table, so the table itself never changes, and they follow
  their rows through sorts, filters and newer copies of the file. Filter to
  rows with a note or one flag colour. Notes belong to a row, not to one cell
  (a CSV file has no place for cell notes).
- Every dialog answers Enter with its main button and Esc with Cancel.
- Read-only lock for a file you only want to look at.

### Find, sort and filter

![The filter menu on the City column, listing each city with its count](docs/screenshots/filter.png)
*Filter by value: every city with how many rows it has, plus sorting and conditions.*

- Find and replace with plain text, wildcards or patterns (regular
  expressions); next change, largest and smallest jumps. With several cells
  selected, Find and Replace keep to that block (Find next moves inside it and
  Replace all changes only it), as in Excel. The Search in box names the
  block (for example "Selection A3:C7"); select another block and press
  Ctrl+F or Ctrl+H again to search that one.
- **Whole word, ignore accents and special characters** in Find: "Ann" can
  find "Ann Lee" but not "Annette", "Jose" can find "José", and with `	 
`
  ticked a tab or line break can be found or put in by Replace. Hidden
  columns are skipped unless "Include hidden" is ticked. Alt+C, Alt+W, Alt+R
  and Alt+L switch match case, whole word, regular expression and selection
  while the find bar has focus.
- **Match count and matches lit up**: as you type, the find bar says how many
  cells match ("3 of 1,204" once you are on one) and every match on screen is
  lit up in the grid.
- **Find all list** (Alt+Enter) of up to 10,000 matches with arrow keys and
  Enter, matches per column ("Last name: 812") to show one column at a time,
  and order by row or by column. Line breaks and tabs show as ⏎ and →.
- **Replace all preview of every change**, a page at a time, with what is
  taken out and put in marked inside each cell (Ctrl+Alt+Enter); a regular
  expression replacement that names a group the pattern lacks (`$2`, or
  `$1a` for `${1}a`) is caught before anything changes.
- **Right-click a cell to find its value**: Find this value, Replace this
  value, or Find all like this in the column.
- **Find blanks, special characters, column titles and notes**: pick "Blank
  cells" to find empty cells; Special… puts in characters that are hard to
  type or see (a non-breaking space, a zero-width space, curly quotes, any
  digit or letter in a regular expression); "Column titles" searches the
  first row too, and replacing there renames the column so views and checks
  follow; "In row notes" searches the notes on rows. In a regular
  expression's replacement, `\U$1` and `\L$1` change case until `\E`, and
  `\u$1` makes just the first letter a capital.
- **Search all open tabs or chosen columns**: Search in "All open tabs" finds
  in every open file at once (each match says its tab) and Replace all shows
  every tab's part before changing anything, one undo step per tab.
  "Columns…" picks columns by name, with Select all and Unselect all. While
  the find bar is open the status bar shows "Find: 1,204".
- **Act on the rows Find all found**: show only those rows, copy them to a
  new tab, flag them with a coloured note flag, or delete them (it asks
  first, and Ctrl+Z puts them back).
- **Replace from a list** (Find › Replace from a list…, or "From a list…" in
  the find bar): paste two columns (what to find, what to put instead) or
  take them from two columns of another open tab, up to 100,000 pairs.
  Whole cells only (fast, for codes like TN → Tennessee) or text inside
  cells, where the longest match wins. Every pair applies at once, so "A to
  B" and "B to C" never chain, and you see what changes before anything
  does.
- **Untick changes and show changed rows**: in the Replace all preview, untick
  any change to leave that cell as it is. After Replace all, "Show changed
  rows" shows only the rows that changed.
- **Find numbers, dates and similar spellings by value**: "Numbers by value"
  finds `5` as 5, 5.00 or $5 (not 15), and takes ranges and comparisons
  (`10..20`, `>5`, `<=100`); "Dates by value" finds `1/5/2024` as 2024-01-05
  or Jan 5 2024, with ranges such as `1/1/2024..3/31/2024` (tick Day first
  for day-first dates); "Similar
  spelling" finds whole cells a letter or two apart ("Jon Smyth" finds "John
  Smith"), case, accents and punctuation aside (one or two letters must
  match exactly; blank cells never match). Replace puts the replacement
  in the whole cell.
- **Preserve case, hidden rows and search by columns**: with Preserve case,
  replacing "smith" with "jones" turns SMITH into JONES and Smith into Jones.
  "Include hidden" also counts, lists and replaces in rows a filter or you
  hid (Find next stays on shown rows), and the Replace all preview says how
  many hidden cells change. "By columns" makes Find next go down each column
  before moving right.
- **Copy the list and saved searches**: Copy list puts Find all's matches
  (row, column, cell) on the clipboard as tab-separated text (after the
  privacy check). Saved ▾ keeps a search, its replacement and its options by
  name on this computer, until you delete it.
- **Find tips and recent searches**: the ? button lists wildcard and pattern
  examples and the keys; Up and Down in the boxes bring back earlier searches,
  kept only until the app closes (Clear search history forgets them).

![The find bar: "1 of 142,857" next to the search, every match on screen lit up in yellow](docs/screenshots/find-count.png)
*Find counts as you type and lights up every match on screen.*

![Find all: matches per column as buttons and a list of matches to move through with the arrow keys](docs/screenshots/find-all-list.png)
*Find all: matches per column, and a list to move through with the arrow keys.*

![Replace all preview: every change a page at a time, with the added text marked in green](docs/screenshots/find-replace-preview.png)
*Replace all shows every change first, with what it adds and removes marked in each cell.*

![Find all with buttons for the rows it found: show only these, copy to a new tab, flag, delete](docs/screenshots/find-row-actions.png)
*Find all can show only the rows with a match, copy them to a new tab, flag them or delete them.*

![Replace from a list: pasted pairs, whole cells only, and a preview of the two cells that change](docs/screenshots/replace-from-list.png)
*Replace from a list: many pairs at once, shown before anything changes.*

![Numbers by value: 10..21 finds 15 and 20.5 in the amount column](docs/screenshots/find-numbers.png)
*Numbers by value: a range finds every amount in it, however it is written.*

![Similar spelling: Jon Smyth finds John Smith and Jon Smyth](docs/screenshots/find-similar.png)
*Similar spelling finds names a letter or two apart.*
- **Search a whole folder** of table files (Ctrl+Shift+F), including Excel and
  JSON files and subfolders; click a hit to open that file at that cell.
- Sort by several columns, numbers and dates in the right order, natural text
  order, by length, by last word, or in your own custom order. Flip or shuffle
  rows. Sort the columns themselves by their titles.
- Quick filter, a filter builder for more complex rules, and largest or
  smallest N (rows or percent).
- **Saved views:** filters, column layout, highlight rules and checks saved
  under a name and applied to other files (columns are matched by title; one
  not found is offered from the same place, and the view asks first).
- Find duplicate rows, and remove them keeping the first, last, newest,
  oldest or most complete copy, filling blanks from the other copies.
- **Column lists start empty:** wherever you tick columns (duplicates, merge
  rows, subtotals, joins, group figures, charts and the rest), nothing is
  ticked until you choose, and Select all / Unselect all tick or clear them
  all at once.

![Remove duplicates with Select all and Unselect all above the column list](docs/screenshots/select-all-columns.png)
*Select all ticks every column; Unselect all clears them.*


### Seeing what is in the data

![A bar chart counting customers on each plan, in the side panel](docs/screenshots/chart.png)
*Count of values on the Plan column, drawn in the side panel.*

- **Summary of each column:** type, count, blanks, distinct values, smallest,
  largest, average, total and the most common values.
- **Quick totals** of the selection (sum, average, count, numbers, smallest,
  largest, different values, characters), with one click to copy a total.
- Charts (line, bar, pie, counts), subtotals and groups, highlight rules.
- **Compare two open files**, by a key column or by whole rows: what was
  added, removed and changed.
- **Compare two columns** (Data > Compare two columns, or right-click a
  heading): for each row, are the two cells the same, different, or filled in
  only one? Light up the rows that differ, show only them, or add a column
  saying Same, Different, Only in first or Only in second. Case, extra spaces
  and numbers of equal value (5, 5.00, $5) count as the same unless you say
  otherwise. "Did the mailing address change from the residence address?"
  in one step.

![Compare two columns: the mailing and residence addresses with a result column](docs/screenshots/compare-columns.png)
*Comparing mail_address with res_address: the added column says which rows differ.*
- Random sample into a new tab.
- **Value editor:** every value in a column with its count; rename or merge
  values, group look-alikes ("Main St" / "Main Street"), or apply a
  translation list.

### Checking data

- **Data checks** per column or row: required, number (with limits, whole
  numbers), date (with limits), length, from a list, pattern, no repeats, or
  your own expression. Cells that fail are marked; show only the rows with
  problems. Checks are saved with views and can run from the command line.
- **Check fields with patterns:** one simple screen with a pattern box next to
  each column and ready-made patterns (e-mail, ZIP, phone, web address,
  letters, digits). Then show the rows that fail or pass, put failing rows
  first, or add a column saying `OK` or what failed.
- **Clean-up suggestions:** spots extra spaces, mixed case, stray characters
  and similar problems and offers a one-click fix.

### Import and export

- **Open:** Excel (`.xlsx`) and LibreOffice (`.ods`) sheets, JSON, XML, HTML
  tables, Markdown tables, fixed-width files (with a ruler you can drag), and
  log files (by a line pattern with presets; stack traces join the line
  above). From a file or from the clipboard, with a preview.
- **SQLite:** open a table or view from a SQLite database (read-only), or
  save the table into a new or existing database.
- **JSON Lines** (`.jsonl`, `.ndjson`): open them (one record per line, read
  as it streams) and export to them, the format web tools and databases use
  for logs and records.
- **Export:** Excel, JSON, JSON Lines, XML, HTML, Markdown, fixed width, PDF, SQL
  statements (standard SQL, MySQL, or SQL Server/Access style) or a template
  of your own; to a file or the clipboard, with a preview.
- **Split one file into many** (by a column's value or a row count) and
  **combine many files into one**.
- **Append** the rows of another file.
- **Encrypted copies** (`.ndenc`): a password-protected copy that opens only
  in this app, straight into memory, never to disk.

### Joining two lists

<p align="center"><img src="docs/screenshots/join.png" width="520" alt="The Join screen bringing Customer ID, Plan and Monthly spend into a sign-up list by e-mail, with a live preview"></p>

*The Join screen: bring a customer number, plan and spend into a sign-up list by e-mail, loosely matched, with a live preview of every row.*

Bring in columns from another file or tab by a shared key, with a live
preview:

- Add new columns, or **update existing ones** (overwrite, or fill blanks only).
- Keep every row, **only matched rows, or only unmatched rows**; check rows
  against a list of keys pasted in or from another tab.
- **Repeated keys:** take the first, last, every match, all in one cell, a
  count, sum, smallest or largest.
- **Loose keys:** ignore leading zeros, punctuation and `1.0` versus `1`.
- **Nearest key:** the nearest smaller or larger key, for tiers, bands and
  date periods.
- Text for rows not found, where new columns go, and a prefix for their names.
- `LOOKUP` inside computed columns, and `natendo-cli join` for scripts.

### Matching people and addresses

![The Match people screen listing scored pairs between two lists](docs/screenshots/match-people.png)
*Matching a sign-up list against the customer list: Leonard is paired with Leo, Myles with Miles, typos and capitals are forgiven. Tick the pairs to keep.*

- **Match people** across two lists, or find **near-duplicates** in one list,
  even when names are written differently: nicknames (Bill / William),
  initials, word order, titles, accents and typing slips; dates in any layout
  (with day and month swapped counting partly); ZIPs, phones and IDs by their
  digits. Jr, Sr, II and III are kept apart. Each pair gets a score; review
  them best first and accept each one, or every pair above a score.
- **Street range lookup:** find the precinct, district or ward for each
  address from a street index of house number ranges, with odd and even sides
  and ZIP or city to tell same-named streets apart. `North Main Street`,
  `N. Main St., Apt 4` and `N MAIN ST` all count as the same street. A note
  column explains any address it could not place.
- **Take changes from a newer file:** pairs rows by key columns and lists
  every changed cell, new row and missing row; tick which to take, and they
  go in as one undo step.

### Reshaping tables

- Split and merge columns and rows, unpivot, column to a table, columns to
  one column. (Pivot tables have their own section below.)
- **Smarter split:** at the last separator, at any of several, at fixed
  positions, or between letters and digits.
- **Extract** a number, e-mail, ZIP, phone number, text in brackets, words or
  a unit into a new column.
- **Choose and order columns:** keep, delete, reorder and rename in one step.
- **Computed columns** with Excel-like expressions (values saved as text).
- **Build a column without a formula:** from a template, or from if-then rules.
- **Group figures** in a new column: count, sum, average, smallest, largest,
  first, last, rank, running count, percent of group.
- Remove blank rows and columns, delete hidden rows and columns, blank rows
  between groups, a line at each change.
- Fill blank cells (from above, below, or a text), count repeats, blank out
  repeats.

### Pivot tables

![The pivot table builder: Region and Rep in Rows, Product in Columns, a sum of Amount in dollars and a distinct count of orders, filtered to 2026](docs/screenshots/pivot-builder.png)
*Sales by region, rep and product for 2026, built by dragging fields. North is collapsed; double-clicking a number lists the orders behind it.*

- **Pivot table builder:** drag fields into Rows, Columns, Values and Filters
  (or click R, C, Σ or F next to a field) and the result updates as you go,
  like Excel. On a big file a quick preview from the first 200,000 rows shows
  first, clearly marked, then the full result.
- **Count or add up any combination:** as many row, column and value fields as
  you like. Count (cells not blank, every row, numbers only, blanks), distinct
  count (different values; like row items, "Ann" and "ann " count once
  unless case or spaces are set to matter), sum, average, smallest, largest, median, standard
  deviation, variance, product, first, last, or a list of the values. Number
  columns start as Sum and text columns as Count.
- **Group dates and numbers:** years, quarters, months, year and month, days,
  days of the week, hours, or number ranges of any size.
- **Filter and sort items:** tick the items to use, add report filters, filter
  by label (begins with, contains, between...) or by value (more than,
  between...), keep the top or bottom N, and sort by name or by any value.
  Totals leave filtered-out items out.
- **Totals and show values as:** grand totals and subtotals on or off, on the
  group's row or on a Total row below; show values as a % of the grand,
  column, row or parent total, running total, difference from the previous
  item, rank or index; decimals, 1,000 separators, currency signs.
- **Expand, collapse and drill down:** open or shut any group, or all of them;
  double-click a number to get the rows behind it in a new tab.
- **Use the result:** open it as a new tab (save or export it like any file)
  or copy it; save the layout and reuse it on next month's file (columns are
  found by their titles; a title not found, after a rename say, is offered
  from the column in the same place, and you choose), or run it with
  `natendo-cli pivot`. Data > Pivot table on a pivot tab reopens its layout.
  Sums are exact (no rounding drift) and a pivot never changes the file.

![Opening a pivot layout after a column was renamed: it asks to use the column in the same place](docs/screenshots/pivot-by-place.png)
*A layout saved when column 4 was "city" asks before using it now that it is "town".*

### Cleaning tools

![The Cleaning tools screen tidying phone numbers with a before-and-after preview](docs/screenshots/cleaning-tools.png)
*Every cleaning step shows a before-and-after preview before anything changes.*

Dozens of cleaning steps, each with a live preview on a sample, for a cell
selection, a column or the whole table:

- Replace text, dates and times, or numbers; add, pad, remove, reverse or
  random text; add or remove spaces, numbers, dates and times.
- Change case, including proper name case (McDonald, O'Brien) and titles with
  small words kept lower case; split names into parts.
- Remove accents, invisible characters and odd spaces; HTML encode and
  extract; web address encode and decode.
- Fix numbers stored as text, calculate inside cells, sort inside a cell,
  numbers written in words, unique random numbers, write display formats into
  cells.
- Dates read in a layout you give, two-digit years, Julian dates.
- **Address, ZIP, phone and e-mail tidy:** USPS-style abbreviations (or
  written out), ZIP+4 and lost leading zeros, phones in the style you choose
  with extensions kept, and e-mails lower case with stray brackets removed.
- **Fill by example** (Ctrl+E): type a few results and the app fills the rest
  of the column in the same pattern.
- **Masking:** mask all but the last few characters, turn dates into a year or
  an age band, or scramble with a secret phrase (the same value always gives
  the same scramble, so joins and counts still work).

### Repeating your work

- **Cleaners:** save a list of cleaning steps and run it on any file, in the
  app or from the command line.
- **Recipes:** the app notes the steps you take on a tab (sorts, filters,
  column changes, Replace All, cleaning, computed columns and more). Save them
  as a recipe and run it on next month's file in one go; it warns you if the
  columns are different. Also from the command line.
- **Watched folder:** while the app is open, files dropped into a folder are
  run through a recipe and written to a results folder. The files dropped in
  are never changed.

### Printing, PDF and labels

- Print with page setup and print preview (Ctrl+P); landscape, margins,
  repeating title row.
- **Save as PDF** at any size, written by the app itself.
- **Mailing labels** on six common sheet sizes, starting part-way through the
  first sheet, and **one letter per row**, from a template with
  `{Column title}` fields. Empty lines are left out and text shrinks to fit.

### Working with files and folders

- Opens files from double-click, "Open with", drag and drop, or the command
  line, each in a new tab of the running window.
- Can be the default app for table files (chosen during setup or later).
- Save all, revert, rename, show in Explorer, copy the path, close unchanged
  tabs.

## Privacy-safe defaults

![The share-safe copy screen with private columns already set to leave out or mask](docs/screenshots/share-safe-copy.png)
*The app spots e-mail, phone and street columns and starts them on safe choices for a share-safe copy.*

- **No internet.** No update checks, no usage reports, no account.
- **Agent link is off.** When switched on, it listens only on this computer,
  needs a random key, and can read but never change data.
- **Log files never hold cell contents**, only what was opened, saved or went
  wrong. Five files of at most 1 MB (Help > Open the log folder).
- **Private data warnings are on.** Tools > Find private data spots columns
  that look like Social Security numbers, birth dates, phone numbers, e-mail
  addresses, street addresses or licence numbers. Before you export, print,
  save as PDF, Save As or copy a large block that touches such a column, the
  app warns you and offers a share-safe copy.
- **Share-safe copy** (File > Save a share-safe copy): per column, keep, leave
  out, mask (`***-**-6789`), year only, age band, or scramble with a secret
  phrase. Likely private columns start on safe choices. The phrase is never
  saved.
- **Private file mode** (File > Private file mode, for a file or a whole
  folder): no recovery copies, no earlier saves, not in recent files, hidden
  from the agent link, and the clipboard is cleared when the tab closes. Tabs
  made from a private tab are private too.
- **Encrypted files** use ChaCha20-Poly1305 with a password stretched by
  PBKDF2 (600,000 rounds); they are decrypted in memory only.

## Build from source

You need, on Windows:

- Node 22 or newer
- Visual Studio 2022 Build Tools with "Desktop development with C++"
- Rust via rustup (`stable-msvc`)

```
npm ci                 # install JavaScript packages
npm run dev            # run the app with live reload
npm run build          # build the app, the command line and the installer
npm test               # screen tests
cargo test --workspace # engine, command line and app tests
pwsh scripts/ci.ps1    # every check (add -UiOnly to skip Rust)
cargo run --release -p natendo-engine --example perf   # the 10-million-row speed checks
```

The installer lands in `target/release/bundle/nsis/`.

| Folder | What is in it |
|---|---|
| `crates/engine` | The data engine: load, parse, edit, save, import, export, PDF. No screen code. |
| `crates/cli` | `natendo-cli`, the command line. |
| `src-tauri` | The desktop app shell: commands, window, file types, agent link, logs, installer. |
| `ui` | The screen (Vite, Svelte 5, TypeScript). |
| `docs/screenshots` | The screenshots in this README (made-up sample data). |
| `scripts` | Helper scripts: the CI check (`ci.ps1`), browser UI tests for the command search and pivot builder, and `build-icons.py` for the icon files. |
| `assets/icon.svg` | Source for the app icon (a grid with the active cell highlighted). `assets/icon-small/` holds hand-drawn 16, 20 and 24 px versions that stay sharp in the taskbar; if the design changes, redraw them to match (or delete them, and the build script falls back to shrinking the big image). To rebuild the icon files after changing the SVG: render it to a 1024 px PNG, run `npx tauri icon <png> -o src-tauri/icons`, delete the `android` and `ios` folders it makes, then run `python scripts/build-icons.py <png>` (needs Pillow) to add the 20 and 40 px Windows sizes and the hand-drawn small frames. Also copy the SVG to `ui/public/favicon.svg`. |

## Command line (`natendo-cli`)

Installed next to the app, for scripts and scheduled jobs. Add `--json` to get
JSON output. `validate` exits with 1 when it finds problems and 2 on failure.

```
natendo-cli info data.csv --json
natendo-cli view data.csv 0 20 --json
natendo-cli convert data.xlsx -o data.csv        # to csv, tsv, psv, json, xml, html, md, xlsx, pdf, sql, fixed (.prn)
natendo-cli convert data.csv -o data.pdf --landscape
natendo-cli compute data.csv -o out.csv --expr "[Price] * [Qty]" --title Total
natendo-cli validate data.csv --checks checks.json --json
natendo-cli join data.csv other.csv -o out.csv --key ID --columns Phone,Email
natendo-cli clean cleaner.ndclean data.csv -o out.csv
natendo-cli run monthly.recipe.json data.csv -o out.csv   # --dry-run lists the steps
natendo-cli pivot by-city.ndpivot data.csv -o summary.csv # a layout saved in the pivot builder
natendo-cli app tabs                              # needs the agent link
```

A checks file is what Data > Data checks > Save writes, for example
`{"checks": [{"column": "Email", "kind": {"type": "pattern", "pattern": {"text": "[^@]+@[^@]+", "kind": "regex"}}}]}`.
Kinds: `required`, `number`, `date`, `length`, `list`, `pattern`, `unique`,
`expression`.

## Agent link

File > Options > Agent link (off by default) lets programs on this computer
read the open tables: list tabs, read cells, open a file in a tab, move the
cursor. It cannot change data. It listens on 127.0.0.1 only; the port and a
random key are in `%APPDATA%\com.natendo.dataedit\agent.json`, and every
request must carry the key. Requests are one JSON object per line:

```
{"key": "...", "cmd": "tabs"}
{"key": "...", "cmd": "read", "tab": 1, "row": 0, "count": 100}
{"key": "...", "cmd": "open", "path": "C:\\data\\file.csv"}
{"key": "...", "cmd": "goto", "tab": 1, "row": 250, "col": 3}
```

## Known limits

- The installer is not code-signed (needs a certificate), so SmartScreen warns.
- Windows only for now.
- Microsoft Access files (`.accdb`, `.mdb`) cannot be opened directly, because
  reading them needs Microsoft's Access driver. Export SQL statements in
  SQL Server/Access style, or CSV, and import those into Access.
- Password-protected Excel files are not opened.
- Formulas, cell colours and comments are not stored in the file, because a
  text table cannot hold them. Computed columns save their values as text;
  highlight rules and checks are kept in saved views.
- PDF output does not include Chinese, Japanese or Korean characters yet.
- Dragging or cutting and pasting a block of cells moves up to 50,000 cells
  at once (Ctrl+drag copies up to 100,000); whole rows and columns have no
  such limit. Rows cannot be moved while a filter is on.
- Undo does not cover: saving (it is not a step, and changes to the data
  made before a save cannot be undone after it, as the file is rewritten;
  changes to the look still can), opening, closing and moving tabs, scrolling, zoom and the
  theme, frozen rows and columns, the "First row is header" switch (it
  changes how the file is read, not the data; Rename column's added header
  row is the exception and undoes as before), Show value types and Show my
  edits (they only show things), and Options. These match Excel and Google
  Sheets, where views, settings and saving are not undo steps either.
- The watched folder stops when the app closes, and recipes with a scramble
  step cannot run there or from the command line (the phrase is never stored).
