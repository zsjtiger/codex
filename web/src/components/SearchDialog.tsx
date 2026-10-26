import { useEffect, useMemo, useRef, useState } from "react";
import type { Thread } from "../api/types";
import { ChatIcon, PlusIcon, SearchIcon } from "./icons";

interface SearchDialogProps {
  threads: Thread[];
  onSelect: (threadId: string) => void;
  onNew: () => void;
  onClose: () => void;
}

type Row = { key: string; label: string; onRun: () => void; icon: "chat" | "new" };

/**
 * Search over the chats already loaded in the page. Filtering is client-side, so
 * it covers the most recent threads the sidebar fetched.
 */
export function SearchDialog({ threads, onSelect, onNew, onClose }: SearchDialogProps) {
  const [query, setQuery] = useState("");
  const [highlight, setHighlight] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const rows: Row[] = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const matches = threads
      .filter((t) => needle === "" || t.title.toLowerCase().includes(needle))
      .map((t) => ({ key: t.id, label: t.title, icon: "chat" as const, onRun: () => onSelect(t.id) }));
    return [...matches, { key: "new", label: "New session", icon: "new" as const, onRun: onNew }];
  }, [threads, query, onSelect, onNew]);

  // Keep the highlight inside the list as the results change.
  useEffect(() => {
    setHighlight((h) => Math.min(h, rows.length - 1));
  }, [rows.length]);

  const run = (row: Row | undefined) => {
    if (!row) return;
    row.onRun();
    onClose();
  };

  const chatRows = rows.filter((r) => r.icon === "chat");
  const actionRows = rows.filter((r) => r.icon === "new");

  const renderRow = (row: Row) => {
    const index = rows.indexOf(row);
    return (
      <button
        key={row.key}
        className={`search-row ${index === highlight ? "search-row-active" : ""}`}
        onMouseEnter={() => setHighlight(index)}
        onClick={() => run(row)}
      >
        {row.icon === "chat" ? <ChatIcon width={18} height={18} /> : <PlusIcon width={18} height={18} />}
        <span className="search-row-label">{row.label}</span>
      </button>
    );
  };

  return (
    <div className="modal-backdrop" onMouseDown={onClose}>
      <div
        className="search-dialog"
        role="dialog"
        aria-modal="true"
        aria-label="Search"
        onMouseDown={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key === "Escape") onClose();
          if (event.key === "ArrowDown") {
            event.preventDefault();
            setHighlight((h) => (rows.length ? (h + 1) % rows.length : 0));
          }
          if (event.key === "ArrowUp") {
            event.preventDefault();
            setHighlight((h) => (rows.length ? (h - 1 + rows.length) % rows.length : 0));
          }
          if (event.key === "Enter") {
            event.preventDefault();
            run(rows[highlight]);
          }
        }}
      >
        <div className="search-input-row">
          <SearchIcon />
          <input
            ref={inputRef}
            className="search-input"
            placeholder="Search chats…"
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setHighlight(0);
            }}
            aria-label="Search chats"
          />
          <button className="icon-button" onClick={onClose} aria-label="Close search">
            ×
          </button>
        </div>

        <div className="search-results">
          {chatRows.length > 0 && <div className="search-heading">{query.trim() ? "Matching chats" : "Recents"}</div>}
          {chatRows.map(renderRow)}
          {chatRows.length === 0 && query.trim() && <p className="empty-note">No chats match “{query.trim()}”</p>}
          <div className="search-heading">Actions</div>
          {actionRows.map(renderRow)}
        </div>

        <footer className="search-footer">
          <span>↑↓ to move</span>
          <span>Enter to open</span>
          <span>Esc to close</span>
        </footer>
      </div>
    </div>
  );
}
