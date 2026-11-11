// The desktop app's native features. Each call does nothing in a browser, where `isDesktop()` is false.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { save } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";

/** True inside the Tauri desktop shell. */
export function isDesktop(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** The session token, kept in the operating system's keychain. */
export async function readStoredToken(): Promise<string | null> {
  return invoke<string | null>("read_token");
}

export async function writeStoredToken(token: string | null): Promise<void> {
  if (token) await invoke("store_token", { token });
  else await invoke("clear_token");
}

/** Asks where to save a file, writes it there, and returns the path (null when cancelled). */
export async function saveFileAs(name: string, bytes: Uint8Array): Promise<string | null> {
  const path = await save({ defaultPath: name });
  if (!path) return null;
  await invoke("save_file", { path, bytes: Array.from(bytes) });
  return path;
}

/** Opens the folder that holds a file, with the file selected. */
export async function showInFolder(path: string): Promise<void> {
  await revealItemInDir(path);
}

/** Shows a system notification when permission allows it. */
export async function notify(title: string, body: string): Promise<void> {
  let granted = await isPermissionGranted();
  if (!granted) granted = (await requestPermission()) === "granted";
  if (granted) sendNotification({ title, body });
}

/** Calls `handle` with the id of each menu item the user chooses (see the menu in main.rs). */
export async function onMenuAction(handle: (id: string) => void): Promise<() => void> {
  return listen<string>("menu-action", (event) => handle(event.payload));
}

export const windowControls = {
  minimize: () => getCurrentWindow().minimize(),
  toggleMaximize: () => getCurrentWindow().toggleMaximize(),
  close: () => getCurrentWindow().close(),
};
