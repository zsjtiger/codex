import { useEffect, useState } from "react";
import { usage } from "../api/resources";
import type { UsageReport } from "../api/types";

const RANGES = [7, 30, 90];

const format = (n: number) => n.toLocaleString();

/** Shows the signed-in user's token usage for a chosen period, with a bar per day. */
export function UsageDialog({ onClose }: { onClose: () => void }) {
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
      .catch((err) => current && setError(err instanceof Error ? err.message : "Could not load usage."));
    return () => {
      current = false;
    };
  }, [days]);

  const dayTotal = (day: UsageReport["daily"][number]) => day.input_tokens + day.output_tokens;
  const peak = Math.max(1, ...(report?.daily.map(dayTotal) ?? [1]));

  return (
    <div className="modal-backdrop" onMouseDown={onClose}>
      <div
        className="usage-dialog"
        role="dialog"
        aria-modal="true"
        aria-label="Usage"
        onMouseDown={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key === "Escape") onClose();
        }}
      >
        <header className="usage-header">
          <h2>Usage</h2>
          <button className="banner-close" onClick={onClose} aria-label="Close">
            ×
          </button>
        </header>

        <div className="usage-ranges" role="group" aria-label="Period">
          {RANGES.map((range) => (
            <button key={range} aria-pressed={days === range} className={days === range ? "range-active" : ""} onClick={() => setDays(range)}>
              {range} days
            </button>
          ))}
        </div>

        {error && <p className="banner error">{error}</p>}
        {!error && !report && <p className="muted">Loading…</p>}

        {report && (
          <>
            <div className="usage-totals">
              <div>
                <span className="muted">Input tokens</span>
                <strong>{format(report.totals.input_tokens)}</strong>
              </div>
              <div>
                <span className="muted">Output tokens</span>
                <strong>{format(report.totals.output_tokens)}</strong>
              </div>
            </div>

            {report.daily.length === 0 ? (
              <p className="muted">No usage in this period.</p>
            ) : (
              <ol className="usage-days" aria-label="Daily usage">
                {report.daily.map((day) => {
                  const total = dayTotal(day);
                  const label = new Date(day.day).toLocaleDateString();
                  return (
                    <li key={day.day} title={`${label}: ${format(total)} tokens`}>
                      <span className="usage-day-label">{label}</span>
                      <span className="usage-bar-track">
                        <span className="usage-bar" style={{ width: `${(total / peak) * 100}%` }} />
                      </span>
                      <span className="usage-day-total">{format(total)}</span>
                    </li>
                  );
                })}
              </ol>
            )}
          </>
        )}
      </div>
    </div>
  );
}
