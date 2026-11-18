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
  /** Values asked each time it runs; steps hold `{{name}}` for them. */
  questions?: Question[];
}

/** A value a recipe asks for each time it runs ("which county?"). */
export interface Question {
  name: string;
  prompt: string;
  /** The last answer, filled in at first. */
  default: string;
}

/** One row of the steps panel. */
export interface StepRow {
  /** "off": turned off; "left_out": cannot be done again. */
  kind: "step" | "off" | "left_out";
  text: string;
  /** The table is now after this step. */
  done: boolean;
  op: RecipeStep | null;
  /** Text values in the step, where a question can go: [place, value]. */
  texts: [string, string][];
}

export interface StepsView {
  steps: StepRow[];
  /** The table is after this many steps. */
  at: number;
  /** Changes by hand after the last step. */
  hand_changes: boolean;
  questions: Question[];
}

/** A change to one step in the steps panel. */
export type StepChange =
  | { kind: "remove" }
  | { kind: "toggle" }
  | { kind: "move_up" }
  | { kind: "move_down" }
  | { kind: "replace"; op: RecipeStep };

/** Whether `name` can name a question (as the engine checks it). */
export function questionNameOk(name: string): boolean {
  const n = name.trim();
  return n.length > 0 && [...n].length <= 40 && /^[\p{L}\p{N} _-]+$/u.test(n);
}

const ANSWERS = "nde.recipeAnswers";

/** The last answers given to a recipe's questions on this computer. */
export function lastAnswers(recipeName: string): Record<string, string> {
  try {
    const all = JSON.parse(localStorage.getItem(ANSWERS) ?? "{}") as Record<string, Record<string, string>>;
    return all[recipeName] ?? {};
  } catch {
    return {};
  }
}

/** Remember the answers given (for the next run). */
export function keepAnswers(recipeName: string, answers: Record<string, string>): void {
  try {
    const all = JSON.parse(localStorage.getItem(ANSWERS) ?? "{}") as Record<string, Record<string, string>>;
    all[recipeName] = answers;
    const keys = Object.keys(all);
    // A bounded list: the 50 recipes run last.
    for (const k of keys.slice(0, Math.max(0, keys.length - 50))) delete all[k];
    localStorage.setItem(ANSWERS, JSON.stringify(all));
  } catch {
    // Storage full or off: the questions start from the recipe's answers.
  }
}

/** The answers to send: [name, answer] for each question. */
export function answerPairs(questions: Question[], answers: Record<string, string>): [string, string][] {
  return questions.map((q) => [q.name, answers[q.name] ?? q.default]);
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
