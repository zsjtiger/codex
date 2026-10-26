import { useEffect, useState } from "react";
import { skills as skillApi } from "../api/resources";
import type { Skill } from "../api/types";

const sections = ["Skills", "Connectors", "Plugins"] as const;

interface CustomizePageProps {
  workspaceId: string;
  isAdmin: boolean;
  onBack: () => void;
  onError: (message: string) => void;
}

/**
 * Skills are instructions added to new chats in the workspace. Connectors and plugins
 * have no gateway API yet, so those tabs say so instead of showing a fake list.
 */
export function CustomizePage({ workspaceId, isAdmin, onBack, onError }: CustomizePageProps) {
  const [section, setSection] = useState<(typeof sections)[number]>("Skills");
  const [items, setItems] = useState<Skill[]>([]);
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [instructions, setInstructions] = useState("");

  const reload = () =>
    skillApi
      .list(workspaceId)
      .then(setItems)
      .catch((err) => onError(err instanceof Error ? err.message : "Could not load skills."));

  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceId]);

  const create = async () => {
    try {
      await skillApi.create(workspaceId, { name: name.trim(), description: description.trim(), instructions: instructions.trim() });
      setCreating(false);
      setName("");
      setDescription("");
      setInstructions("");
      await reload();
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not create the skill.");
    }
  };

  const toggle = async (skill: Skill) => {
    try {
      await skillApi.update(skill.id, { enabled: !skill.enabled });
      await reload();
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not update the skill.");
    }
  };

  const remove = async (skill: Skill) => {
    try {
      await skillApi.remove(skill.id);
      await reload();
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not delete the skill.");
    }
  };

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
              <button key={name} role="tab" aria-selected={section === name} className={`tab ${section === name ? "tab-active" : ""}`} onClick={() => setSection(name)}>
                {name}
              </button>
            ))}
          </div>
          {section === "Skills" && isAdmin && !creating && (
            <button className="add-button" onClick={() => setCreating(true)}>
              New skill
            </button>
          )}
        </div>

        {section !== "Skills" ? (
          <p className="empty-state">{section} are not available yet.</p>
        ) : (
          <>
            {!isAdmin && <p className="muted small">Only workspace admins can add or change skills.</p>}
            {creating && (
              <form
                className="form-card"
                onSubmit={(event) => {
                  event.preventDefault();
                  void create();
                }}
              >
                <label className="field">
                  <span>Name</span>
                  <input value={name} onChange={(e) => setName(e.target.value)} maxLength={100} required />
                </label>
                <label className="field">
                  <span>Description</span>
                  <input value={description} onChange={(e) => setDescription(e.target.value)} maxLength={500} />
                </label>
                <label className="field">
                  <span>Instructions</span>
                  <textarea value={instructions} onChange={(e) => setInstructions(e.target.value)} rows={6} maxLength={20000} required />
                </label>
                <div className="form-actions">
                  <button type="button" onClick={() => setCreating(false)}>
                    Cancel
                  </button>
                  <button className="primary" type="submit" disabled={!name.trim() || !instructions.trim()}>
                    Add skill
                  </button>
                </div>
              </form>
            )}
            {items.length === 0 && !creating && <p className="empty-state">No skills added yet.</p>}
            <div className="list">
              {items.map((skill) => (
                <div key={skill.id} className="list-row">
                  <div className="list-main">
                    <div className="list-title">{skill.name}</div>
                    <div className="list-sub">{skill.description || skill.instructions.slice(0, 120)}</div>
                  </div>
                  <div className="list-actions">
                    <span className={`status ${skill.enabled ? "status-on" : ""}`}>{skill.enabled ? "On" : "Off"}</span>
                    {isAdmin && <button onClick={() => void toggle(skill)}>{skill.enabled ? "Turn off" : "Turn on"}</button>}
                    {isAdmin && <button onClick={() => void remove(skill)}>Delete</button>}
                  </div>
                </div>
              ))}
            </div>
          </>
        )}
      </div>
    </main>
  );
}
