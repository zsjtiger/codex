import { useState } from "react";
import { artifacts } from "../data/mock";
import { MicIcon, PlusIcon, SparkIcon, WaveIcon, ChevronIcon } from "./icons";

const models = ["Haiku 5.5", "Sonnet 5.5", "Opus 5.5"];
const efforts = ["Low", "Medium", "High"];

function greeting(date: Date): string {
  const hour = date.getHours();
  if (hour < 12) return "Good morning";
  if (hour < 18) return "Good afternoon";
  return "Good evening";
}

export function Home({ name }: { name?: string }) {
  const [draft, setDraft] = useState("");
  const [model, setModel] = useState(models[0]);
  const [effort, setEffort] = useState(efforts[1]);
  const [mode, setMode] = useState<"Manual" | "Auto">("Manual");
  const [project, setProject] = useState("Project");

  return (
    <main className="home">
      <div className="home-inner">
        <h1 className="greeting">
          <SparkIcon className="greeting-mark" />
          <span>{name ? `${greeting(new Date())}, ${name}` : greeting(new Date())}</span>
        </h1>

        <form
          className="composer"
          onSubmit={(event) => {
            event.preventDefault();
            setDraft("");
          }}
        >
          <textarea
            className="composer-input"
            placeholder="How can I help you today?"
            rows={2}
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && !event.shiftKey) {
                event.currentTarget.form?.requestSubmit();
              }
            }}
          />
          <div className="composer-bar">
            <button type="button" className="icon-button" aria-label="Attach">
              <PlusIcon />
            </button>
            <div className="composer-right">
              <button type="button" className="icon-button" aria-label="Voice input">
                <MicIcon />
              </button>
              <button type="button" className="icon-button" aria-label="Voice mode">
                <WaveIcon />
              </button>
            </div>
          </div>
        </form>

        <div className="composer-options">
          <button type="button" className="pill">
            {project}
            <ChevronIcon />
          </button>
          <div className="composer-options-right">
            <label className="pill select">
              <select value={model} onChange={(event) => setModel(event.target.value)} aria-label="Model">
                {models.map((m) => (
                  <option key={m}>{m}</option>
                ))}
              </select>
            </label>
            <label className="pill select">
              <select value={effort} onChange={(event) => setEffort(event.target.value)} aria-label="Effort">
                {efforts.map((e) => (
                  <option key={e}>{e}</option>
                ))}
              </select>
            </label>
            <button
              type="button"
              className="pill"
              onClick={() => setMode(mode === "Manual" ? "Auto" : "Manual")}
              aria-label="Permission mode"
            >
              {mode}
            </button>
          </div>
        </div>

        <section className="recent-artifacts">
          <div className="recent-heading">
            <span>Recent artifacts</span>
            <a href="#">View all</a>
          </div>
          {artifacts.length === 0 && <p className="empty-note">No artifacts yet</p>}
          <div className="artifact-grid">
            {artifacts.map((artifact) => (
              <a key={artifact.id} className="artifact-card" href="#">
                <div className="artifact-preview" aria-hidden="true">
                  <div className="artifact-page" />
                </div>
                <div className="artifact-title">{artifact.title}</div>
                <div className="artifact-meta">{artifact.updated}</div>
              </a>
            ))}
          </div>
        </section>
      </div>
    </main>
  );
}
