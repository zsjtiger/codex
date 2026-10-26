import { useState } from "react";
import { ArtifactIcon, ChatIcon, ChevronIcon, ClockIcon, CustomizeIcon, FileIcon, MoreIcon, PinIcon, PlusIcon, ProjectIcon, SearchIcon, SidebarIcon } from "./icons";
import { type ChatItem, pinned, recents, user } from "../data/mock";

interface SidebarProps {
  open: boolean;
  onToggle: () => void;
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

function ChatRow({ item }: { item: ChatItem }) {
  return (
    <a className="chat-row" href="#" title={item.title}>
      <ChatIcon className="chat-row-icon" />
      <span className="chat-row-title">{item.title}</span>
      {item.hasFile && <FileIcon className="row-marker" />}
      {item.scheduled && <ClockIcon className="row-marker" width={14} height={14} />}
      {item.unread && <span className="unread-dot" aria-label="unread" />}
    </a>
  );
}

export function Sidebar({ open, onToggle }: SidebarProps) {
  const [pinnedOpen, setPinnedOpen] = useState(true);
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
        {navItems.map(({ label, Icon }) => (
          <a key={label} className="nav-item" href="#">
            <Icon />
            <span>{label}</span>
          </a>
        ))}
      </nav>

      <div className="sidebar-scroll">
        <section>
          <button className="section-heading" onClick={() => setPinnedOpen(!pinnedOpen)} aria-expanded={pinnedOpen}>
            <span>Pinned</span>
            <ChevronIcon className={pinnedOpen ? "" : "chevron-closed"} />
          </button>
          {pinnedOpen && (
            <div className="chat-list">
              {pinned.map((item) => (
                <ChatRow key={item.id} item={item} />
              ))}
            </div>
          )}
        </section>

        <section>
          <button className="section-heading" onClick={() => setRecentsOpen(!recentsOpen)} aria-expanded={recentsOpen}>
            <span>Recents</span>
            <ChevronIcon className={recentsOpen ? "" : "chevron-closed"} />
          </button>
          {recentsOpen && (
            <div className="chat-list">
              {recents.map((item) => (
                <ChatRow key={item.id} item={item} />
              ))}
              <a className="view-all" href="#">
                View all
              </a>
            </div>
          )}
        </section>
      </div>

      <footer className="sidebar-footer">
        <div className="avatar">{user.initial}</div>
        <div className="account">
          <span className="account-name">{user.name}</span>
          <span className="account-plan"> · {user.plan}</span>
        </div>
        <ChevronIcon className="footer-chevron" />
      </footer>
    </aside>
  );
}
