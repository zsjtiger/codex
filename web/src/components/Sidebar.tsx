import { useEffect, useRef, useState } from "react";
import type { Thread, User } from "../api/types";
import type { SocketStatus } from "../realtime/socket";
import { UsageDialog } from "./UsageDialog";
import { useI18n, type MessageKey } from "../i18n";
import { ArtifactIcon, ChatIcon, ChevronIcon, ClockIcon, CodeIcon, CustomizeIcon, MoreIcon, PlusIcon, ProjectIcon, SearchIcon, SidebarIcon, SparkIcon } from "./icons";
import { isDesktop } from "../desktop";
import { WindowLights } from "./TitleBar";
import { tx } from "../i18n";

export type NavLabel = "New" | "Search" | "Projects" | "Artifacts" | "Scheduled" | "Customize";
export type View = "chat" | "artifacts" | "customize" | "scheduled" | "projects" | "code";
export type Mode = "chat" | "code";

const navItems: { label: NavLabel; Icon: typeof PlusIcon; key: MessageKey }[] = [
  { label: "New", Icon: PlusIcon, key: "nav.new" },
  { label: "Search", Icon: SearchIcon, key: "nav.search" },
  { label: "Projects", Icon: ProjectIcon, key: "nav.projects" },
  { label: "Artifacts", Icon: ArtifactIcon, key: "nav.artifacts" },
  { label: "Scheduled", Icon: ClockIcon, key: "nav.scheduled" },
  { label: "Customize", Icon: CustomizeIcon, key: "nav.customize" },
];

const viewFor: Partial<Record<NavLabel, View>> = {
  Projects: "projects",
  Artifacts: "artifacts",
  Scheduled: "scheduled",
  Customize: "customize",
};

interface SidebarProps {
  open: boolean;
  onRenameThread: (threadId: string, title: string) => void;
  onArchiveThread: (threadId: string) => void;
  onDeleteThread: (threadId: string) => void;
  onOpenSettings: () => void;
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
  workspaceName: string;
  mode: Mode;
  onMode: (mode: Mode) => void;
  socketStatus: SocketStatus;
  onSignOut: () => void;
}

const statusLabel: Record<SocketStatus, string> = { open: "Live", connecting: tx("Connecting…"), closed: "Offline" };

export function Sidebar(props: SidebarProps) {
  const { t, locale, setLocale } = useI18n();
  const { open, onOpenSettings, onRenameThread, onArchiveThread, onDeleteThread, onToggle, threads, activeId, activeView, onSelect, onNew, onSearch, onView, hidden, onHiddenChange, user, workspaceName, socketStatus, onSignOut, mode, onMode } = props;
  const [chatMenuId, setChatMenuId] = useState<string | null>(null);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameText, setRenameText] = useState("");

  // Closes the chat menu on a click elsewhere.
  useEffect(() => {
    if (!chatMenuId) return;
    const close = () => setChatMenuId(null);
    document.addEventListener("click", close);
    return () => document.removeEventListener("click", close);
  }, [chatMenuId]);
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
    <aside className={`sidebar ${open ? "" : "sidebar-collapsed"}`} aria-label={tx("Sidebar")}>
      <div className="sidebar-top" data-tauri-drag-region={isDesktop() ? "" : undefined}>
        {isDesktop() && <WindowLights />}
        <div className="brand">
          <button className="icon-button" onClick={onToggle} aria-label={tx("Toggle sidebar")}>
            <SidebarIcon />
          </button>
          <span className="brand-name">{mode === "code" ? "BaizeCode" : tx("Baize")}</span>
        </div>
        <div className="mode-toggle" role="group" aria-label={tx("Mode")}>
          <button className={mode === "chat" ? "mode-on" : ""} aria-pressed={mode === "chat"} title={tx("Chat")} aria-label={tx("Chat")} onClick={() => onMode("chat")}>
            <ChatIcon />
          </button>
          <button className={mode === "code" ? "mode-on" : ""} aria-pressed={mode === "code"} title={tx("Code")} aria-label={tx("Code")} onClick={() => onMode("code")}>
            <CodeIcon />
          </button>
        </div>
      </div>

      <nav className="nav">
        {visibleItems.map(({ label, Icon }) => (
          <div key={label} className="nav-row">
            <button className={`nav-item ${isActive(label) ? "nav-item-active" : ""}`} onClick={() => activate(label)}>
              <Icon />
              <span>{t(navItems.find((item) => item.label === label)!.key)}</span>
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
            <span>{t("nav.more")}</span>
          </button>
          {moreOpen && (
            <div className="more-menu" role="menu">
              <button role="menuitem" className="more-item more-item-enabled" onClick={() => { setMoreOpen(false); onView("projects"); }}>
                <ProjectIcon width={16} height={16} />
                <span>{tx("Projects")}</span>
              </button>
              <button role="menuitem" className="more-item more-item-enabled" onClick={() => { setMoreOpen(false); onView("scheduled"); }}>
                <ClockIcon width={16} height={16} />
                <span>{tx("Scheduled")}</span>
              </button>
              <hr className="more-divider" />
              <button role="menuitem" className="more-item more-item-enabled" onClick={() => { setMoreOpen(false); setEditing(!editing); }}>
                <span>{editing ? tx("Done editing sidebar") : tx("Edit sidebar…")}</span>
              </button>
            </div>
          )}
        </div>
      </nav>

      <div className="sidebar-scroll">
        <section>
          <button className="section-heading" onClick={() => setRecentsOpen(!recentsOpen)} aria-expanded={recentsOpen}>
            <span>{t("nav.recents")}</span>
            <ChevronIcon className={recentsOpen ? "" : "chevron-closed"} />
          </button>
          {recentsOpen && (
            <div className="chat-list">
              {threads.length === 0 && <p className="empty-note">{tx("No chats yet")}</p>}
              {threads.map((thread) =>
                renamingId === thread.id ? (
                  <form
                    key={thread.id}
                    className="chat-rename"
                    onSubmit={(event) => {
                      event.preventDefault();
                      const title = renameText.trim();
                      setRenamingId(null);
                      if (title && title !== thread.title) onRenameThread(thread.id, title);
                    }}
                  >
                    <input
                      autoFocus
                      value={renameText}
                      maxLength={200}
                      onChange={(e) => setRenameText(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Escape") setRenamingId(null);
                      }}
                      onBlur={() => setRenamingId(null)}
                      aria-label={tx("Chat name")}
                    />
                  </form>
                ) : (
                  <div key={thread.id} className="chat-row-wrap" data-menu-for={thread.id}>
                    <button className={`chat-row ${thread.id === activeId ? "chat-row-active" : ""}`} onClick={() => onSelect(thread.id)} title={thread.title}>
                      <ChatIcon className="chat-row-icon" />
                      <span className="chat-row-title">{thread.title}</span>
                      {thread.state === "running" && <span className="unread-dot" aria-label={tx("running")} />}
                      {thread.state === "failed" && <span className="failed-dot" aria-label={tx("failed")} />}
                    </button>
                    <button
                      className="chat-row-more"
                      aria-label={`${tx("Actions for")} ${thread.title}`}
                      aria-haspopup="menu"
                      aria-expanded={chatMenuId === thread.id}
                      onClick={(e) => {
                        e.stopPropagation();
                        setChatMenuId(chatMenuId === thread.id ? null : thread.id);
                      }}
                    >
                      ⋯
                    </button>
                    {chatMenuId === thread.id && (
                      <div className="chat-menu" role="menu">
                        <button role="menuitem" onClick={() => { setChatMenuId(null); setRenameText(thread.title); setRenamingId(thread.id); }}>{t("chat.rename")}</button>
                        <button role="menuitem" onClick={() => { setChatMenuId(null); onArchiveThread(thread.id); }}>{t("chat.archive")}</button>
                        <button
                          role="menuitem"
                          className="chat-menu-danger"
                          onClick={() => {
                            setChatMenuId(null);
                            if (window.confirm(`Delete “${thread.title}”? This cannot be undone.`)) onDeleteThread(thread.id);
                          }}
                        >
                          {t("chat.delete")}
                        </button>
                      </div>
                    )}
                  </div>
                ),
              )}
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
                onOpenSettings();
              }}
            >
              {t("account.settings")}
            </button>
            <button
              role="menuitem"
              className="account-menu-item"
              onClick={() => {
                setAccountOpen(false);
                setUsageOpen(true);
              }}
            >
              {t("account.usage")}
            </button>
            <button
              role="menuitem"
              className="account-menu-item"
              onClick={() => {
                setAccountOpen(false);
                setLocale(locale === "en" ? "zh" : "en");
              }}
            >
              {t("account.language")}: {locale === "en" ? "English" : "中文"}
            </button>
            <button
              role="menuitem"
              className="account-menu-item"
              onClick={() => {
                setAccountOpen(false);
                onSignOut();
              }}
            >
              {t("account.logout")}
            </button>
          </div>
        )}
        <button
          className="account-button"
          aria-haspopup="menu"
          aria-expanded={accountOpen}
          onClick={() => setAccountOpen(!accountOpen)}
        >
          <SparkIcon className="footer-mark" />
          <div className="account">
            <span className="account-name">{workspaceName || user.display_name}</span>
            <span className="account-plan"> · {tx("Gateway")}</span>
          </div>
          <span className={`status-dot status-${socketStatus}`} title={statusLabel[socketStatus]} />
          <ChevronIcon />
        </button>
      </footer>
      {usageOpen && <UsageDialog onClose={() => setUsageOpen(false)} />}
    </aside>
  );
}
