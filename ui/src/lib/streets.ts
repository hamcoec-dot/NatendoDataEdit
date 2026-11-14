// Street range lookup (Milestone 15): settings and a first guess at the
// street index's columns from their titles.

export interface IndexCols {
  street: number;
  low: number;
  high: number;
  side: number | null;
  area: number | null;
  bring: number[];
  header: boolean;
}

export interface StreetSpec {
  address: number;
  number: number | null;
  area: number | null;
  keep_first: boolean;
  note: boolean;
}

export interface StreetReport {
  found: number;
  no_street: number;
  no_range: number;
  overlaps: number;
  no_number: number;
  bad_ranges: number;
}

/** Guess the index's columns: street, from/low, to/high, side, ZIP or
 *  city, and bring in precinct/district/ward columns. */
export function guessIndexCols(titles: string[]): IndexCols {
  const find = (re: RegExp, not: number[] = []) => titles.findIndex((t, i) => re.test(t.toLowerCase()) && !not.includes(i));
  const street = Math.max(0, find(/street|road|name/));
  const low = find(/\b(from|low|lo|start|begin|min)/, [street]);
  const high = find(/\b(to|high|hi|end|max|thru|through)/, [street, low]);
  const side = find(/(side|odd|even|parity)/);
  const area = find(/(zip|postal|city|town)/);
  const used = [street, low, high, side, area];
  const bring = titles.map((t, i) => i).filter((i) => !used.includes(i) && /(precinct|district|ward|pct|commission|school|council|poll)/.test(titles[i]!.toLowerCase()));
  return {
    street,
    low: low < 0 ? Math.min(1, titles.length - 1) : low,
    high: high < 0 ? Math.min(2, titles.length - 1) : high,
    side: side < 0 ? null : side,
    area: area < 0 ? null : area,
    bring,
    header: true,
  };
}

/** The result in words. */
export function streetSummary(r: StreetReport): string {
  const n = (x: number, one: string, many = `${one}s`) => `${x.toLocaleString()} ${x === 1 ? one : many}`;
  const parts = [`Found ${n(r.found, "address", "addresses")}.`];
  if (r.no_street) parts.push(`${n(r.no_street, "street")} not in the index.`);
  if (r.no_range) parts.push(`${n(r.no_range, "house number")} outside the street's ranges.`);
  if (r.no_number) parts.push(`${n(r.no_number, "address", "addresses")} without a house number.`);
  if (r.overlaps) parts.push(`${n(r.overlaps, "address", "addresses")} in more than one range (the first was used).`);
  if (r.bad_ranges) parts.push(`${n(r.bad_ranges, "index row")} could not be read and ${r.bad_ranges === 1 ? "was" : "were"} skipped.`);
  return parts.join(" ");
}
