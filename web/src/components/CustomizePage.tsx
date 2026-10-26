import { useState } from "react";
import { PlusIcon, SearchIcon } from "./icons";

const sections = ["Skills", "Connectors", "Plugins"] as const;
const scopes = ["Yours", "Discover"] as const;
const addOptions = ["Upload skill", "Create a skill", "Create with Claude"];

/**
 * Customize shell: section tabs, scope switch, and an Add menu. Skills, connectors and
 * plugins have no gateway API yet, so each list shows an empty state and the actions are
 * disabled.
 */
export function CustomizePage({ onBack }: { onBack: () => void }) {
  const [section, setSection] = useState<(typeof sections)[number]>("Skills");
  const [scope, setScope] = useState<(typeof scopes)[number]>("Yours");
  const [menuOpen, setMenuOpen] = useState(false);

  return (
    <main className="customize-page">
      <div className="customize-inner">
        <button className="link-button" onClick={onBack}>
          ← Back to chat
        </button>
        <h1>Customize</h1>

        <div className="customize-toolbar">
          <div className="tabs" role="tablist">
            {sections.map((name) => (
              <button
                key={name}
                role="tab"
                aria-selected={section === name}
                className={`tab ${section === name ? "tab-active" : ""}`}
                onClick={() => setSection(name)}
              >
                {name}
              </button>
            ))}
          </div>

          <div className="customize-tools">
            <div className="segmented" role="group" aria-label="Scope">
              {scopes.map((name) => (
                <button
                  key={name}
                  className={scope === name ? "segment-active" : ""}
                  aria-pressed={scope === name}
                  onClick={() => setScope(name)}
                >
                  {name}
                </button>
              ))}
            </div>
            <button className="icon-button" aria-label="Search" disabled>
              <SearchIcon />
            </button>
            <div className="add-menu">
              <button className="add-button" onClick={() => setMenuOpen(!menuOpen)} aria-expanded={menuOpen}>
                <PlusIcon width={16} height={16} /> Add
              </button>
              {menuOpen && (
                <ul className="add-menu-list" role="menu">
                  {addOptions.map((option) => (
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
            {scope === "Yours"
              ? `No ${section.toLowerCase()} added yet.`
              : `Nothing to discover for ${section.toLowerCase()} yet.`}
          </p>
        </section>
      </div>
    </main>
  );
}
