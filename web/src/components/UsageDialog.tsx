import { useEffect, useState } from "react";
import { usage } from "../api/resources";
import type { UsageReport } from "../api/types";

const RANGES = [7, 30, 90];

const format = (n: number) => n.toLocaleString();

/** Shows the signed-in user's token usage for a chosen period. Only used amounts are shown. */
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

  return (
    <div className="modal-backdrop" onMouseDown={onClose}>
      <div
        className="usage-dialog"
        role="dialog"
        aria-modal="true"
        aria-label="Your usage"
        onMouseDown={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key === "Escape") onClose();
        }}
      >
        <header className="usage-header">
          <h2>Your usage</h2>
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
            <div className="usage-used">
              <div className="usage-row">
                <span>Input</span>
                <strong>{format(report.totals.input_tokens)} tokens</strong>
              </div>
              <div className="usage-row">
                <span>Output</span>
                <strong>{format(report.totals.output_tokens)} tokens</strong>
              </div>
            </div>

            {report.daily.length === 0 ? (
              <p className="muted">No usage in this period.</p>
            ) : (
              <ul className="usage-days" aria-label="Daily usage">
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
      </div>
    </div>
  );
}
