// The CSV formula guard: cells a spreadsheet could run as formulas get an
// apostrophe in reports, safe copies and (only when ticked) split files. The
// rule itself is in the engine; here are the setting's words and the result note.

export const GUARD_LABEL = "Keep exported CSV cells from running as formulas in Excel";

/** What to add to a result message ("" when no cell was changed). */
export const guardNote = (n?: number): string =>
  n ? ` ${n.toLocaleString()} ${n === 1 ? "cell" : "cells"} that Excel could run as a formula got an apostrophe in front, so ${n === 1 ? "it shows" : "they show"} as text.` : "";
