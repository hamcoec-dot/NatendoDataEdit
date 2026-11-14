// Recipes (Milestone 14): the steps done on a tab, saved to a file and run
// again on another file. The engine checks and runs them; this only shows
// them and fills in a secret phrase the file never holds.

/** One step, as the engine writes it (`op` names it). */
export type RecipeStep = { op: string; [k: string]: unknown };

export interface Recipe {
  version: number;
  name: string;
  /** Column titles of the file the steps were done on. */
  columns: string[];
  header: boolean;
  steps: RecipeStep[];
  /** Things done that a recipe cannot repeat, in words. */
  left_out: string[];
}

export interface RecipeView {
  recipe: Recipe;
  /** Each step in words. */
  described: string[];
  /** Steps that hold values from the file (filter ticks, texts, examples). */
  holds_values: boolean[];
}

export interface RecipeOpened extends RecipeView {
  path: string;
  /** How this tab's columns differ from the recipe's, in words. */
  problems: string[];
}

type CleanStep = { tool?: string; phrase?: string };

function scrambleSteps(r: Recipe): CleanStep[] {
  const out: CleanStep[] = [];
  for (const s of r.steps) {
    if (s.op !== "clean") continue;
    const spec = s.spec as { cleaner?: { steps?: CleanStep[] } } | undefined;
    for (const c of spec?.cleaner?.steps ?? []) if (c.tool === "scramble") out.push(c);
  }
  return out;
}

/** Whether the recipe scrambles (it then needs the secret phrase to run). */
export function needsPhrase(r: Recipe): boolean {
  return scrambleSteps(r).length > 0;
}

/** A copy of the recipe with the secret phrase put into its scramble steps. */
export function withPhrase(r: Recipe, phrase: string): Recipe {
  const copy = JSON.parse(JSON.stringify(r)) as Recipe;
  for (const c of scrambleSteps(copy)) c.phrase = phrase;
  return copy;
}
