// Watched folder (Milestone 15).

export interface WatchStatus {
  running: boolean;
  folder: string | null;
  recipe: string | null;
  out: string | null;
  done: number;
  failed: number;
  /** Newest first. */
  log: string[];
}
