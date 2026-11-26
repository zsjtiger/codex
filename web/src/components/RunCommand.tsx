import { useState } from "react";
import { threads as threadApi } from "../api/resources";
import type { CommandResult } from "../api/types";
import { tx } from "../i18n";
import { type CommandOutput, forgetCommandOutput } from "../state/commandOutput";

/** How many recent runs stay on screen. */
const KEEP = 5;

interface Run {
  command: string;
  processId: string;
  /** Set once the run has ended; until then the output is what has streamed so far. */
  result: CommandResult | null;
}

/**
 * Runs a command in the code chat's checkout. Output shows as codex prints it, and a run can be stopped while
 * it goes. The gateway runs it under a sandbox that can write only inside the checkout and cannot reach the network.
 */
export function RunCommand({
  threadId,
  output,
  onError,
}: {
  threadId: string;
  output: Record<string, CommandOutput>;
  onError: (message: string) => void;
}) {
  const [command, setCommand] = useState("");
  const [runs, setRuns] = useState<Run[]>([]);

  const finish = (processId: string, result: CommandResult) => {
    forgetCommandOutput(processId);
    setRuns((prev) => prev.map((run) => (run.processId === processId ? { ...run, result } : run)));
  };

  const run = async () => {
    const text = command.trim();
    if (!text) return;
    // The browser names the run, so it can stop it while it goes.
    const processId = crypto.randomUUID();
    setRuns((prev) => [{ command: text, processId, result: null }, ...prev].slice(0, KEEP));
    setCommand("");
    try {
      const result = await threadApi.run(threadId, text, processId);
      finish(processId, result);
    } catch (err) {
      forgetCommandOutput(processId);
      setRuns((prev) => prev.filter((entry) => entry.processId !== processId));
      onError(err instanceof Error ? err.message : tx("The command could not be run."));
    }
  };

  const stop = async (processId: string) => {
    try {
      await threadApi.stopRun(threadId, processId);
    } catch (err) {
      onError(err instanceof Error ? err.message : tx("The command could not be stopped."));
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
        />
        <button className="primary" disabled={!command.trim()}>
          {tx("Run")}
        </button>
      </form>
      {runs.map((entry) => {
        const live = output[entry.processId];
        const stdout = entry.result ? entry.result.stdout : live?.stdout ?? "";
        const stderr = entry.result ? entry.result.stderr : live?.stderr ?? "";
        const running = entry.result === null;
        return (
          <div key={entry.processId} className="run-result">
            <div className="run-head">
              <code>$ {entry.command}</code>
              {running ? (
                <button type="button" className="link-button" onClick={() => void stop(entry.processId)}>
                  {tx("Stop")}
                </button>
              ) : (
                <span className={entry.result!.exit_code === 0 ? "run-exit run-ok" : "run-exit run-failed"}>
                  {tx("exit")} {entry.result!.exit_code}
                </span>
              )}
            </div>
            {running && <div className="muted small">{tx("Running…")}</div>}
            {stdout && <pre className="run-output">{stdout}</pre>}
            {stderr && <pre className="run-output run-error">{stderr}</pre>}
          </div>
        );
      })}
    </div>
  );
}
