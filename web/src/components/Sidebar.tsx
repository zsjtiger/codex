import { useState } from "react";
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
  activeView: "chat" | "artifacts";
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

export function Sidebar({ open, onToggle, threads, activeId, onSelect, onNew, onSearch, onArtifacts, activeView, user, socketStatus, onSignOut }: SidebarProps) {
  const [recentsOpen, setRecentsOpen] = useState(true);

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
          label === "New" || label === "Search" || label === "Artifacts" ? (
            <button
              key={label}
              className={`nav-item ${label === "Artifacts" && activeView === "artifacts" ? "nav-item-active" : ""}`}
              onClick={label === "New" ? onNew : label === "Search" ? onSearch : onArtifacts}
            >
              <Icon />
              <span>{label}</span>
            </button>
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
