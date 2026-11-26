import { useState } from "react";
import { threads as threadApi } from "../api/resources";
import type { BackgroundTerminal } from "../api/types";
import { tx } from "../i18n";

/**
 * The background terminals codex started for this code chat, such as a dev server it left running. Each can be
 * stopped; codex can also clean up the ones it has finished with.
 */
export function BackgroundTerminals({ threadId, onError }: { threadId: string; onError: (message: string) => void }) {
  const [terminals, setTerminals] = useState<BackgroundTerminal[] | null>(null);
  const [loading, setLoading] = useState(false);

  const read = async () => {
    setLoading(true);
    try {
      const page = await threadApi.backgroundTerminals(threadId);
      setTerminals(page.terminals);
    } catch (err) {
      onError(err instanceof Error ? err.message : tx("The terminals could not be read."));
    } finally {
      setLoading(false);
    }
  };

  const stop = async (processId: string) => {
    try {
      await threadApi.stopTerminal(threadId, processId);
      await read();
    } catch (err) {
      onError(err instanceof Error ? err.message : tx("The terminal could not be stopped."));
    }
  };

  const clean = async () => {
    try {
      await threadApi.cleanTerminals(threadId);
      await read();
    } catch (err) {
      onError(err instanceof Error ? err.message : tx("The terminals could not be cleaned up."));
    }
  };

  return (
    <section className="codex-skills">
      <div className="live-check">
        <h2 className="codex-skills-title">{tx("Background terminals")}</h2>
        <button type="button" className="link-button" disabled={loading} onClick={() => void read()}>
          {loading ? tx("Reading…") : terminals ? tx("Read again") : tx("Read from codex")}
        </button>
        {terminals && terminals.length > 0 && (
          <button type="button" className="link-button" onClick={() => void clean()}>
            {tx("Clean up finished")}
          </button>
        )}
      </div>
      {terminals && terminals.length === 0 && <p className="muted small">{tx("No terminals are running for this chat.")}</p>}
      {terminals && terminals.length > 0 && (
        <ul className="codex-skill-list">
          {terminals.map((terminal) => (
            <li key={terminal.process_id} className="codex-skill">
              <div className="checkout-line">
                <code className="codex-terminal-command">{terminal.command}</code>
                <button type="button" className="link-button" onClick={() => void stop(terminal.process_id)}>
                  {tx("Stop")}
                </button>
              </div>
              <div className="muted small">
                {terminal.cwd}
                {terminal.os_pid !== null && ` · pid ${terminal.os_pid}`}
                {terminal.cpu_percent !== null && ` · ${terminal.cpu_percent.toFixed(1)}% CPU`}
                {terminal.rss_kb !== null && ` · ${Math.round(terminal.rss_kb / 1024)} MB`}
              </div>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
