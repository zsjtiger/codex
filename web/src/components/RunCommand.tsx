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
  /** The run takes text on its input while it goes. */
  interactive: boolean;
  /** The run is in a terminal, so its size can be set. */
  tty: boolean;
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
  const [interactive, setInteractive] = useState(false);
  const [tty, setTty] = useState(false);
  const [runs, setRuns] = useState<Run[]>([]);
  // Text being typed into each running interactive command, and the terminal size asked for each.
  const [typed, setTyped] = useState<Record<string, string>>({});
  const [sizes, setSizes] = useState<Record<string, { rows: number; cols: number }>>({});

  const finish = (processId: string, result: CommandResult) => {
    forgetCommandOutput(processId);
    setRuns((prev) => prev.map((run) => (run.processId === processId ? { ...run, result } : run)));
  };

  const run = async () => {
    const text = command.trim();
    if (!text) return;
    // The browser names the run, so it can stop it while it goes.
    const processId = crypto.randomUUID();
    // A terminal takes input too, so choosing one implies input.
    const takesInput = interactive || tty;
    setRuns((prev) => [{ command: text, processId, interactive: takesInput, tty, result: null }, ...prev].slice(0, KEEP));
    setCommand("");
    try {
      const result = await threadApi.run(threadId, text, processId, { interactive: takesInput, tty });
      finish(processId, result);
    } catch (err) {
      forgetCommandOutput(processId);
      setRuns((prev) => prev.filter((entry) => entry.processId !== processId));
      onError(err instanceof Error ? err.message : tx("The command could not be run."));
    }
  };

  const send = async (processId: string, body: { text?: string; close_stdin?: boolean }) => {
    try {
      await threadApi.sendInput(threadId, processId, body);
      if (body.text !== undefined) setTyped((prev) => ({ ...prev, [processId]: "" }));
    } catch (err) {
      onError(err instanceof Error ? err.message : tx("The input could not be sent."));
    }
  };

  const resize = async (processId: string) => {
    const size = sizes[processId] ?? { rows: 24, cols: 80 };
    try {
      await threadApi.resizeTerminal(threadId, processId, size.rows, size.cols);
    } catch (err) {
      onError(err instanceof Error ? err.message : tx("The terminal could not be resized."));
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
      <div className="run-options">
        <label className="muted small">
          <input type="checkbox" checked={interactive || tty} disabled={tty} onChange={(event) => setInteractive(event.target.checked)} /> {tx("Takes input")}
        </label>
        <label className="muted small">
          <input type="checkbox" checked={tty} onChange={(event) => setTty(event.target.checked)} /> {tx("In a terminal")}
        </label>
      </div>
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
            {running && entry.interactive && (
              <form
                className="run-form"
                onSubmit={(event) => {
                  event.preventDefault();
                  const text = typed[entry.processId] ?? "";
                  if (text) void send(entry.processId, { text: `${text}\n` });
                }}
              >
                <input
                  value={typed[entry.processId] ?? ""}
                  onChange={(event) => setTyped((prev) => ({ ...prev, [entry.processId]: event.target.value }))}
                  placeholder={tx("Type into the command, then Enter")}
                  aria-label={tx("Input to the command")}
                />
                <button className="primary" disabled={!(typed[entry.processId] ?? "")}>
                  {tx("Send")}
                </button>
                <button type="button" className="link-button" onClick={() => void send(entry.processId, { close_stdin: true })}>
                  {tx("Close input")}
                </button>
              </form>
            )}
            {running && entry.tty && (
              <div className="run-form">
                <input
                  type="number"
                  min={1}
                  max={1000}
                  value={(sizes[entry.processId] ?? { rows: 24, cols: 80 }).rows}
                  onChange={(event) => setSizes((prev) => ({ ...prev, [entry.processId]: { ...(prev[entry.processId] ?? { rows: 24, cols: 80 }), rows: Number(event.target.value) } }))}
                  aria-label={tx("Rows")}
                />
                <input
                  type="number"
                  min={1}
                  max={1000}
                  value={(sizes[entry.processId] ?? { rows: 24, cols: 80 }).cols}
                  onChange={(event) => setSizes((prev) => ({ ...prev, [entry.processId]: { ...(prev[entry.processId] ?? { rows: 24, cols: 80 }), cols: Number(event.target.value) } }))}
                  aria-label={tx("Columns")}
                />
                <button type="button" className="link-button" onClick={() => void resize(entry.processId)}>
                  {tx("Set size")}
                </button>
              </div>
            )}
            {stdout && <pre className="run-output">{stdout}</pre>}
            {stderr && <pre className="run-output run-error">{stderr}</pre>}
          </div>
        );
      })}
    </div>
  );
}
