// SQLite databases (Milestone 15): helpers for the dialog.

export interface DbTable {
  name: string;
  /** null: a view (never counted) or a table not counted in time. */
  rows: number | null;
  view: boolean;
}

/** How a table shows in the list. */
export function tableLabel(t: DbTable): string {
  if (t.view) return `${t.name} (view)`;
  return t.rows === null ? t.name : `${t.name} (${t.rows.toLocaleString()} rows)`;
}

/** A table name suggested from a tab's name: "Voters 2026.csv" -> "Voters_2026". */
export function tableName(docName: string): string {
  const stem = docName.replace(/\.[^.]*$/, "");
  const t = stem.replace(/[^A-Za-z0-9_]+/g, "_").replace(/^_+|_+$/g, "");
  if (!t) return "data";
  return /^sqlite_/i.test(t) ? `t_${t}` : t.slice(0, 128);
}
