import { useEffect, useRef, useState } from "react";
import { attachments as attachmentApi, search } from "../api/resources";
import type { ProjectFileHit, SearchResults, Thread } from "../api/types";
import { ChatIcon, PlusIcon, SearchIcon } from "./icons";
import { useI18n } from "../i18n";
import { tx } from "../i18n";

interface SearchDialogProps {
  workspaceId: string;
  threads: Thread[];
  onOpenThread: (threadId: string) => void;
  onOpenArtifacts: () => void;
  onOpenProjects: () => void;
  /** Opens a project's page, for a file found in it. */
  onOpenProject: (projectId: string) => void;
  onNew: () => void;
  onClose: () => void;
  onError: (message: string) => void;
}

type Row = { key: string; label: string; detail?: string; onRun: () => void };

/**
 * Search over the workspace through the gateway: chat titles, message text, artifacts and
 * projects. With no query it lists recent chats.
 */
export function SearchDialog({ workspaceId, threads, onOpenThread, onOpenArtifacts, onOpenProjects, onOpenProject, onNew, onClose, onError }: SearchDialogProps) {
  const { t } = useI18n();
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResults | null>(null);
  const [fileHits, setFileHits] = useState<ProjectFileHit[]>([]);
  const [highlight, setHighlight] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // Debounced server search. Stale responses are dropped.
  useEffect(() => {
    const q = query.trim();
    if (!q) {
      setResults(null);
      setFileHits([]);
      return;
    }
    let current = true;
    const timer = setTimeout(() => {
      search(workspaceId, q)
        .then((found) => current && setResults(found))
        .catch((err) => current && onError(err instanceof Error ? err.message : tx("Search failed.")));
      attachmentApi
        .searchProjectFiles(workspaceId, q)
        .then((hits) => current && setFileHits(hits))
        .catch(() => current && setFileHits([]));
    }, 200);
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [query, workspaceId, onError]);

  const close = () => onClose();
  const sections: { heading: string; rows: Row[] }[] = [];
  if (!query.trim()) {
    sections.push({
      heading: "Recents",
      rows: threads.map((t) => ({ key: `t-${t.id}`, label: t.title, onRun: () => onOpenThread(t.id) })),
    });
  } else if (results) {
    sections.push(
      { heading: "Chats", rows: results.threads.map((t) => ({ key: `t-${t.id}`, label: t.title, onRun: () => onOpenThread(t.id) })) },
      {
        heading: "Messages",
        rows: results.turns.map((t) => ({ key: `m-${t.id}`, label: t.thread_title, detail: t.input.slice(0, 120), onRun: () => onOpenThread(t.thread_id) })),
      },
      { heading: "Artifacts", rows: results.artifacts.map((a) => ({ key: `a-${a.id}`, label: a.title, detail: a.kind, onRun: onOpenArtifacts })) },
      { heading: "Projects", rows: results.projects.map((p) => ({ key: `p-${p.id}`, label: p.name, onRun: onOpenProjects })) },
      {
        heading: "Project files",
        rows: fileHits.map((f) => ({ key: `f-${f.id}`, label: f.filename, detail: f.snippet ?? f.project_name, onRun: () => onOpenProject(f.project_id) })),
      },
    );
  }
  sections.push({ heading: "Actions", rows: [{ key: "new", label: "New session", onRun: onNew }] });
  const rows = sections.flatMap((s) => s.rows);
  const shown = Math.min(highlight, Math.max(rows.length - 1, 0));

  const run = (row: Row | undefined) => {
    if (!row) return;
    row.onRun();
    close();
  };

  return (
    <div className="modal-backdrop" onMouseDown={close}>
      <div
        className="search-dialog"
        role="dialog"
        aria-modal="true"
        aria-label={tx("Search")}
        onMouseDown={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key === "Escape") close();
          if (event.key === "ArrowDown") {
            event.preventDefault();
            setHighlight(rows.length ? (shown + 1) % rows.length : 0);
          }
          if (event.key === "ArrowUp") {
            event.preventDefault();
            setHighlight(rows.length ? (shown - 1 + rows.length) % rows.length : 0);
          }
          if (event.key === "Enter") {
            event.preventDefault();
            run(rows[shown]);
          }
        }}
      >
        <div className="search-input-row">
          <SearchIcon />
          <input
            ref={inputRef}
            className="search-input"
            placeholder={t("search.placeholder")}
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setHighlight(0);
            }}
            aria-label={tx("Search")}
          />
          <button className="icon-button" onClick={close} aria-label={tx("Close search")}>
            ×
          </button>
        </div>

        <div className="search-results">
          {sections.map((section) =>
            section.rows.length === 0 ? null : (
              <div key={section.heading}>
                <div className="search-heading">{tx(section.heading)}</div>
                {section.rows.map((row) => {
                  const index = rows.indexOf(row);
                  return (
                    <button
                      key={row.key}
                      className={`search-row ${index === shown ? "search-row-active" : ""}`}
                      onMouseEnter={() => setHighlight(index)}
                      onClick={() => run(row)}
                    >
                      {section.heading === "Actions" ? <PlusIcon width={18} height={18} /> : <ChatIcon width={18} height={18} />}
                      <span className="search-row-label">{tx(row.label)}</span>
                      {row.detail && <span className="search-row-detail">{row.detail}</span>}
                    </button>
                  );
                })}
              </div>
            ),
          )}
          {query.trim() && results && rows.length === 1 && <p className="empty-note">{tx("No matches for")} “{query.trim()}”</p>}
        </div>

        <footer className="search-footer">
          <span>{tx("↑↓ to move")}</span>
          <span>{tx("Enter to open")}</span>
          <span>{tx("Esc to close")}</span>
        </footer>
      </div>
    </div>
  );
}
