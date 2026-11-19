import type { SessionItem } from "./backend";

/** The files from last time worth offering back at start: not gone, and
 *  not open already (a file opened from Windows started the app). */
export function sessionToOffer(items: SessionItem[]): SessionItem[] {
  return items.filter((i) => !i.missing && !i.open);
}
