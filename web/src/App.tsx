import { useState } from "react";
import { Sidebar } from "./components/Sidebar";
import { Home } from "./components/Home";
import { currentUser } from "./data/mock";
import { SidebarIcon } from "./components/icons";

export function App() {
  // Open on wide screens; start closed on narrow ones, where it overlays the page.
  const [sidebarOpen, setSidebarOpen] = useState(() => window.matchMedia("(min-width: 761px)").matches);

  return (
    <div className="app">
      <Sidebar open={sidebarOpen} onToggle={() => setSidebarOpen(!sidebarOpen)} />
      <div className="workspace">
        {!sidebarOpen && (
          <button className="icon-button reopen" onClick={() => setSidebarOpen(true)} aria-label="Open sidebar">
            <SidebarIcon />
          </button>
        )}
        <Home name={currentUser?.name} />
      </div>
    </div>
  );
}
