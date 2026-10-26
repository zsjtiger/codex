import { useState } from "react";
import { Sidebar } from "./components/Sidebar";
import { Home } from "./components/Home";
import { SignIn } from "./components/SignIn";
import { SidebarIcon } from "./components/icons";
import { useCodex } from "./state/useCodex";

export function App() {
  // Open on wide screens; start closed on narrow ones, where it overlays the page.
  const [sidebarOpen, setSidebarOpen] = useState(() => window.matchMedia("(min-width: 761px)").matches);
  const codex = useCodex();

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
        onNew={codex.newChat}
        user={codex.user}
        socketStatus={codex.socketStatus}
        onSignOut={() => void codex.signOut()}
      />
      <div className="workspace">
        {!sidebarOpen && (
          <button className="icon-button reopen" onClick={() => setSidebarOpen(true)} aria-label="Open sidebar">
            <SidebarIcon />
          </button>
        )}
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
      </div>
    </div>
  );
}
