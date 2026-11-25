// A document's info can come back "stale": the engine answers from its last reading while a change
// (a sort, a replace) holds the document, so the window never waits for it. A stale answer is
// never taken as the tab's state (it could roll back history, revisions and row counts that undo,
// row heights and Find rely on). This is the ONE place that rule lives: every `docInfo` call goes
// through `get`, which hands back the last fresh answer for a stale one, and asks again (at most
// 250 ms apart) until a fresh answer arrives, which is then passed to `onFresh` so the tab learns
// it. (The job's own end also brings fresh info through the operation's reply.)

import type { DocInfo } from "./backend";

/** How soon a stale answer is asked about again. */
export const RETRY_MS = 250;

export class InfoGuard {
  private last = new Map<number, DocInfo>();
  private retrying = new Set<number>();
  /** Told every fresh answer that arrives after a stale one (the tab takes it as its state). */
  onFresh: ((info: DocInfo) => void) | null = null;

  constructor(
    private fetch: (doc: number) => Promise<DocInfo>,
    private wait: (ms: number) => Promise<void> = (ms) => new Promise((r) => setTimeout(r, ms)),
  ) {}

  /** The info of a document: always a fresh answer, or the last fresh one while a change runs. */
  async get(doc: number): Promise<DocInfo> {
    const info = await this.fetch(doc);
    if (!info.stale) {
      this.last.set(doc, info);
      return info;
    }
    const prev = this.last.get(doc);
    if (prev) {
      this.retry(doc);
      return prev;
    }
    // Nothing fresh to fall back on: ask again until there is one.
    for (;;) {
      await this.wait(RETRY_MS);
      const again = await this.fetch(doc);
      if (!again.stale) {
        this.last.set(doc, again);
        return again;
      }
    }
  }

  /** A fresh answer that came another way (an operation's reply): the new last one. */
  note(info: DocInfo): void {
    if (!info.stale) this.last.set(info.id, info);
  }

  forget(doc: number): void {
    this.last.delete(doc);
  }

  private retry(doc: number): void {
    if (this.retrying.has(doc)) return;
    this.retrying.add(doc);
    void (async () => {
      try {
        for (;;) {
          await this.wait(RETRY_MS);
          const info = await this.fetch(doc);
          if (!info.stale) {
            this.last.set(doc, info);
            this.onFresh?.(info);
            return;
          }
        }
      } catch {
        // The tab closed meanwhile.
      } finally {
        this.retrying.delete(doc);
      }
    })();
  }
}
