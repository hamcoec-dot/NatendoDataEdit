import { mount } from "svelte";
import App from "./App.svelte";
import { backend } from "./lib/backend";
import { SECOND_WINDOW } from "./lib/windowRole";
import "./app.css";

// The second window shows tabs moved to it. Files are opened in the main
// window (a file opened here could already be open there, and a tab lives in
// one window only), then moved here.
if (SECOND_WINDOW) {
  const b = backend() as unknown as Record<string, unknown>;
  const refuse = async (): Promise<never> => {
    throw { kind: "window", message: "Open files in the main window, then move the tab here (right-click its tab, Move tab to a new window)." };
  };
  for (const name of ["openFile", "openRecent", "folderOpen", "openFolderHit", "recoveryOpen", "versionOpen", "openEncrypted", "sqliteOpen"]) b[name] = refuse;
}

const target = document.getElementById("app");
if (!target) throw new Error("missing #app element");

export default mount(App, { target });
