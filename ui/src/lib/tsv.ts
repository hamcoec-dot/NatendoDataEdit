// Clipboard text <-> cells. Copy produces tab-separated text (values with a
// tab, quote or line break quoted); paste reads the same, so a copy from
// this app or from a spreadsheet pastes back cell for cell.

export function parseTsv(text: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let cell = "";
  let i = 0;
  const n = text.length;
  let atCellStart = true;
  while (i < n) {
    const ch = text[i];
    if (atCellStart && ch === '"') {
      // Quoted cell: runs to the closing quote; "" is a quote.
      let j = i + 1;
      let value = "";
      let closed = false;
      while (j < n) {
        const cj = text[j];
        if (cj === '"') {
          if (text[j + 1] === '"') {
            value += '"';
            j += 2;
            continue;
          }
          closed = true;
          j++;
          break;
        }
        value += cj;
        j++;
      }
      const next = text[j];
      if (closed && (j >= n || next === "\t" || next === "\n" || next === "\r")) {
        cell = value;
        i = j;
        atCellStart = false;
        continue;
      }
      // Not a well-formed quoted cell: take the quote literally.
    }
    atCellStart = false;
    if (ch === "\t") {
      row.push(cell);
      cell = "";
      atCellStart = true;
      i++;
    } else if (ch === "\r" || ch === "\n") {
      row.push(cell);
      rows.push(row);
      row = [];
      cell = "";
      atCellStart = true;
      i += ch === "\r" && text[i + 1] === "\n" ? 2 : 1;
    } else {
      cell += ch;
      i++;
    }
  }
  if (!atCellStart || row.length > 0 || cell !== "") {
    row.push(cell);
    rows.push(row);
  }
  return rows;
}

export function toTsv(rows: string[][]): string {
  return rows
    .map((r) =>
      r.map((v) => (/[\t"\r\n]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v)).join("\t"),
    )
    .join("\r\n")
    .concat(rows.length ? "\r\n" : "");
}
