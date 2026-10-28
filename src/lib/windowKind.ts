import { getCurrentWindow } from "@tauri-apps/api/window";

/**
 * Which window this webview is.
 *
 * The label is asked of Tauri first, but it is not trusted alone: the settings and
 * add-clip windows load the same `index.html`, and a label that comes back empty
 * (Tauri's internals are installed by the host, and a webview that boots before
 * they land — or one opened while another is being built — can see nothing) makes
 * the window render the history UI. That is the "settings window turned into
 * another main window" bug: the window was fine, the guess about which window it
 * was had failed.
 *
 * Reading it from the URL instead cannot race, because the backend puts it there
 * when the window is created and the URL is in hand before any script runs.
 */
export type WindowKind = "main" | "settings" | "add-clip" | "sticky";

export function windowKind(): WindowKind {
  const fromUrl = new URLSearchParams(window.location.search).get("window");
  if (fromUrl === "settings" || fromUrl === "add-clip" || fromUrl === "sticky") return fromUrl;

  try {
    const label = getCurrentWindow().label;
    if (label === "settings" || label === "add-clip") return label;
    if (label.startsWith("sticky-")) return "sticky";
  } catch {
    // No Tauri internals: treat it as the main window, which is what a browser
    // preview of the dev server should look like.
  }
  return "main";
}
