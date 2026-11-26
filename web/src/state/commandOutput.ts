// Output of a command run from a code chat, as codex streams it: each chunk is base64 bytes for one stream
// of one process. Chunks are decoded as they arrive, so a character split across two chunks is kept whole.

export interface CommandOutput {
  stdout: string;
  stderr: string;
}

// One decoder per process and stream, so a multi-byte character split across chunks is decoded together.
const decoders = new Map<string, TextDecoder>();

/** Adds one `command/exec/outputDelta` notification's output to what has arrived for its process. */
export function appendCommandOutput(
  current: Record<string, CommandOutput>,
  params: Record<string, unknown>,
): Record<string, CommandOutput> {
  const processId = String(params.processId ?? "");
  if (!processId) return current;
  const stream = params.stream === "stderr" ? "stderr" : "stdout";
  const key = `${processId}:${stream}`;
  let decoder = decoders.get(key);
  if (!decoder) {
    decoder = new TextDecoder();
    decoders.set(key, decoder);
  }
  const binary = atob(String(params.deltaBase64 ?? ""));
  const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0));
  const text = decoder.decode(bytes, { stream: true });
  const previous = current[processId] ?? { stdout: "", stderr: "" };
  return { ...current, [processId]: { ...previous, [stream]: previous[stream] + text } };
}

/** Forgets a process's decoders once it has ended. */
export function forgetCommandOutput(processId: string): void {
  decoders.delete(`${processId}:stdout`);
  decoders.delete(`${processId}:stderr`);
}
