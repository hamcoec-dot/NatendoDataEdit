## Not yet released

- **Fixed: Changes from a newer file with hidden rows.** With rows filtered or hidden, it could offer to add a second copy of a hidden row, or to delete a row the newer file still had. Every row of both tables is now compared; changes to hidden rows are marked, and a hidden row is never deleted without being ticked. Tools that read another tab say how many of its rows are hidden.
- **CSV export safety** (on to start with): in the list of changes, the report of what changed since a checkpoint and the share-safe copy, a cell that a spreadsheet could run as a formula gets one apostrophe in front, so it shows as text. Split into files has its own tick, off to start with. Saving your own file never changes it.
- **Shifted rows:** a note on opening counts the rows with a different number of fields than most, and the Data menu shows only those, the short ones or the long ones.
- Emptying a cell past the end of a short row no longer adds empty fields to it.
- **Leading zeros from LibreOffice sheets:** a number an `.ods` sheet shows as `00123` can now be imported that way, with the same question as for Excel files. Excel formats written with quoted dashes or spaces (`00"-"000`) are recognised too.
- **Repair bar:** fix a shifted row where it broke: join fields, join it with the next or previous row of the file, or read a stray quote mark as text, with a preview and one undo step. **Fix all like this** repairs every row of the same shape at once, and the emptied extra column can be removed.
- **Find gaps in a sequence:** Data > Find gaps in a sequence lists the numbers or IDs missing from a column as ranges, with repeats and out-of-order rows, and can show the rows next to each gap. Also `natendo-cli gaps`.
- **Ambiguous dates:** Check dates in this column lights the dates that could be read two ways and says what the column tells about their order.
- **Clean matrix:** Clean > Clean matrix cleans many columns at once, each with its own ticked steps, with exact preview counts, in one pass, one undo step and one recipe step.
