// The Record panel's field list: which columns show, in what order. Pure, so it is tested without the screen.

export interface RecordListInput {
  /** Columns read (the first n of the file). */
  n: number;
  titles: string[];
  /** The row's values, one per column read. */
  values: string[];
  hidden: (col: number) => boolean;
  showHidden: boolean;
  query: string;
  /** Titles pinned by the person, in the order pinned. */
  pins: string[];
}

export interface RecordList {
  /** Columns to show: pinned first (in pin order), then the rest in column order. */
  order: number[];
  /** The columns that are pinned. */
  pinned: Set<number>;
}

const shownTitle = (titles: string[], c: number) => titles[c] || `Column ${c + 1}`;

/** The column a pinned title stands for: the first column with that title. */
export function pinColumn(titles: string[], n: number, title: string): number | null {
  for (let c = 0; c < n; c++) if (shownTitle(titles, c) === title) return c;
  return null;
}

export function recordList(i: RecordListInput): RecordList {
  const q = i.query.trim().toLowerCase();
  const pinned = new Set<number>();
  const pinOrder: number[] = [];
  for (const p of i.pins) {
    const c = pinColumn(i.titles, i.n, p);
    if (c !== null && !pinned.has(c)) {
      pinned.add(c);
      pinOrder.push(c);
    }
  }
  const keep = (c: number): boolean => {
    if (!i.showHidden && i.hidden(c)) return false;
    if (q === "") return true;
    return shownTitle(i.titles, c).toLowerCase().includes(q) || (i.values[c] ?? "").toLowerCase().includes(q);
  };
  const order = pinOrder.filter(keep);
  for (let c = 0; c < i.n; c++) if (!pinned.has(c) && keep(c)) order.push(c);
  return { order, pinned };
}

/** Pins after a star was clicked on `col`: a pinned field is unpinned, another is pinned last. Titles, not column numbers. */
export function togglePin(pins: string[], titles: string[], col: number): string[] {
  const t = shownTitle(titles, col);
  return pins.includes(t) ? pins.filter((p) => p !== t) : [...pins, t].slice(-200);
}
