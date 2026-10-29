import { useEffect, useRef, useState } from "react";
import type { Thread, User } from "../api/types";
import type { SocketStatus } from "../realtime/socket";
import { UsageDialog } from "./UsageDialog";
import { ArtifactIcon, ChatIcon, ChevronIcon, ClockIcon, CustomizeIcon, MoreIcon, PlusIcon, ProjectIcon, SearchIcon, SidebarIcon } from "./icons";

export type NavLabel = "New" | "Search" | "Projects" | "Artifacts" | "Scheduled" | "Customize";
export type View = "chat" | "artifacts" | "customize" | "scheduled" | "projects";

const navItems: { label: NavLabel; Icon: typeof PlusIcon }[] = [
  { label: "New", Icon: PlusIcon },
  { label: "Search", Icon: SearchIcon },
  { label: "Projects", Icon: ProjectIcon },
  { label: "Artifacts", Icon: ArtifactIcon },
  { label: "Scheduled", Icon: ClockIcon },
  { label: "Customize", Icon: CustomizeIcon },
];

const viewFor: Partial<Record<NavLabel, View>> = {
  Projects: "projects",
  Artifacts: "artifacts",
  Scheduled: "scheduled",
  Customize: "customize",
};

interface SidebarProps {
  open: boolean;
  onToggle: () => void;
  threads: Thread[];
  activeId: string | null;
  activeView: View;
  onSelect: (threadId: string) => void;
  onNew: () => void;
  onSearch: () => void;
  onView: (view: View) => void;
  hidden: NavLabel[];
  onHiddenChange: (hidden: NavLabel[]) => void;
  user: User;
  socketStatus: SocketStatus;
  onSignOut: () => void;
}

const statusLabel: Record<SocketStatus, string> = { open: "Live", connecting: "Connecting…", closed: "Offline" };

export function Sidebar(props: SidebarProps) {
  const { open, onToggle, threads, activeId, activeView, onSelect, onNew, onSearch, onView, hidden, onHiddenChange, user, socketStatus, onSignOut } = props;
  const [recentsOpen, setRecentsOpen] = useState(true);
  const [moreOpen, setMoreOpen] = useState(false);
  const [editing, setEditing] = useState(false);
  const moreRef = useRef<HTMLDivElement>(null);
  const [accountOpen, setAccountOpen] = useState(false);
  const [usageOpen, setUsageOpen] = useState(false);
  const accountRef = useRef<HTMLDivElement>(null);

  // The account menu closes on Escape or a click elsewhere.
  useEffect(() => {
    if (!accountOpen) return;
    const onPointer = (event: MouseEvent) => {
      if (!accountRef.current?.contains(event.target as Node)) setAccountOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setAccountOpen(false);
    };
    document.addEventListener("mousedown", onPointer);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onPointer);
      document.removeEventListener("keydown", onKey);
    };
  }, [accountOpen]);

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

  const activate = (label: NavLabel) => {
    if (label === "New") onNew();
    else if (label === "Search") onSearch();
    else onView(viewFor[label]!);
  };

  const isActive = (label: NavLabel) => viewFor[label] === activeView;
  const visibleItems = navItems.filter((item) => editing || !hidden.includes(item.label));

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
        {visibleItems.map(({ label, Icon }) => (
          <div key={label} className="nav-row">
            <button className={`nav-item ${isActive(label) ? "nav-item-active" : ""}`} onClick={() => activate(label)}>
              <Icon />
              <span>{label}</span>
            </button>
            {editing && (
              <input
                type="checkbox"
                aria-label={`Show ${label}`}
                checked={!hidden.includes(label)}
                onChange={(e) => onHiddenChange(e.target.checked ? hidden.filter((h) => h !== label) : [...hidden, label])}
              />
            )}
          </div>
        ))}

        <div className="more-wrap" ref={moreRef}>
          <button className={`nav-item ${moreOpen ? "nav-item-active" : ""}`} onClick={() => setMoreOpen(!moreOpen)} aria-expanded={moreOpen} aria-haspopup="menu">
            <MoreIcon />
            <span>More</span>
          </button>
          {moreOpen && (
            <div className="more-menu" role="menu">
              <button role="menuitem" className="more-item more-item-enabled" onClick={() => { setMoreOpen(false); onView("projects"); }}>
                <ProjectIcon width={16} height={16} />
                <span>Projects</span>
              </button>
              <button role="menuitem" className="more-item more-item-enabled" onClick={() => { setMoreOpen(false); onView("scheduled"); }}>
                <ClockIcon width={16} height={16} />
                <span>Scheduled</span>
              </button>
              <hr className="more-divider" />
              <button role="menuitem" className="more-item more-item-enabled" onClick={() => { setMoreOpen(false); setEditing(!editing); }}>
                <span>{editing ? "Done editing sidebar" : "Edit sidebar…"}</span>
              </button>
            </div>
          )}
        </div>
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
                <button key={thread.id} className={`chat-row ${thread.id === activeId ? "chat-row-active" : ""}`} onClick={() => onSelect(thread.id)} title={thread.title}>
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

      <footer className="sidebar-footer" ref={accountRef}>
        {accountOpen && (
          <div className="account-menu" role="menu">
            <div className="account-menu-email">{user.email}</div>
            <button
              role="menuitem"
              className="account-menu-item"
              onClick={() => {
                setAccountOpen(false);
                setUsageOpen(true);
              }}
            >
              Usage
            </button>
            <button
              role="menuitem"
              className="account-menu-item"
              onClick={() => {
                setAccountOpen(false);
                onSignOut();
              }}
            >
              Log out
            </button>
          </div>
        )}
        <button
          className="account-button"
          aria-haspopup="menu"
          aria-expanded={accountOpen}
          onClick={() => setAccountOpen(!accountOpen)}
        >
          <div className="avatar">{(user.display_name || user.email).charAt(0).toUpperCase()}</div>
          <div className="account">
            <span className="account-name">{user.display_name}</span>
            <span className="account-plan"> · {statusLabel[socketStatus]}</span>
          </div>
          <ChevronIcon />
        </button>
      </footer>
      {usageOpen && <UsageDialog onClose={() => setUsageOpen(false)} />}
    </aside>
  );
}
