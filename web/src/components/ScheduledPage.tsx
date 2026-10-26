import { useState } from "react";
import { PlusIcon } from "./icons";

const scopes = ["Yours", "Templates"] as const;
const newTaskOptions = ["Create with Claude", "Set up manually"];

/**
 * Scheduled tasks shell: Yours / Templates, and a New task menu. The gateway has no
 * scheduled-task API yet, so both lists show empty states and the actions are disabled.
 */
export function ScheduledPage({ onBack }: { onBack: () => void }) {
  const [scope, setScope] = useState<(typeof scopes)[number]>("Yours");
  const [menuOpen, setMenuOpen] = useState(false);

  return (
    <main className="customize-page">
      <div className="customize-inner">
        <button className="link-button" onClick={onBack}>
          ← Back to chat
        </button>
        <h1>Scheduled tasks</h1>

        <div className="customize-toolbar">
          <div className="tabs" role="tablist">
            {scopes.map((name) => (
              <button
                key={name}
                role="tab"
                aria-selected={scope === name}
                className={`tab ${scope === name ? "tab-active" : ""}`}
                onClick={() => setScope(name)}
              >
                {name}
              </button>
            ))}
          </div>

          <div className="customize-tools">
            <div className="add-menu">
              <button className="add-button" onClick={() => setMenuOpen(!menuOpen)} aria-expanded={menuOpen}>
                <PlusIcon width={16} height={16} /> New task
              </button>
              {menuOpen && (
                <ul className="add-menu-list" role="menu">
                  {newTaskOptions.map((option) => (
                    <li key={option} role="menuitem" aria-disabled="true" className="add-menu-item">
                      {option}
                      <span className="badge">Soon</span>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </div>
        </div>

        <section>
          <p className="empty-state">
            {scope === "Yours" ? "No scheduled tasks yet." : "Templates are coming soon."}
          </p>
        </section>
      </div>
    </main>
  );
}
