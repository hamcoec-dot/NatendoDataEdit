// Recent searches of the find bar. Searches often hold people's names or
// addresses, so they live in memory only, for this session, and are never
// written to prefs, logs or recovery files.

const MAX = 20;

export const findHistory = $state<{ finds: string[]; replaces: string[] }>({ finds: [], replaces: [] });

/** Put `text` first in a list (no repeats, at most 20). */
function remember(list: string[], text: string): void {
  if (!text || text.length > 1000) return;
  const i = list.indexOf(text);
  if (i >= 0) list.splice(i, 1);
  list.unshift(text);
  if (list.length > MAX) list.length = MAX;
}

export function rememberFind(text: string): void {
  remember(findHistory.finds, text);
}

export function rememberReplace(text: string): void {
  remember(findHistory.replaces, text);
}

export function clearFindHistory(): void {
  findHistory.finds.length = 0;
  findHistory.replaces.length = 0;
}

/** The entry Up (older) or Down (newer) moves to from `current`. */
export function stepHistory(list: string[], current: string, older: boolean): string | null {
  if (!list.length) return null;
  const i = list.indexOf(current);
  if (i < 0) return older ? list[0]! : null;
  const j = older ? i + 1 : i - 1;
  if (j < 0) return "";
  return j < list.length ? list[j]! : null;
}
