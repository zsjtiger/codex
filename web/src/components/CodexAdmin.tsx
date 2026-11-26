import { useEffect, useState } from "react";
import { environments } from "../api/resources";
import type { CodexSkill, Environment, ExperimentalFeature } from "../api/types";
import { tx } from "../i18n";

/** Errors are shown in the panel; the message comes from the gateway when it has one. */
function messageOf(err: unknown, fallback: string): string {
  return err instanceof Error ? err.message : tx(fallback);
}

/** Sets one codex config value. The gateway refuses keys that look like secrets, and records the key and strategy only. */
function ConfigEditor({ environmentId, onDone }: { environmentId: string; onDone: (message: string) => void }) {
  const [keyPath, setKeyPath] = useState("");
  const [value, setValue] = useState("");
  const [strategy, setStrategy] = useState<"replace" | "upsert">("replace");
  const [expectedVersion, setExpectedVersion] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const save = async () => {
    let parsed: unknown;
    try {
      parsed = JSON.parse(value);
    } catch {
      setError(tx("The value must be JSON, for example \"gpt-5\", true, or 4."));
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const result = (await environments.writeConfigValue(environmentId, {
        key_path: keyPath.trim(),
        value: parsed,
        merge_strategy: strategy,
        expected_version: expectedVersion.trim() || null,
      })) as { status?: string; version?: string };
      onDone(`${tx("Saved.")} ${result.status ?? ""} ${result.version ? `· ${result.version}` : ""}`.trim());
      setValue("");
    } catch (err) {
      setError(messageOf(err, "Could not save the config value."));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="settings-row settings-row-stack">
      <div className="detail-label">{tx("Set a config value")}</div>
      <div className="muted small">{tx("Secret keys (anything named key, token, secret, password or credential) are refused. Values are not written to the audit log.")}</div>
      <input aria-label={tx("Key path")} placeholder="features.goals" value={keyPath} onChange={(e) => setKeyPath(e.target.value)} />
      <textarea aria-label={tx("Value as JSON")} placeholder={'"gpt-5"'} rows={2} value={value} onChange={(e) => setValue(e.target.value)} />
      <div className="row-gap">
        <select aria-label={tx("Merge strategy")} value={strategy} onChange={(e) => setStrategy(e.target.value as "replace" | "upsert")}>
          <option value="replace">{tx("Replace")}</option>
          <option value="upsert">{tx("Merge into")}</option>
        </select>
        <input aria-label={tx("Expected version")} placeholder={tx("Version (optional)")} value={expectedVersion} onChange={(e) => setExpectedVersion(e.target.value)} />
        <button type="button" className="link-button" disabled={busy || !keyPath.trim() || !value.trim()} onClick={() => void save()}>
          {busy ? tx("Saving…") : tx("Save")}
        </button>
      </div>
      {error && <div className="error small">{error}</div>}
    </div>
  );
}

/** Turns codex's experimental features on and off for the environment. */
function FeatureList({ environmentId, onError, onDone }: { environmentId: string; onError: (message: string) => void; onDone: (message: string) => void }) {
  const [features, setFeatures] = useState<ExperimentalFeature[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const load = () =>
    environments
      .experimentalFeatures(environmentId)
      .then((next) => setFeatures(next.data))
      .catch((err) => onError(messageOf(err, "Could not list the experimental features.")));

  useEffect(() => {
    setFeatures(null);
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [environmentId]);

  const toggle = async (feature: ExperimentalFeature) => {
    setBusy(feature.name);
    try {
      await environments.setExperimentalFeatures(environmentId, { [feature.name]: !feature.enabled });
      await load();
      onDone(`${feature.displayName ?? feature.name}: ${!feature.enabled ? tx("on") : tx("off")}`);
    } catch (err) {
      onError(messageOf(err, "Could not change the experimental feature."));
    } finally {
      setBusy(null);
    }
  };

  if (!features) return <div className="muted small">{tx("Loading…")}</div>;
  if (features.length === 0) return <div className="muted small">{tx("Codex has no experimental features.")}</div>;
  return (
    <>
      {features.map((feature) => (
        <div className="settings-row" key={feature.name}>
          <div>
            <div className="detail-label">{feature.displayName ?? feature.name}</div>
            <div className="muted small">
              {feature.description ?? feature.stage}
              {feature.defaultEnabled ? ` · ${tx("on by default")}` : ""}
            </div>
          </div>
          <input
            type="checkbox"
            className="switch"
            checked={feature.enabled}
            disabled={busy === feature.name}
            onChange={() => void toggle(feature)}
            aria-label={feature.displayName ?? feature.name}
          />
        </div>
      ))}
    </>
  );
}

/** Switches codex's skills on and off. The list is codex's own; this only changes which ones are enabled. */
function SkillSwitches({ environmentId, onError, onDone }: { environmentId: string; onError: (message: string) => void; onDone: (message: string) => void }) {
  const [skills, setSkills] = useState<CodexSkill[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const load = () =>
    environments
      .codexSkills(environmentId)
      .then((next) => setSkills(next.skills))
      .catch((err) => onError(messageOf(err, "Could not list codex's skills.")));

  useEffect(() => {
    setSkills(null);
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [environmentId]);

  const toggle = async (skill: CodexSkill) => {
    setBusy(skill.name);
    try {
      await environments.setSkillEnabled(environmentId, skill.name, !skill.enabled);
      await load();
      onDone(`${skill.name}: ${!skill.enabled ? tx("on") : tx("off")}`);
    } catch (err) {
      onError(messageOf(err, "Could not switch the skill."));
    } finally {
      setBusy(null);
    }
  };

  if (!skills) return <div className="muted small">{tx("Loading…")}</div>;
  if (skills.length === 0) return <div className="muted small">{tx("Codex has no skills here.")}</div>;
  return (
    <>
      {skills.map((skill) => (
        <div className="settings-row" key={`${skill.scope}:${skill.path}`}>
          <div>
            <div className="detail-label">{skill.name}</div>
            <div className="muted small">{skill.description}</div>
          </div>
          <input
            type="checkbox"
            className="switch"
            checked={skill.enabled}
            disabled={busy === skill.name}
            onChange={() => void toggle(skill)}
            aria-label={skill.name}
          />
        </div>
      ))}
    </>
  );
}

/** Read-only views of codex's config requirements and its diagnostics, for an admin to check. */
function JsonView({ load, label }: { load: () => Promise<unknown>; label: string }) {
  const [value, setValue] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const show = () =>
    load()
      .then((next) => setValue(JSON.stringify(next, null, 2)))
      .catch((err) => setError(messageOf(err, "Could not read it.")));
  return (
    <div className="settings-row settings-row-stack">
      <div className="settings-row">
        <div className="detail-label">{label}</div>
        <button type="button" className="link-button" onClick={() => void show()}>
          {tx("Show")}
        </button>
      </div>
      {error && <div className="error small">{error}</div>}
      {value && <pre className="small code-block">{value}</pre>}
    </div>
  );
}

/**
 * Codex's configuration and runtime switches for one environment. Only workspace admins see this section, and
 * every change is recorded in the audit log.
 */
export function CodexAdminSection({ workspaceId, onError }: { workspaceId: string; onError: (message: string) => void }) {
  const [envs, setEnvs] = useState<Environment[] | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    let current = true;
    environments
      .list(workspaceId)
      .then((next) => {
        if (!current) return;
        setEnvs(next);
        setSelected((prev) => prev ?? next[0]?.id ?? null);
      })
      .catch((err) => current && onError(messageOf(err, "Could not list the environments.")));
    return () => {
      current = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceId]);

  if (!envs) return <div className="muted small">{tx("Loading…")}</div>;
  if (envs.length === 0) return <div className="muted small">{tx("This workspace has no environments yet.")}</div>;

  return (
    <>
      <h3 className="settings-title">{tx("Codex")}</h3>
      <div className="settings-row">
        <div>
          <div className="detail-label">{tx("Environment")}</div>
          <div className="muted small">{tx("Changes apply to codex in this environment, for every chat in it.")}</div>
        </div>
        <select aria-label={tx("Environment")} value={selected ?? ""} onChange={(e) => setSelected(e.target.value)}>
          {envs.map((env) => (
            <option key={env.id} value={env.id}>
              {env.name}
            </option>
          ))}
        </select>
      </div>
      {notice && <div className="muted small">{notice}</div>}
      {selected && (
        <>
          <ConfigEditor environmentId={selected} onDone={setNotice} />
          <div className="settings-row">
            <div>
              <div className="detail-label">{tx("MCP servers")}</div>
              <div className="muted small">{tx("Reload the servers after their config changes.")}</div>
            </div>
            <button
              type="button"
              className="link-button"
              onClick={() =>
                void environments
                  .reloadMcpServers(selected)
                  .then(() => setNotice(tx("MCP servers reloaded.")))
                  .catch((err) => onError(messageOf(err, "Could not reload the MCP servers.")))
              }
            >
              {tx("Reload")}
            </button>
          </div>
          <h4 className="settings-subtitle">{tx("Experimental features")}</h4>
          <FeatureList environmentId={selected} onError={onError} onDone={setNotice} />
          <h4 className="settings-subtitle">{tx("Skills")}</h4>
          <SkillSwitches environmentId={selected} onError={onError} onDone={setNotice} />
          <h4 className="settings-subtitle">{tx("Diagnostics")}</h4>
          <JsonView label={tx("Config requirements")} load={() => environments.configRequirements(selected)} />
          <JsonView label={tx("Codex diagnostics")} load={() => environments.diagnostics(selected)} />
        </>
      )}
    </>
  );
}
