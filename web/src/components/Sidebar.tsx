import { useEffect, useRef, useState } from "react";
import type { Thread, User } from "../api/types";
import type { SocketStatus } from "../realtime/socket";
import { ArtifactIcon, ChatIcon, ChevronIcon, ClockIcon, CustomizeIcon, MoreIcon, PlusIcon, ProjectIcon, SearchIcon, SidebarIcon } from "./icons";

interface SidebarProps {
  open: boolean;
  onToggle: () => void;
  threads: Thread[];
  activeId: string | null;
  onSelect: (threadId: string) => void;
  onNew: () => void;
  onSearch: () => void;
  onArtifacts: () => void;
  onCustomize: () => void;
  onScheduled: () => void;
  activeView: "chat" | "artifacts" | "customize" | "scheduled";
  user: User;
  socketStatus: SocketStatus;
  onSignOut: () => void;
}

const navItems = [
  { label: "New", Icon: PlusIcon },
  { label: "Search", Icon: SearchIcon },
  { label: "Projects", Icon: ProjectIcon },
  { label: "Artifacts", Icon: ArtifactIcon },
  { label: "Scheduled", Icon: ClockIcon },
  { label: "Customize", Icon: CustomizeIcon },
  { label: "More", Icon: MoreIcon },
];

const statusLabel: Record<SocketStatus, string> = {
  open: "Live",
  connecting: "Connecting…",
  closed: "Offline",
};

export function Sidebar({ open, onToggle, threads, activeId, onSelect, onNew, onSearch, onArtifacts, onCustomize, onScheduled, activeView, user, socketStatus, onSignOut }: SidebarProps) {
  const [recentsOpen, setRecentsOpen] = useState(true);
  const [moreOpen, setMoreOpen] = useState(false);
  const moreRef = useRef<HTMLDivElement>(null);

  // The More menu closes on Escape or a click elsewhere.
  useEffect(() => {
    if (!moreOpen) return;
    const onPointer = (event: MouseEvent) => {
      if (!moreRef.current?.contains(event.target as Node)) setMoreOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMoreOpen(false);
    };
    document.addEventListener("mousedown", onPointer);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onPointer);
      document.removeEventListener("keydown", onKey);
    };
  }, [moreOpen]);

  return (
    <aside className={`sidebar ${open ? "" : "sidebar-collapsed"}`} aria-label="Sidebar">
      <div className="sidebar-top">
        <div className="brand">
          <button className="icon-button" onClick={onToggle} aria-label="Toggle sidebar">
            <SidebarIcon />
          </button>
          <span className="brand-name">Claude</span>
        </div>
      </div>

      <nav className="nav">
        {navItems.map(({ label, Icon }) =>
          label === "New" || label === "Search" || label === "Artifacts" || label === "Customize" || label === "Scheduled" ? (
            <button
              key={label}
              className={`nav-item ${(label === "Artifacts" && activeView === "artifacts") || (label === "Customize" && activeView === "customize") || (label === "Scheduled" && activeView === "scheduled") ? "nav-item-active" : ""}`}
              onClick={
                label === "New"
                  ? onNew
                  : label === "Search"
                    ? onSearch
                    : label === "Artifacts"
                      ? onArtifacts
                      : label === "Customize"
                        ? onCustomize
                        : onScheduled
              }
            >
              <Icon />
              <span>{label}</span>
            </button>
          ) : label === "More" ? (
            <div key={label} className="more-wrap" ref={moreRef}>
              <button
                className={`nav-item ${moreOpen ? "nav-item-active" : ""}`}
                onClick={() => setMoreOpen(!moreOpen)}
                aria-expanded={moreOpen}
                aria-haspopup="menu"
              >
                <Icon />
                <span>{label}</span>
              </button>
              {moreOpen && (
                <div className="more-menu" role="menu">
                  <button role="menuitem" className="more-item" disabled>
                    <ProjectIcon width={16} height={16} />
                    <span>Projects</span>
                    <span className="badge">Beta</span>
                  </button>
                  <button
                    role="menuitem"
                    className="more-item more-item-enabled"
                    onClick={() => {
                      setMoreOpen(false);
                      onScheduled();
                    }}
                  >
                    <ClockIcon width={16} height={16} />
                    <span>Scheduled</span>
                  </button>
                  <hr className="more-divider" />
                  <button role="menuitem" className="more-item" disabled>
                    <span>Edit sidebar…</span>
                  </button>
                </div>
              )}
            </div>
          ) : (
            <a key={label} className="nav-item" href="#" onClick={(event) => event.preventDefault()} aria-disabled="true">
              <Icon />
              <span>{label}</span>
            </a>
          ),
        )}
      </nav>

      <div className="sidebar-scroll">
        <section>
          <button className="section-heading" onClick={() => setRecentsOpen(!recentsOpen)} aria-expanded={recentsOpen}>
            <span>Recents</span>
            <ChevronIcon className={recentsOpen ? "" : "chevron-closed"} />
          </button>
          {recentsOpen && (
            <div className="chat-list">
              {threads.length === 0 && <p className="empty-note">No chats yet</p>}
              {threads.map((thread) => (
                <button
                  key={thread.id}
                  className={`chat-row ${thread.id === activeId ? "chat-row-active" : ""}`}
                  onClick={() => onSelect(thread.id)}
                  title={thread.title}
                >
                  <ChatIcon className="chat-row-icon" />
                  <span className="chat-row-title">{thread.title}</span>
                  {thread.state === "running" && <span className="unread-dot" aria-label="running" />}
                  {thread.state === "failed" && <span className="failed-dot" aria-label="failed" />}
                </button>
              ))}
            </div>
          )}
        </section>
      </div>

      <footer className="sidebar-footer">
        <div className="avatar">{(user.display_name || user.email).charAt(0).toUpperCase()}</div>
        <div className="account">
          <span className="account-name">{user.display_name}</span>
          <span className="account-plan"> · {statusLabel[socketStatus]}</span>
        </div>
        <button className="icon-button" onClick={onSignOut} aria-label="Sign out" title="Sign out">
          <ChevronIcon />
        </button>
      </footer>
    </aside>
  );
}
