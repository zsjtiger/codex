// Converts between a document's Markdown, which is what the gateway stores, and the HTML a rich-text
// editor edits. It covers the blocks documents use: headings, paragraphs, lists, quotes, code blocks,
// rules, and bold, italic, code and link text. Anything else is kept as plain text.

const ESCAPES: Record<string, string> = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" };

function escapeHtml(text: string): string {
  return text.replace(/[&<>"']/g, (ch) => ESCAPES[ch]);
}

function inlineHtml(text: string): string {
  return escapeHtml(text)
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/(^|[^*\w])\*([^*\s][^*]*)\*(?!\w)/g, "$1<em>$2</em>")
    .replace(/\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g, '<a href="$2">$1</a>')
    .replace(/\n/g, "<br>");
}

/** Renders Markdown as the HTML the editor shows. Text is escaped, so no markup comes through as HTML. */
export function markdownToHtml(markdown: string): string {
  const lines = markdown.replace(/\r\n?/g, "\n").split("\n");
  const out: string[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (/^```/.test(line)) {
      const code: string[] = [];
      i++;
      while (i < lines.length && !/^```/.test(lines[i])) code.push(lines[i++]);
      i++;
      out.push(`<pre><code>${escapeHtml(code.join("\n"))}</code></pre>`);
    } else if (/^#{1,6}\s/.test(line)) {
      const level = line.match(/^#+/)![0].length;
      out.push(`<h${level}>${inlineHtml(line.replace(/^#+\s*/, "").trim())}</h${level}>`);
      i++;
    } else if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(line)) {
      out.push("<hr>");
      i++;
    } else if (/^>\s?/.test(line)) {
      const quoted: string[] = [];
      while (i < lines.length && /^>\s?/.test(lines[i])) quoted.push(lines[i++].replace(/^>\s?/, ""));
      out.push(`<blockquote>${markdownToHtml(quoted.join("\n"))}</blockquote>`);
    } else if (/^\s*[-*+]\s+/.test(line)) {
      const items: string[] = [];
      while (i < lines.length && /^\s*[-*+]\s+/.test(lines[i])) items.push(`<li>${inlineHtml(lines[i++].replace(/^\s*[-*+]\s+/, ""))}</li>`);
      out.push(`<ul>${items.join("")}</ul>`);
    } else if (/^\s*\d+\.\s+/.test(line)) {
      const items: string[] = [];
      while (i < lines.length && /^\s*\d+\.\s+/.test(lines[i])) items.push(`<li>${inlineHtml(lines[i++].replace(/^\s*\d+\.\s+/, ""))}</li>`);
      out.push(`<ol>${items.join("")}</ol>`);
    } else if (line.trim() === "") {
      i++;
    } else {
      const paragraph: string[] = [];
      while (i < lines.length && lines[i].trim() !== "" && !isBlockStart(lines[i])) paragraph.push(lines[i++]);
      out.push(`<p>${inlineHtml(paragraph.join("\n"))}</p>`);
    }
  }
  return out.join("");
}

function isBlockStart(line: string): boolean {
  return /^```|^#{1,6}\s|^>\s?|^\s*[-*+]\s+|^\s*\d+\.\s+|^\s*([-*_])(\s*\1){2,}\s*$/.test(line);
}

const BLOCKS = new Set(["H1", "H2", "H3", "H4", "H5", "H6", "P", "DIV", "UL", "OL", "LI", "BLOCKQUOTE", "PRE", "HR"]);

function inlineMarkdown(node: Node): string {
  if (node.nodeType === Node.TEXT_NODE) return node.textContent ?? "";
  if (!(node instanceof HTMLElement)) return "";
  const inner = () => Array.from(node.childNodes).map(inlineMarkdown).join("");
  switch (node.tagName) {
    case "STRONG":
    case "B": {
      const text = inner();
      return text.trim() ? `**${text}**` : text;
    }
    case "EM":
    case "I": {
      const text = inner();
      return text.trim() ? `*${text}*` : text;
    }
    case "CODE":
      return `\`${node.textContent ?? ""}\``;
    case "A": {
      const href = node.getAttribute("href") ?? "";
      const text = inner();
      return /^https?:\/\//.test(href) ? `[${text}](${href})` : text;
    }
    case "BR":
      return "\n";
    default:
      return inner();
  }
}

/** Markdown for a container's children: runs of inline content become paragraphs, blocks keep their form. */
function containerMarkdown(container: Node): string {
  const parts: string[] = [];
  let run = "";
  const flush = () => {
    const text = run.trim();
    if (text) parts.push(`${text}\n\n`);
    run = "";
  };
  for (const child of Array.from(container.childNodes)) {
    if (child instanceof HTMLElement && BLOCKS.has(child.tagName)) {
      flush();
      parts.push(blockMarkdown(child));
    } else {
      run += inlineMarkdown(child);
    }
  }
  flush();
  return parts.join("");
}

function blockMarkdown(el: HTMLElement): string {
  const tag = el.tagName;
  const heading = /^H([1-6])$/.exec(tag);
  if (heading) return `${"#".repeat(Number(heading[1]))} ${inlineMarkdown(el).trim()}\n\n`;
  switch (tag) {
    case "HR":
      return "---\n\n";
    case "PRE":
      return `\`\`\`\n${(el.textContent ?? "").replace(/\n$/, "")}\n\`\`\`\n\n`;
    case "BLOCKQUOTE":
      return containerMarkdown(el)
        .trimEnd()
        .split("\n")
        .map((line) => (line ? `> ${line}` : ">"))
        .join("\n") + "\n\n";
    case "UL":
    case "OL": {
      const items = Array.from(el.children).filter((child) => child.tagName === "LI");
      return items
        .map((li, index) => {
          const marker = tag === "OL" ? `${index + 1}. ` : "- ";
          const indent = " ".repeat(marker.length);
          const body = containerMarkdown(li).trim().split("\n").map((line, n) => (n === 0 ? line : indent + line));
          return `${marker}${body.join("\n")}`;
        })
        .join("\n") + "\n\n";
    }
    case "LI":
      return containerMarkdown(el);
    default: {
      // A paragraph, or a division the editor creates on Enter: blocks inside keep their form.
      const hasBlocks = Array.from(el.childNodes).some((child) => child instanceof HTMLElement && BLOCKS.has(child.tagName));
      return hasBlocks ? containerMarkdown(el) : `${inlineMarkdown(el).trim()}\n\n`.replace(/^\n\n$/, "");
    }
  }
}

/** Reads the editor's HTML back into Markdown, the form the gateway stores. */
export function htmlToMarkdown(html: string): string {
  const body = new DOMParser().parseFromString(html, "text/html").body;
  return containerMarkdown(body).replace(/\n{3,}/g, "\n\n").trim();
}
