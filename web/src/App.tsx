import { useEffect, useState } from "react";
import { Sidebar } from "./components/Sidebar";
import { Home } from "./components/Home";
import { SignIn } from "./components/SignIn";
import { SearchDialog } from "./components/SearchDialog";
import { ArtifactsPage } from "./components/ArtifactsPage";
import { CustomizePage } from "./components/CustomizePage";
import { SidebarIcon } from "./components/icons";
import { useCodex } from "./state/useCodex";

export function App() {
  // Open on wide screens; start closed on narrow ones, where it overlays the page.
  const [sidebarOpen, setSidebarOpen] = useState(() => window.matchMedia("(min-width: 761px)").matches);
  const codex = useCodex();
  const [searchOpen, setSearchOpen] = useState(false);
  const [view, setView] = useState<"chat" | "artifacts" | "customize">("chat");

  // Cmd/Ctrl+K opens search from anywhere in the app.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setSearchOpen((open) => !open);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  if (codex.checking) return <div className="loading">Loading…</div>;
  if (!codex.user) return <SignIn onSignIn={codex.signIn} />;

  const activeThread = codex.threads.find((t) => t.id === codex.activeId) ?? null;

  return (
    <div className="app">
      <Sidebar
        open={sidebarOpen}
        onToggle={() => setSidebarOpen(!sidebarOpen)}
        threads={codex.threads}
        activeId={codex.activeId}
        onSelect={(id) => void codex.selectThread(id)}
        onNew={() => {
          setView("chat");
          codex.newChat();
        }}
        onSearch={() => setSearchOpen(true)}
        onArtifacts={() => setView("artifacts")}
        onCustomize={() => setView("customize")}
        activeView={view}
        user={codex.user}
        socketStatus={codex.socketStatus}
        onSignOut={() => void codex.signOut()}
      />
      {searchOpen && (
        <SearchDialog
          threads={codex.threads}
          onSelect={(id) => void codex.selectThread(id)}
          onNew={codex.newChat}
          onClose={() => setSearchOpen(false)}
        />
      )}
      <div className="workspace">
        {!sidebarOpen && (
          <button className="icon-button reopen" onClick={() => setSidebarOpen(true)} aria-label="Open sidebar">
            <SidebarIcon />
          </button>
        )}
        {view === "artifacts" && <ArtifactsPage onBack={() => setView("chat")} />}
        {view === "customize" && <CustomizePage onBack={() => setView("chat")} />}
        {view === "chat" && (
        <Home
          user={codex.user}
          thread={activeThread}
          turns={codex.turns}
          agentText={codex.agentText}
          approvals={codex.approvals}
          sending={codex.sending}
          onSend={(text) => void codex.sendMessage(text)}
          onAnswer={codex.answerApproval}
          error={codex.error}
          onDismissError={codex.dismissError}
        />
        )}
      </div>
    </div>
  );
}
