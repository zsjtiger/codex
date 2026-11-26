import { useEffect, useState } from "react";
import { environments, usage } from "../api/resources";
import type { Environment, RateLimits, RateWindow, UsageReport } from "../api/types";
import { tx } from "../i18n";
import { subscribeEnvironment } from "../state/environmentEvents";

const RANGES = [7, 30, 90];

const format = (n: number) => n.toLocaleString();

/** A window's length in words: "5 hours", "7 days". */
function windowLabel(minutes: number | null): string {
  if (minutes === null) return tx("Limit");
  if (minutes % 1440 === 0) return `${minutes / 1440} ${tx("days")}`;
  if (minutes % 60 === 0) return `${minutes / 60} ${tx("hours")}`;
  return `${minutes} ${tx("minutes")}`;
}

function WindowRow({ window }: { window: RateWindow }) {
  const used = Math.min(100, Math.max(0, window.used_percent));
  return (
    <div className="limit-window">
      <div className="usage-row">
        <span>{windowLabel(window.window_minutes)}</span>
        <strong>{used}% {tx("used")}</strong>
      </div>
      <div className="limit-bar" role="meter" aria-valuenow={used} aria-valuemin={0} aria-valuemax={100}>
        <span style={{ width: `${used}%` }} />
      </div>
      <div className="muted small">
        {window.resets_at !== null ? `${tx("Resets")} ${new Date(window.resets_at * 1000).toLocaleString()}` : tx("No reset time")}
      </div>
    </div>
  );
}

/** One environment's codex account: its plan, its windows, and its credits. */
function EnvironmentLimits({ environment }: { environment: Environment }) {
  const [limits, setLimits] = useState<RateLimits | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let current = true;
    const read = () =>
      environments
        .rateLimits(environment.id)
        .then((next) => current && setLimits(next))
        .catch((err) => current && setError(err instanceof Error ? err.message : tx("Could not read the limits.")));
    void read();
    // codex announces a change to the account's limits; the dialog reads them again.
    const stop = subscribeEnvironment(environment.id, (method) => {
      if (method === "account/rateLimits/updated") void read();
    });
    return () => {
      current = false;
      stop();
    };
  }, [environment.id]);
  return (
    <section className="limit-environment">
      <h3>{environment.name}</h3>
      {error && <p className="muted small">{error}</p>}
      {!error && !limits && <p className="muted">{tx("Loading…")}</p>}
      {limits && (
        <>
          {limits.plan_type && <div className="muted small">{tx("Plan")}: {limits.plan_type}</div>}
          {limits.reached_type && <p className="banner error">{tx("A limit has been reached.")}</p>}
          {limits.primary && <WindowRow window={limits.primary} />}
          {limits.secondary && <WindowRow window={limits.secondary} />}
          {limits.credits && (
            <div className="usage-row">
              <span>{tx("Credits")}</span>
              <strong>
                {limits.credits.unlimited ? tx("Unlimited") : limits.credits.has_credits ? limits.credits.balance ?? tx("Available") : tx("None")}
              </strong>
            </div>
          )}
        </>
      )}
    </section>
  );
}

/** Shows the signed-in user's token usage for a chosen period, and the codex limits of the workspace's environments. */
export function UsageDialog({ workspaceId, onClose }: { workspaceId: string | null; onClose: () => void }) {
  const [envs, setEnvs] = useState<Environment[]>([]);
  useEffect(() => {
    if (!workspaceId) return;
    let current = true;
    environments
      .list(workspaceId)
      .then((next) => current && setEnvs(next))
      .catch(() => current && setEnvs([]));
    return () => {
      current = false;
    };
  }, [workspaceId]);
  const [days, setDays] = useState(30);
  const [report, setReport] = useState<UsageReport | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let current = true;
    setReport(null);
    setError(null);
    usage
      .mine(days)
      .then((next) => current && setReport(next))
      .catch((err) => current && setError(err instanceof Error ? err.message : tx("Could not load usage.")));
    return () => {
      current = false;
    };
  }, [days]);

  return (
    <div className="modal-backdrop" onMouseDown={onClose}>
      <div
        className="usage-dialog"
        role="dialog"
        aria-modal="true"
        aria-label={tx("Your usage")}
        onMouseDown={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key === "Escape") onClose();
        }}
      >
        <header className="usage-header">
          <h2>{tx("Your usage")}</h2>
          <button className="banner-close" onClick={onClose} aria-label={tx("Close")}>
            ×
          </button>
        </header>

        <div className="usage-ranges" role="group" aria-label={tx("Period")}>
          {RANGES.map((range) => (
            <button key={range} aria-pressed={days === range} className={days === range ? "range-active" : ""} onClick={() => setDays(range)}>
              {range} days
            </button>
          ))}
        </div>

        {error && <p className="banner error">{error}</p>}
        {!error && !report && <p className="muted">{tx("Loading…")}</p>}

        {report && (
          <>
            <div className="usage-used">
              <div className="usage-row">
                <span>{tx("Input")}</span>
                <strong>{format(report.totals.input_tokens)} tokens</strong>
              </div>
              <div className="usage-row">
                <span>{tx("Output")}</span>
                <strong>{format(report.totals.output_tokens)} tokens</strong>
              </div>
            </div>

            {report.daily.length === 0 ? (
              <p className="muted">{tx("No usage in this period.")}</p>
            ) : (
              <ul className="usage-days" aria-label={tx("Daily usage")}>
                {report.daily.map((day) => (
                  <li key={day.day}>
                    <span className="muted">{new Date(day.day).toLocaleDateString()}</span>
                    <span>{format(day.input_tokens + day.output_tokens)} tokens</span>
                  </li>
                ))}
              </ul>
            )}
          </>
        )}

        {envs.length > 0 && (
          <div className="limit-list">
            <h2 className="limit-heading">{tx("Codex limits")}</h2>
            <p className="muted small">{tx("Each environment has its own codex account.")}</p>
            {envs.map((environment) => (
              <EnvironmentLimits key={environment.id} environment={environment} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
