// Password-protected files (Milestone 15).

/** Fewest characters in a password (the app checks again). */
export const MIN_PASSWORD = 12;

/** What is wrong with a new password, in words ("" when fine). */
export function passwordProblem(password: string, again: string): string {
  if ([...password].length < MIN_PASSWORD) return `The password needs at least ${MIN_PASSWORD} characters.`;
  if (password !== again) return "The two passwords are not the same.";
  return "";
}
