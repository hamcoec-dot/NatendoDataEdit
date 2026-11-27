// Explode cells (C7): the settings a new explode starts from (the engine's defaults).

import type { Explode } from "./backend";

export const EXPLODE_BASE: Explode = {
  col: 0,
  keep_first: true,
  kind: "object",
  how: "columns",
  separator: "|",
  kv: "=",
  depth: 1,
  keys: null,
  add_new_keys: false,
  keep_original: true,
  trim: true,
  mark: "1",
  null_blank: true,
  original_col: false,
  match_case: false,
  skip_empty: true,
};
