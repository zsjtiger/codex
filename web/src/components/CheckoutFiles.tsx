import { useState } from "react";
import { threads as threadApi } from "../api/resources";
import type { CheckoutEntry } from "../api/types";
import { tx } from "../i18n";
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

  const openFile = async (path: string) => {
    try {
      const file = await threadApi.readFile(threadId, path);
      const name = path.split("/").pop() ?? path;
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
      {open && <div className="checkout-tree">{renderFolder("", 0)}</div>}
    </div>
  );
}
