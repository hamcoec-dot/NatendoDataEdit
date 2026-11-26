## Not yet released

- **CSV export safety** (on to start with): in the list of changes, the report of what changed since a checkpoint and the share-safe copy, a cell that a spreadsheet could run as a formula gets one apostrophe in front, so it shows as text. Split into files has its own tick, off to start with. Saving your own file never changes it.
- **Shifted rows:** a note on opening counts the rows with a different number of fields than most, and the Data menu shows only those, the short ones or the long ones.
- Emptying a cell past the end of a short row no longer adds empty fields to it.
- **Leading zeros from LibreOffice sheets:** a number an `.ods` sheet shows as `00123` can now be imported that way, with the same question as for Excel files. Excel formats written with quoted dashes or spaces (`00"-"000`) are recognised too.
- **Repair bar:** fix a shifted row where it broke: join fields, join it with the next or previous row of the file, or read a stray quote mark as text, with a preview and one undo step. **Fix all like this** repairs every row of the same shape at once, and the emptied extra column can be removed.
- **Find gaps in a sequence:** Data > Find gaps in a sequence lists the numbers or IDs missing from a column as ranges, with repeats and out-of-order rows, and can show the rows next to each gap. Also `natendo-cli gaps`.
- **Ambiguous dates:** Check dates in this column lights the dates that could be read two ways and says what the column tells about their order.
- **Clean matrix:** Clean > Clean matrix cleans many columns at once, each with its own ticked steps, with exact preview counts, in one pass, one undo step and one recipe step.
- **Compare lists:** Data > Compare lists counts the keys shared by 2 to 5 open tabs, slice by slice, and opens any slice as a tab. Also `natendo-cli sets`.
- **Combine by title no longer loses data.** When a file had two columns with the same title, the second overwrote the first. Both are now kept, as `Note` and `Note (2)`. A file that already has a `Source file` column keeps it when you add the file-name column, which is then numbered.
- **`natendo-cli cat --by-name` keeps every column.** It had the same loss, and also silently left out columns the first file did not have. The output now has every file's columns.
- **`natendo-cli --json` works on every command.** It was ignored by `headers`, `select`, `filter`, `sort`, `dedupe`, `stats`, `count`, `sample`, `slice`, `split`, `cat`, `clean`, `convert`, `compute`, `run` and `pivot`. Scripts that already passed it will now get JSON.
- **Shapefile attribute tables:** open a `.dbf` (or the `.shp` beside it), with the text encoding picked for you and shown, and a FID column to join your edits back to the shapes. The `.dbf` is never written.
- **GeoPackage:** open a `.gpkg` and pick a layer or attribute table. Shapes are left out; nothing is written beside the file. `.gpkg`, `.sqlite` and `.sqlite3` files now open from Open, drag and drop and double-click.
- A SQLite database in write-ahead mode with unsaved changes in its `-wal` file (or an unfinished `-journal`) is no longer opened, because reading it could create or change files beside it. Close it in the program using it first.
