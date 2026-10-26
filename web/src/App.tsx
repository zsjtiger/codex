import { useCallback, useEffect, useState } from "react";
import { projects as projectApi } from "./api/resources";
import type { Artifact, Project, ScheduledTask } from "./api/types";
import { ArtifactsPage } from "./components/ArtifactsPage";
import { CustomizePage } from "./components/CustomizePage";
import { Home } from "./components/Home";
import { ProjectsPage } from "./components/ProjectsPage";
import { ScheduledPage } from "./components/ScheduledPage";
import { SearchDialog } from "./components/SearchDialog";
import { SignIn } from "./components/SignIn";
import { type NavLabel, Sidebar, type View } from "./components/Sidebar";
import { SidebarIcon } from "./components/icons";
import { useCodex } from "./state/useCodex";

const NAV_KEY = "codex.sidebar.hidden";

function loadHidden(): NavLabel[] {
  try {
    return JSON.parse(localStorage.getItem(NAV_KEY) ?? "[]") as NavLabel[];
  } catch {
    return [];
  }
}

export function App() {
  // Open on wide screens; start closed on narrow ones, where it overlays the page.
  const [sidebarOpen, setSidebarOpen] = useState(() => window.matchMedia("(min-width: 761px)").matches);
  const [view, setView] = useState<View>("chat");
  const [searchOpen, setSearchOpen] = useState(false);
  const [hidden, setHidden] = useState<NavLabel[]>(loadHidden);
  const [projects, setProjects] = useState<Project[]>([]);
  const [notice, setNotice] = useState<string | null>(null);
  const codex = useCodex();

  const workspaceId = codex.workspace?.id ?? null;
  const isAdmin = codex.workspace ? codex.workspace.role === "admin" || codex.workspace.role === "owner" : false;
  const userId = codex.user?.id ?? "";

  const reloadProjects = useCallback(async () => {
    if (!workspaceId) return;
    setProjects(await projectApi.list(workspaceId));
  }, [workspaceId]);

  useEffect(() => {
    void reloadProjects().catch((err) => setNotice(err instanceof Error ? err.message : "Could not load projects."));
  }, [reloadProjects]);

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

  const changeHidden = (next: NavLabel[]) => {
    setHidden(next);
    try {
      localStorage.setItem(NAV_KEY, JSON.stringify(next));
    } catch {
      // Preferences are optional; the sidebar still works without storage.
    }
  };

  if (codex.checking) return <div className="loading">Loading…</div>;
  if (!codex.user || !workspaceId) return <SignIn onSignIn={codex.signIn} />;

  const error = notice ?? codex.error;
  const dismiss = () => {
    setNotice(null);
    codex.dismissError();
  };
  const onError = (message: string) => setNotice(message);
  const canEditOwned = (createdBy: string | null) => isAdmin || createdBy === userId;
  const openThread = (threadId: string) => {
    setView("chat");
    void codex.selectThread(threadId);
  };
  const newChat = () => {
    setView("chat");
    codex.newChat();
  };
  const activeThread = codex.threads.find((t) => t.id === codex.activeId) ?? null;

  return (
    <div className="app">
      <Sidebar
        open={sidebarOpen}
        onToggle={() => setSidebarOpen(!sidebarOpen)}
        threads={codex.threads}
        activeId={codex.activeId}
        activeView={view}
        onSelect={openThread}
        onNew={newChat}
        onSearch={() => setSearchOpen(true)}
        onView={setView}
        hidden={hidden}
        onHiddenChange={changeHidden}
        user={codex.user}
        socketStatus={codex.socketStatus}
        onSignOut={() => void codex.signOut()}
      />
      {searchOpen && (
        <SearchDialog
          workspaceId={workspaceId}
          threads={codex.threads}
          onOpenThread={openThread}
          onOpenArtifacts={() => setView("artifacts")}
          onOpenProjects={() => setView("projects")}
          onNew={newChat}
          onClose={() => setSearchOpen(false)}
          onError={onError}
        />
      )}
      <div className="workspace">
        {!sidebarOpen && (
          <button className="icon-button reopen" onClick={() => setSidebarOpen(true)} aria-label="Open sidebar">
            <SidebarIcon />
          </button>
        )}
        {view === "projects" && (
          <ProjectsPage
            workspaceId={workspaceId}
            projects={projects}
            reload={reloadProjects}
            canEdit={(p: Project) => canEditOwned(p.created_by)}
            onStartChat={(projectId) => {
              codex.setDraftProjectId(projectId);
              newChat();
            }}
            onBack={() => setView("chat")}
            onError={onError}
          />
        )}
        {view === "artifacts" && (
          <ArtifactsPage
            workspaceId={workspaceId}
            userId={userId}
            projects={projects}
            canEdit={(a: Artifact) => canEditOwned(a.created_by)}
            onBack={() => setView("chat")}
            onError={onError}
          />
        )}
        {view === "customize" && <CustomizePage workspaceId={workspaceId} isAdmin={isAdmin} onBack={() => setView("chat")} onError={onError} />}
        {view === "scheduled" && (
          <ScheduledPage
            workspaceId={workspaceId}
            projects={projects}
            canEdit={(t: ScheduledTask) => canEditOwned(t.created_by)}
            onOpenThread={openThread}
            onBack={() => setView("chat")}
            onError={onError}
          />
        )}
        {view === "chat" && (
          <Home
            user={codex.user}
            workspaceId={workspaceId}
            thread={activeThread}
            turns={codex.turns}
            agentText={codex.agentText}
            toolUses={codex.toolUses}
            approvals={codex.approvals}
            sending={codex.sending}
            projects={projects}
            draftProjectId={codex.draftProjectId}
            onDraftProject={codex.setDraftProjectId}
            onThreadChanged={() => void codex.refreshThreads()}
            onSend={(text, options) => void codex.sendMessage(text, options)}
            onStop={(turnId) => void codex.interrupt(turnId)}
            onAnswer={codex.answerApproval}
            onOpenArtifacts={() => setView("artifacts")}
            upload={codex.uploadAttachment}
            error={error}
            onError={onError}
            onDismissError={dismiss}
          />
        )}
      </div>
    </div>
  );
}
