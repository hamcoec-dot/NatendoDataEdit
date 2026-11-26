// SQLite databases (Milestone 15): helpers for the dialog.

export interface DbTable {
  name: string;
  /** null: a view (never counted) or a table not counted in time. */
  rows: number | null;
  view: boolean;
  /** GeoPackage: "features", "attributes", "tiles", "system" or "other"; "" elsewhere. */
  kind?: string;
}

/** The tables offered: a GeoPackage's own bookkeeping and tiles only on request. */
export function offered(tables: DbTable[], all: boolean): DbTable[] {
  return all ? tables : tables.filter((t) => !isHidden(t));
}

export function isHidden(t: DbTable): boolean {
  return t.kind === "system" || t.kind === "tiles" || t.kind === "other";
}

/** How a table shows in the list. */
export function tableLabel(t: DbTable): string {
  const notes = [t.view ? "view" : "", t.kind ?? "", t.rows === null || t.view ? "" : `${t.rows.toLocaleString()} rows`].filter(Boolean);
  return notes.length ? `${t.name} (${notes.join(", ")})` : t.name;
}

/** A table name suggested from a tab's name: "Voters 2026.csv" -> "Voters_2026". */
export function tableName(docName: string): string {
  const stem = docName.replace(/\.[^.]*$/, "");
  const t = stem.replace(/[^A-Za-z0-9_]+/g, "_").replace(/^_+|_+$/g, "");
  if (!t) return "data";
  return /^sqlite_/i.test(t) ? `t_${t}` : t.slice(0, 128);
}
