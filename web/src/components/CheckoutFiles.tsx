import { useEffect, useRef, useState } from "react";
import { threads as threadApi } from "../api/resources";
import type { CheckoutEntry } from "../api/types";
import { tx } from "../i18n";
import { subscribeThread } from "../state/threadEvents";
import { usePreview } from "./Preview";
import { ChevronIcon, FileIcon } from "./icons";

/** A collapsed folder list is a relative path; the checkout's root is "". */
type Listing = Record<string, CheckoutEntry[] | "loading">;

function joinPath(folder: string, name: string): string {
  return folder ? `${folder}/${name}` : name;
}

/**
 * The files of a code chat's checkout, as a tree. Folders load when opened; a file opens in the side panel,
 * with its text as it is in the checkout.
 */
export function CheckoutFiles({ threadId, onError }: { threadId: string; onError: (message: string) => void }) {
  const preview = usePreview();
  const [open, setOpen] = useState(false);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [listing, setListing] = useState<Listing>({});

  const load = async (folder: string) => {
    setListing((prev) => ({ ...prev, [folder]: "loading" }));
    try {
      const entries = await threadApi.listFiles(threadId, folder);
      setListing((prev) => ({ ...prev, [folder]: entries }));
    } catch (err) {
      setListing((prev) => {
        const { [folder]: _failed, ...rest } = prev;
        return rest;
      });
      onError(err instanceof Error ? err.message : tx("The folder could not be opened."));
    }
  };

  const toggleOpen = () => {
    if (!open && listing[""] === undefined) void load("");
    setOpen(!open);
  };

  const toggleFolder = (folder: string) => {
    const next = new Set(expanded);
    if (next.has(folder)) {
      next.delete(folder);
    } else {
      next.add(folder);
      if (listing[folder] === undefined) void load(folder);
    }
    setExpanded(next);
  };

  // Reloads the root and every folder that is open, after a change.
  const refresh = async () => {
    const folders = ["", ...expanded];
    await Promise.all(folders.map((folder) => load(folder)));
  };

  const openFile = async (path: string) => {
    try {
      const file = await threadApi.readFile(threadId, path);
      const name = path.split("/").pop() ?? path;
      // Only a whole text file is edited: saving a cut or binary file would lose what was not shown.
      if (!file.binary && !file.truncated && file.content !== null) {
        preview.open({
          kind: "checkout",
          title: path,
          text: file.content,
          onSave: async (text) => {
            await threadApi.writeFile(threadId, path, text);
            await refresh();
          },
        });
        return;
      }
      const lang = name.includes(".") ? (name.split(".").pop() ?? "") : "";
      let text = file.binary ? tx("This file is binary, so it is not shown.") : file.content ?? "";
      if (file.truncated) {
        text += `\n\n… ${tx("The file is cut here. It is")} ${file.bytes.toLocaleString()} ${tx("bytes.")}`;
      }
      preview.open({ kind: "code", title: path, lang, text });
    } catch (err) {
      onError(err instanceof Error ? err.message : tx("The file could not be opened."));
    }
  };

  // Each change runs in the checkout, then the tree is read again. Errors are shown above the chat.
  const change = async (action: () => Promise<void>, failed: string) => {
    try {
      await action();
      await refresh();
    } catch (err) {
      onError(err instanceof Error ? err.message : failed);
    }
  };

  const newFile = () => {
    const path = window.prompt(tx("Path of the new file, from the checkout's root, such as src/new.rs"));
    if (!path?.trim()) return;
    void change(() => threadApi.writeFile(threadId, path.trim(), ""), tx("The file could not be created."));
  };

  const newFolder = () => {
    const path = window.prompt(tx("Path of the new folder, from the checkout's root, such as docs/notes"));
    if (!path?.trim()) return;
    void change(() => threadApi.createFolder(threadId, path.trim()), tx("The folder could not be created."));
  };

  const rename = (path: string) => {
    const to = window.prompt(tx("New path, from the checkout's root"), path);
    if (!to?.trim() || to.trim() === path) return;
    void change(() => threadApi.moveFile(threadId, path, to.trim()), tx("It could not be renamed."));
  };

  const remove = (path: string) => {
    if (!window.confirm(tx("Delete this from the checkout? A folder is deleted with everything in it."))) return;
    void change(() => threadApi.deleteFile(threadId, path), tx("It could not be deleted."));
  };

  // The tree follows the checkout: while it is open, changes to its files reload it. Changes come in bursts
  // (a turn writes many files), so the reload waits until they stop.
  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;
  useEffect(() => {
    if (!open) return;
    let stopped = false;
    let watchId: string | null = null;
    let timer: number | undefined;
    let unsubscribe: (() => void) | null = null;
    void threadApi
      .watchFiles(threadId)
      .then((watch) => {
        if (stopped) {
          void threadApi.unwatchFiles(threadId, watch.watch_id).catch(() => undefined);
          return;
        }
        watchId = watch.watch_id;
        unsubscribe = subscribeThread(threadId, (method, params) => {
          if (method !== "fs/changed" || params.watchId !== watchId) return;
          window.clearTimeout(timer);
          timer = window.setTimeout(() => void refreshRef.current(), 250);
        });
      })
      // A checkout that cannot be watched still reads on demand; the tree just does not follow it.
      .catch(() => undefined);
    return () => {
      stopped = true;
      window.clearTimeout(timer);
      unsubscribe?.();
      if (watchId) void threadApi.unwatchFiles(threadId, watchId).catch(() => undefined);
    };
  }, [open, threadId]);

  const renderFolder = (folder: string, depth: number) => {
    const entries = listing[folder];
    if (entries === undefined || entries === "loading") {
      return <div className="checkout-row muted small" style={{ paddingLeft: 12 + depth * 14 }}>{tx("Loading…")}</div>;
    }
    if (entries.length === 0) {
      return <div className="checkout-row muted small" style={{ paddingLeft: 12 + depth * 14 }}>{tx("Empty folder")}</div>;
    }
    return entries.map((entry) => {
      const path = joinPath(folder, entry.name);
      const isOpen = expanded.has(path);
      return (
        <div key={path}>
          <div className="checkout-line">
          <button
            type="button"
            className="checkout-row"
            style={{ paddingLeft: 12 + depth * 14 }}
            aria-expanded={entry.is_directory ? isOpen : undefined}
            onClick={() => (entry.is_directory ? toggleFolder(path) : void openFile(path))}
          >
            {entry.is_directory ? (
              <ChevronIcon className={isOpen ? "thinking-chevron thinking-chevron-open" : "thinking-chevron"} />
            ) : (
              <FileIcon />
            )}
            <span>{entry.name}</span>
          </button>
          <span className="checkout-row-actions">
            <button type="button" className="link-button" onClick={() => rename(path)}>
              {tx("Rename")}
            </button>
            <button type="button" className="link-button" onClick={() => remove(path)}>
              {tx("Delete")}
            </button>
          </span>
          </div>
          {entry.is_directory && isOpen && renderFolder(path, depth + 1)}
        </div>
      );
    });
  };

  return (
    <div className="checkout-files">
      <button type="button" className="checkout-toggle" aria-expanded={open} onClick={toggleOpen}>
        <span>{tx("Files")}</span>
        <ChevronIcon className={open ? "thinking-chevron thinking-chevron-open" : "thinking-chevron"} />
      </button>
      {open && (
        <>
          <div className="checkout-toolbar">
            <button type="button" className="link-button" onClick={newFile}>
              {tx("New file")}
            </button>
            <button type="button" className="link-button" onClick={newFolder}>
              {tx("New folder")}
            </button>
          </div>
          <div className="checkout-tree">{renderFolder("", 0)}</div>
        </>
      )}
    </div>
  );
}
