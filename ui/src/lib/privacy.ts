// Private data (Milestone 13): words for what the finder found, and when
// to warn before data leaves the app.
import type { Finding, PrivateKind } from "./backend";

export const KIND_WORDS: Record<PrivateKind, string> = {
  ssn: "Social Security numbers",
  birth_date: "birth dates",
  phone: "phone numbers",
  email: "e-mail addresses",
  address: "street addresses",
  licence: "licence or ID numbers",
};

/** "SSN (Social Security numbers), Born (birth dates)" (at most four named). */
export function findingsText(f: Finding[]): string {
  const named = f.slice(0, 4).map((x) => `${x.title} (${KIND_WORDS[x.kind]})`);
  return f.length > 4 ? `${named.join(", ")} and ${f.length - 4} more` : named.join(", ");
}

/** Copies this large (cells) that touch a private column are warned about. */
export const LARGE_COPY = 1_000;

/** Whether a copy of columns col..col+cols-1 touches a private column. */
export function copyTouches(f: Finding[], col: number, cols: number): boolean {
  return f.some((x) => x.col >= col && x.col < col + cols);
}
