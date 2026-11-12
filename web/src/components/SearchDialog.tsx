import { useEffect, useMemo, useRef, useState } from "react";
import { attachments as attachmentApi, scheduled as scheduledApi, search } from "../api/resources";
import type { ProjectFileHit, ScheduledTask, SearchResults, Thread } from "../api/types";
import { ArtifactIcon, ChatIcon, ClockIcon, CodeIcon, PlusIcon, ProjectIcon, SearchIcon } from "./icons";
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
  /** Opens a scheduled task on its page. */
  onOpenScheduled: (taskId: string) => void;
  onNew: () => void;
  onClose: () => void;
  onError: (message: string) => void;
}

/** The types a search can be limited to. "all" searches everything. */
export const SEARCH_KINDS = [
  { id: "all", label: "All" },
  { id: "artifacts", label: "Artifacts" },
  { id: "sessions", label: "Sessions" },
  { id: "code", label: "Code" },
  { id: "projects", label: "Projects" },
  { id: "scheduled", label: "Scheduled" },
] as const;
type Kind = (typeof SEARCH_KINDS)[number]["id"];

type Icon = "chat" | "code" | "artifact" | "project" | "clock" | "plus";
type Row = { key: string; label: string; detail?: string; icon: Icon; attention?: boolean; onRun: () => void };
type Section = { heading: string; rows: Row[] };

const ICONS: Record<Icon, typeof ChatIcon> = {
  chat: ChatIcon,
  code: CodeIcon,
  artifact: ArtifactIcon,
  project: ProjectIcon,
  clock: ClockIcon,
  plus: PlusIcon,
};

/**
 * Search over the workspace: chats, code sessions, messages, artifacts, projects, project files and
 * scheduled tasks. A type filter limits the results. With no query it lists sessions by state.
 */
export function SearchDialog({
  workspaceId,
  threads,
  onOpenThread,
  onOpenArtifacts,
  onOpenProjects,
  onOpenProject,
  onOpenScheduled,
  onNew,
  onClose,
  onError,
}: SearchDialogProps) {
  const { t } = useI18n();
  const [query, setQuery] = useState("");
  const [kind, setKind] = useState<Kind>("all");
  const [results, setResults] = useState<SearchResults | null>(null);
  const [fileHits, setFileHits] = useState<ProjectFileHit[]>([]);
  const [tasks, setTasks] = useState<ScheduledTask[]>([]);
  const [highlight, setHighlight] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  useEffect(() => {
    scheduledApi
      .list(workspaceId)
      .then(setTasks)
      .catch(() => setTasks([]));
  }, [workspaceId]);

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

  // Which threads are code sessions, by id, from the workspace's threads.
  const isCode = useMemo(() => {
    const codeIds = new Set(threads.filter((thread) => thread.repo_id).map((thread) => thread.id));
    return (threadId: string) => codeIds.has(threadId);
  }, [threads]);

  const wantsChats = kind === "all" || kind === "sessions" || kind === "code";
  const keepThread = (threadId: string) => {
    if (kind === "sessions") return !isCode(threadId);
    if (kind === "code") return isCode(threadId);
    return true;
  };

  const sections = useMemo<Section[]>(() => {
    const out: Section[] = [];
    const q = query.trim().toLowerCase();
    const threadRow = (thread: { id: string; title: string }): Row => ({
      key: `t-${thread.id}`,
      label: thread.title,
      icon: isCode(thread.id) ? "code" : "chat",
      onRun: () => onOpenThread(thread.id),
    });

    if (!q) {
      // No query: sessions that need attention (running now), then recent ones.
      if (wantsChats) {
        const visible = threads.filter((thread) => !thread.archived_at && keepThread(thread.id));
        const attention = visible.filter((thread) => thread.state === "running");
        const recent = visible.filter((thread) => thread.state !== "running");
        if (attention.length > 0) {
          out.push({ heading: "Needs attention", rows: attention.map((thread) => ({ ...threadRow(thread), attention: true })) });
        }
        out.push({ heading: "Recents", rows: recent.map(threadRow) });
      }
      if (kind === "all" || kind === "scheduled") {
        out.push({
          heading: "Scheduled",
          rows: tasks.map((task) => ({ key: `s-${task.id}`, label: task.name, icon: "clock", onRun: () => onOpenScheduled(task.id) })),
        });
      }
      if (kind === "all" || kind === "sessions" || kind === "code") {
        out.push({ heading: "Actions", rows: [{ key: "new", label: "New session", icon: "plus", onRun: onNew }] });
      }
      return out;
    }

    if (!results) return out;
    if (wantsChats) {
      out.push({
        heading: "Chats",
        rows: results.threads.filter((thread) => keepThread(thread.id)).map(threadRow),
      });
      out.push({
        heading: "Messages",
        rows: results.turns
          .filter((turn) => keepThread(turn.thread_id))
          .map((turn) => ({
            key: `m-${turn.id}`,
            label: turn.thread_title,
            detail: turn.input.slice(0, 120),
            icon: isCode(turn.thread_id) ? "code" : "chat",
            onRun: () => onOpenThread(turn.thread_id),
          })),
      });
    }
    if (kind === "all" || kind === "artifacts") {
      out.push({
        heading: "Artifacts",
        rows: results.artifacts.map((artifact) => ({ key: `a-${artifact.id}`, label: artifact.title, detail: artifact.kind, icon: "artifact", onRun: onOpenArtifacts })),
      });
    }
    if (kind === "all" || kind === "projects") {
      out.push({
        heading: "Projects",
        rows: results.projects.map((project) => ({ key: `p-${project.id}`, label: project.name, icon: "project", onRun: onOpenProjects })),
      });
      out.push({
        heading: "Project files",
        rows: fileHits.map((file) => ({
          key: `f-${file.id}`,
          label: file.filename,
          detail: file.snippet ?? file.project_name,
          icon: "project",
          onRun: () => onOpenProject(file.project_id),
        })),
      });
    }
    if (kind === "all" || kind === "scheduled") {
      out.push({
        heading: "Scheduled",
        rows: tasks
          .filter((task) => task.name.toLowerCase().includes(q) || task.prompt.toLowerCase().includes(q))
          .map((task) => ({ key: `s-${task.id}`, label: task.name, icon: "clock", onRun: () => onOpenScheduled(task.id) })),
      });
    }
    return out;
    // The row handlers are recreated each render; the data they depend on is listed here.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query, kind, results, fileHits, tasks, threads, isCode]);

  const rows = sections.flatMap((section) => section.rows);
  const shown = Math.min(highlight, Math.max(rows.length - 1, 0));
  const hasQuery = query.trim().length > 0;

  const run = (row: Row | undefined) => {
    if (!row) return;
    row.onRun();
    onClose();
  };

  const cycleKind = (step: number) => {
    const index = SEARCH_KINDS.findIndex((item) => item.id === kind);
    setKind(SEARCH_KINDS[(index + step + SEARCH_KINDS.length) % SEARCH_KINDS.length].id);
    setHighlight(0);
  };

  return (
    <div className="modal-backdrop" onMouseDown={onClose}>
      <div
        className="search-dialog"
        role="dialog"
        aria-modal="true"
        aria-label={tx("Search")}
        onMouseDown={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key === "Escape") onClose();
          if (event.altKey && event.key === "ArrowRight") {
            event.preventDefault();
            cycleKind(1);
          }
          if (event.altKey && event.key === "ArrowLeft") {
            event.preventDefault();
            cycleKind(-1);
          }
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
          <button className="icon-button" onClick={onClose} aria-label={tx("Close search")}>
            ×
          </button>
        </div>

        <div className="search-types" role="tablist" aria-label={tx("Type")}>
          {SEARCH_KINDS.map((item) => (
            <button
              key={item.id}
              role="tab"
              aria-selected={kind === item.id}
              className={`search-type ${kind === item.id ? "search-type-on" : ""}`}
              onClick={() => {
                setKind(item.id);
                setHighlight(0);
              }}
            >
              {tx(item.label)}
            </button>
          ))}
        </div>

        <div className="search-results">
          {sections.map((section) =>
            section.rows.length === 0 ? null : (
              <div key={section.heading}>
                <div className="search-heading">{tx(section.heading)}</div>
                {section.rows.map((row) => {
                  const index = rows.indexOf(row);
                  const Icon = ICONS[row.icon];
                  return (
                    <button
                      key={row.key}
                      className={`search-row ${index === shown ? "search-row-active" : ""}`}
                      onMouseEnter={() => setHighlight(index)}
                      onClick={() => run(row)}
                    >
                      <span className="search-row-icon">
                        <Icon width={18} height={18} />
                        {row.attention && <span className="search-attention" aria-label={tx("Needs attention")} />}
                      </span>
                      <span className="search-row-label">{tx(row.label)}</span>
                      {row.detail && <span className="search-row-detail">{row.detail}</span>}
                      {index === shown && <span className="search-row-hint" aria-hidden="true">↵</span>}
                    </button>
                  );
                })}
              </div>
            ),
          )}
          {hasQuery && results && rows.length === 0 && <p className="empty-note">{tx("No matches for")} “{query.trim()}”</p>}
          {!hasQuery && rows.length === 0 && <p className="empty-note">{tx("Nothing here yet.")}</p>}
        </div>

        <footer className="search-footer">
          <span>
            <kbd>↑↓</kbd> {tx("to move")}
          </span>
          <span>
            <kbd>⌥←→</kbd> {tx("change type")}
          </span>
          <span>
            <kbd>Enter</kbd> {tx("to open")}
          </span>
          <span>
            <kbd>Esc</kbd> {tx("to close")}
          </span>
        </footer>
      </div>
    </div>
  );
}
