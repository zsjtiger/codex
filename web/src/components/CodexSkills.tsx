import { useEffect, useState } from "react";
import { environments } from "../api/resources";
import type { CodexSkills as CodexSkillsState } from "../api/types";
import { tx } from "../i18n";
import { subscribeEnvironment } from "../state/environmentEvents";

/** The labels codex uses for where a skill came from. */
const SCOPE_LABEL: Record<string, string> = {
  user: "User",
  repo: "Repository",
  system: "System",
  admin: "Admin",
};

/**
 * The skills codex finds in an environment, as codex reads them now. Read-only: the skills a person
 * adds in Baize are managed above; this shows what codex actually loaded, and what it could not read.
 */
export function CodexSkills({ environmentId, onError }: { environmentId: string; onError: (message: string) => void }) {
  const [found, setFound] = useState<CodexSkillsState | null>(null);
  const [loading, setLoading] = useState(false);

  // When codex's skills change while the list is shown, the list is read again.
  useEffect(() => {
    if (!found) return;
    return subscribeEnvironment(environmentId, (method) => {
      if (method === "skills/changed") void load();
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [environmentId, found !== null]);

  const load = async () => {
    setLoading(true);
    try {
      setFound(await environments.codexSkills(environmentId));
    } catch (err) {
      onError(err instanceof Error ? err.message : tx("The skills could not be read."));
    } finally {
      setLoading(false);
    }
  };

  return (
    <section className="codex-skills">
      <div className="live-check">
        <h2 className="codex-skills-title">{tx("Skills codex finds")}</h2>
        <button type="button" className="link-button" disabled={loading} onClick={() => void load()}>
          {loading ? tx("Reading…") : found ? tx("Read again") : tx("Read from codex")}
        </button>
      </div>
      {found && found.skills.length === 0 && found.errors.length === 0 && (
        <p className="muted small">{tx("Codex finds no skills in this environment.")}</p>
      )}
      {found && found.skills.length > 0 && (
        <ul className="codex-skill-list">
          {found.skills.map((skill) => (
            <li key={skill.path} className="codex-skill">
              <div className="pl-name">
                {skill.name}
                <span className="muted small"> · {tx(SCOPE_LABEL[skill.scope] ?? skill.scope)}</span>
                {!skill.enabled && <span className="muted small"> · {tx("Off")}</span>}
              </div>
              {skill.description && <div className="muted small">{skill.description}</div>}
              <code className="muted small">{skill.path}</code>
            </li>
          ))}
        </ul>
      )}
      {found && found.errors.length > 0 && (
        <ul className="codex-skill-errors">
          {found.errors.map((error) => (
            <li key={error.path} className="muted small">
              <code>{error.path}</code> — {error.message}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
