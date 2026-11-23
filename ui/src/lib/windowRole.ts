/** The page runs as the second window when it is opened with `?window=2`
 *  (the main window has no such flag). The second window shows tabs moved to
 *  it and does not start things the main window owns: the start-up offers,
 *  files from Windows, the remembered session, preferences. */
export const SECOND_WINDOW: boolean = typeof location !== "undefined" && new URLSearchParams(location.search).get("window") === "2";
