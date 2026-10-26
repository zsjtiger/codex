import { useState } from "react";
import { type Artifact, artifacts } from "../data/mock";
import { PlusIcon } from "./icons";

const tabs = ["All", "Yours", "Shared with you"] as const;

// Creation types shown on the page. None are wired to the backend yet.
const creators = [
  { label: "Docs", tone: "blue", badge: "New" },
  { label: "Slides", tone: "orange", badge: "New" },
  { label: "Design", tone: "purple", badge: "New" },
  { label: "Dashboard", tone: "red", badge: "Beta" },
];

export function ArtifactsPage({ onBack }: { onBack: () => void }) {
  const [tab, setTab] = useState<(typeof tabs)[number]>("All");
  // The gateway has no artifact API yet, so the list comes from placeholder data.
  const visible: Artifact[] = tab === "All" ? artifacts : [];

  return (
    <main className="artifacts-page">
      <div className="artifacts-inner">
        <button className="link-button" onClick={onBack}>
          ← Back to chat
        </button>
        <h1>Artifacts</h1>

        <div className="tabs" role="tablist">
          {tabs.map((name) => (
            <button
              key={name}
              role="tab"
              aria-selected={tab === name}
              className={`tab ${tab === name ? "tab-active" : ""}`}
              onClick={() => setTab(name)}
            >
              {name}
            </button>
          ))}
        </div>

        <section>
          <h2 className="section-label">Make something new</h2>
          <div className="creator-grid">
            {creators.map((c) => (
              <button key={c.label} className={`creator tone-${c.tone}`} disabled title="Coming soon">
                <span className="creator-icon" aria-hidden="true" />
                <span className="creator-label">
                  {c.label} <span className="badge">{c.badge}</span>
                </span>
              </button>
            ))}
          </div>
        </section>

        <section>
          {visible.length === 0 ? (
            <p className="empty-state">
              <PlusIcon width={16} height={16} /> {tab === "All" ? "No artifacts yet" : `Nothing under “${tab}” yet`}
            </p>
          ) : (
            <div className="artifact-grid">
              {visible.map((artifact) => (
                <a key={artifact.id} className="artifact-card" href="#" onClick={(e) => e.preventDefault()}>
                  <div className="artifact-preview" aria-hidden="true">
                    <div className="artifact-page" />
                  </div>
                  <div className="artifact-title">{artifact.title}</div>
                  <div className="artifact-meta">{artifact.updated}</div>
                </a>
              ))}
            </div>
          )}
        </section>
      </div>
    </main>
  );
}
