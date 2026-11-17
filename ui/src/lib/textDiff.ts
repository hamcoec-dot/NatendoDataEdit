/** The part of `before` that `after` replaced, as three pieces each: the
 *  same start, the changed middle and the same end (by characters), for
 *  marking a change inside a cell. */
export function cellDiff(before: string, after: string): { head: string; del: string; ins: string; tail: string } {
  const a = [...before];
  const b = [...after];
  let i = 0;
  while (i < a.length && i < b.length && a[i] === b[i]) i++;
  let j = 0;
  while (j < a.length - i && j < b.length - i && a[a.length - 1 - j] === b[b.length - 1 - j]) j++;
  return {
    head: a.slice(0, i).join(""),
    del: a.slice(i, a.length - j).join(""),
    ins: b.slice(i, b.length - j).join(""),
    tail: a.slice(a.length - j).join(""),
  };
}
