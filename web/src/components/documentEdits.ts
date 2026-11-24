// The `document-edit` blocks a reply may hold: the model's changes to a chat's documents.
// The gateway applies each block when its turn completes; while the turn streams, the open
// document shows the change as it is written.

/**
 * Takes the `document-edit` blocks out of a reply. Each block is a change the gateway has applied to
 * a document; the reader sees that a document changed, not the text of the change.
 */
export function splitDocumentEdits(reply: string): { text: string; edited: number; created: number } {
  const out: string[] = [];
  let edited = 0;
  let created = 0;
  const lines = reply.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const open = /^\s*(`{3,})document-(edit\s+id=|create\b)/.exec(lines[i]);
    if (!open) {
      out.push(lines[i]);
      continue;
    }
    let close = i + 1;
    while (close < lines.length && lines[close].trim() !== open[1]) close++;
    if (close >= lines.length) {
      // Never closed: it is not a change the gateway applied, so it stays as text.
      out.push(lines[i]);
      continue;
    }
    if (open[2].startsWith("create")) created++;
    else edited++;
    i = close;
  }
  return { text: out.join("\n").replace(/\n{3,}/g, "\n\n").trim(), edited, created };
}

/** The change a streaming reply is making now: the document, its text so far, and whether the block has closed. */
export interface LiveDocumentEdit {
  id: string;
  content: string;
  closed: boolean;
}

/**
 * The newest `document-edit` block in a reply that may still be streaming. Its text so far is the
 * document's text so far; a closing fence that is only partly written is not part of the text.
 */
export function liveDocumentEdit(text: string): LiveDocumentEdit | null {
  const lines = text.split("\n");
  let opener: { id: string; fence: string; start: number } | null = null;
  for (let i = 0; i < lines.length; i++) {
    const match = /^\s*(`{3,})document-edit\s+id=([0-9a-fA-F-]{36})/.exec(lines[i]);
    if (match) opener = { id: match[2].toLowerCase(), fence: match[1], start: i };
  }
  if (!opener) return null;
  const body: string[] = [];
  for (let j = opener.start + 1; j < lines.length; j++) {
    if (lines[j].trim() === opener.fence) return { id: opener.id, content: body.join("\n"), closed: true };
    body.push(lines[j]);
  }
  // Still open: a last line of backticks shorter than the fence may be the start of the closing fence.
  const last = body[body.length - 1];
  if (last !== undefined && /^\s*`+\s*$/.test(last) && last.trim().length < opener.fence.length) body.pop();
  return { id: opener.id, content: body.join("\n"), closed: false };
}
