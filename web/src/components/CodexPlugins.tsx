import { useState } from "react";
import { environments } from "../api/resources";
import type { CodexPlugins as CodexPluginsState } from "../api/types";
import { tx } from "../i18n";

/**
 * The plugins codex has installed in an environment, grouped by where they came from. Read-only: plugins a
 * person installs in Baize are managed above; this shows what codex has installed and what it could not load.
 */
export function CodexPlugins({ environmentId, onError }: { environmentId: string; onError: (message: string) => void }) {
  const [found, setFound] = useState<CodexPluginsState | null>(null);
  const [loading, setLoading] = useState(false);

  const load = async () => {
    setLoading(true);
    try {
      setFound(await environments.codexPlugins(environmentId));
    } catch (err) {
      onError(err instanceof Error ? err.message : tx("The plugins could not be read."));
    } finally {
      setLoading(false);
    }
  };

  const markets = found ? [...new Set(found.plugins.map((plugin) => plugin.marketplace))] : [];

  return (
    <section className="codex-skills">
      <div className="live-check">
        <h2 className="codex-skills-title">{tx("Plugins codex has installed")}</h2>
        <button type="button" className="link-button" disabled={loading} onClick={() => void load()}>
          {loading ? tx("Reading…") : found ? tx("Read again") : tx("Read from codex")}
        </button>
      </div>
      {found && found.plugins.length === 0 && found.errors.length === 0 && (
        <p className="muted small">{tx("Codex has no plugins installed in this environment.")}</p>
      )}
      {markets.map((market) => (
        <div key={market} className="codex-plugin-group">
          <div className="muted small">{market}</div>
          <ul className="codex-skill-list">
            {found!.plugins
              .filter((plugin) => plugin.marketplace === market)
              .map((plugin) => (
                <li key={plugin.id} className="codex-skill">
                  <div className="pl-name">
                    {plugin.name}
                    {plugin.version && <span className="muted small"> · {plugin.version}</span>}
                    {!plugin.enabled && <span className="muted small"> · {tx("Off")}</span>}
                  </div>
                  <code className="muted small">{plugin.id}</code>
                </li>
              ))}
          </ul>
        </div>
      ))}
      {found && found.errors.length > 0 && (
        <ul className="codex-skill-errors">
          {found.errors.map((error) => (
            <li key={error.marketplace_path} className="muted small">
              <code>{error.marketplace_path}</code> — {error.message}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
