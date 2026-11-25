import { useState } from "react";
import { threads as threadApi } from "../api/resources";
import type { CommandResult } from "../api/types";
import { tx } from "../i18n";

/** How many recent runs stay on screen. */
const KEEP = 5;

interface Run {
  command: string;
  result: CommandResult;
}

/**
 * Runs a command in the code chat's checkout and shows what it printed. The gateway runs it under a sandbox
 * that can write only inside the checkout and cannot reach the network.
 */
export function RunCommand({ threadId, onError }: { threadId: string; onError: (message: string) => void }) {
  const [command, setCommand] = useState("");
  const [busy, setBusy] = useState(false);
  const [runs, setRuns] = useState<Run[]>([]);

  const run = async () => {
    const text = command.trim();
    if (!text || busy) return;
    setBusy(true);
    try {
      const result = await threadApi.run(threadId, text);
      setRuns((prev) => [{ command: text, result }, ...prev].slice(0, KEEP));
      setCommand("");
    } catch (err) {
      onError(err instanceof Error ? err.message : tx("The command could not be run."));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="run-command">
      <form
        className="run-form"
        onSubmit={(event) => {
          event.preventDefault();
          void run();
        }}
      >
        <input
          value={command}
          onChange={(event) => setCommand(event.target.value)}
          placeholder={tx("Run a command in the checkout, such as npm test")}
          aria-label={tx("Command to run")}
          disabled={busy}
        />
        <button className="primary" disabled={busy || !command.trim()}>
          {busy ? tx("Running…") : tx("Run")}
        </button>
      </form>
      {runs.map((entry, index) => (
        <div key={index} className="run-result">
          <div className="run-head">
            <code>$ {entry.command}</code>
            <span className={entry.result.exit_code === 0 ? "run-exit run-ok" : "run-exit run-failed"}>
              {tx("exit")} {entry.result.exit_code}
            </span>
          </div>
          {entry.result.stdout && <pre className="run-output">{entry.result.stdout}</pre>}
          {entry.result.stderr && <pre className="run-output run-error">{entry.result.stderr}</pre>}
        </div>
      ))}
    </div>
  );
}
