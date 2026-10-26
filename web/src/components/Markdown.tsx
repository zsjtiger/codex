import { type ReactNode } from "react";

export interface Heading {
  id: string;
  level: number;
  text: string;
}

type Block =
  | { type: "heading"; level: number; text: string; id: string }
  | { type: "paragraph"; text: string }
  | { type: "list"; ordered: boolean; items: string[] }
  | { type: "code"; text: string }
  | { type: "quote"; text: string }
  | { type: "rule" };

/** Turns heading text into a stable anchor, keeping CJK characters. */
function slugify(text: string): string {
  return text
    .toLowerCase()
    .replace(/[`*_~[\]()]/g, "")
    .trim()
    .replace(/\s+/g, "-")
    .replace(/[^\p{L}\p{N}-]/gu, "");
}

function parseBlocks(source: string): Block[] {
  const lines = source.replace(/\r\n?/g, "\n").split("\n");
  const blocks: Block[] = [];
  const seen = new Map<string, number>();
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    if (line.trim() === "") {
      i++;
      continue;
    }

    if (line.trimStart().startsWith("```")) {
      const body: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trimStart().startsWith("```")) {
        body.push(lines[i]);
        i++;
      }
      i++;
      blocks.push({ type: "code", text: body.join("\n") });
      continue;
    }

    const heading = /^(#{1,6})\s+(.*?)\s*#*\s*$/.exec(line);
    if (heading) {
      const text = heading[2];
      const base = slugify(text) || "section";
      const count = seen.get(base) ?? 0;
      seen.set(base, count + 1);
      blocks.push({ type: "heading", level: heading[1].length, text, id: count ? `${base}-${count}` : base });
      i++;
      continue;
    }

    if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(line)) {
      blocks.push({ type: "rule" });
      i++;
      continue;
    }

    if (/^\s*>/.test(line)) {
      const body: string[] = [];
      while (i < lines.length && /^\s*>/.test(lines[i])) {
        body.push(lines[i].replace(/^\s*>\s?/, ""));
        i++;
      }
      blocks.push({ type: "quote", text: body.join("\n") });
      continue;
    }

    const bullet = /^\s*[-*+]\s+(.*)$/;
    const ordered = /^\s*\d+[.)]\s+(.*)$/;
    if (bullet.test(line) || ordered.test(line)) {
      const isOrdered = ordered.test(line);
      const pattern = isOrdered ? ordered : bullet;
      const items: string[] = [];
      while (i < lines.length && pattern.test(lines[i])) {
        items.push(pattern.exec(lines[i])?.[1] ?? "");
        i++;
      }
      blocks.push({ type: "list", ordered: isOrdered, items });
      continue;
    }

    const body: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() !== "" &&
      !/^(#{1,6}\s|```|\s*>|\s*[-*+]\s|\s*\d+[.)]\s)/.test(lines[i])
    ) {
      body.push(lines[i]);
      i++;
    }
    if (body.length === 0) {
      body.push(lines[i]);
      i++;
    }
    blocks.push({ type: "paragraph", text: body.join("\n") });
  }

  return blocks;
}

/** Lists the headings in a document, in order, for a table of contents. */
export function markdownHeadings(source: string): Heading[] {
  return parseBlocks(source)
    .filter((block): block is Extract<Block, { type: "heading" }> => block.type === "heading")
    .map(({ id, level, text }) => ({ id, level, text }));
}

const SAFE_LINK = /^(https?:\/\/|mailto:)/i;

/** Renders inline Markdown as React nodes. Nothing is parsed as HTML. */
function inline(text: string, keyPrefix = "i"): ReactNode[] {
  const nodes: ReactNode[] = [];
  const pattern = /(`[^`]+`)|(\*\*[^*]+\*\*|__[^_]+__)|(\*[^*\n]+\*|_[^_\n]+_)|(\[[^\]]+\]\([^)\s]+\))/g;
  let last = 0;
  let match: RegExpExecArray | null;
  let n = 0;

  while ((match = pattern.exec(text)) !== null) {
    if (match.index > last) nodes.push(text.slice(last, match.index));
    const key = `${keyPrefix}-${n++}`;
    const token = match[0];
    if (match[1]) {
      nodes.push(<code key={key}>{token.slice(1, -1)}</code>);
    } else if (match[2]) {
      nodes.push(<strong key={key}>{inline(token.slice(2, -2), key)}</strong>);
    } else if (match[3]) {
      nodes.push(<em key={key}>{inline(token.slice(1, -1), key)}</em>);
    } else {
      const link = /^\[([^\]]+)\]\(([^)\s]+)\)$/.exec(token);
      if (link && SAFE_LINK.test(link[2])) {
        nodes.push(
          <a key={key} href={link[2]} target="_blank" rel="noreferrer noopener">
            {inline(link[1], key)}
          </a>,
        );
      } else {
        nodes.push(token);
      }
    }
    last = match.index + token.length;
  }
  if (last < text.length) nodes.push(text.slice(last));
  return nodes;
}

function withBreaks(text: string, keyPrefix: string): ReactNode[] {
  return text.split("\n").flatMap((line, index, all) => {
    const parts = inline(line, `${keyPrefix}-${index}`);
    return index < all.length - 1 ? [...parts, <br key={`${keyPrefix}-br-${index}`} />] : parts;
  });
}

/** Renders a Markdown document. Output is React elements only, so raw HTML stays as text. */
export function MarkdownView({ source }: { source: string }) {
  const blocks = parseBlocks(source);
  return (
    <article className="markdown">
      {blocks.map((block, index) => {
        const key = `b-${index}`;
        switch (block.type) {
          case "heading": {
            const Tag = `h${Math.min(block.level, 6)}` as "h1";
            return (
              <Tag key={key} id={block.id}>
                {inline(block.text, key)}
              </Tag>
            );
          }
          case "paragraph":
            return <p key={key}>{withBreaks(block.text, key)}</p>;
          case "list": {
            const items = block.items.map((item, itemIndex) => <li key={`${key}-${itemIndex}`}>{inline(item, `${key}-${itemIndex}`)}</li>);
            return block.ordered ? <ol key={key}>{items}</ol> : <ul key={key}>{items}</ul>;
          }
          case "code":
            return (
              <pre key={key}>
                <code>{block.text}</code>
              </pre>
            );
          case "quote":
            return <blockquote key={key}>{withBreaks(block.text, key)}</blockquote>;
          case "rule":
            return <hr key={key} />;
        }
      })}
    </article>
  );
}

/** Table of contents with links to each heading. */
export function MarkdownToc({ headings }: { headings: Heading[] }) {
  if (headings.length === 0) return null;
  return (
    <nav className="markdown-toc" aria-label="Table of contents">
      <div className="markdown-toc-title">目录</div>
      <ol>
        {headings
          .filter((heading) => heading.level <= 2)
          .map((heading) => (
            <li key={heading.id}>
              <a
                href={`#${heading.id}`}
                onClick={(event) => {
                  event.preventDefault();
                  document.getElementById(heading.id)?.scrollIntoView({ behavior: "smooth", block: "start" });
                }}
              >
                {heading.text}
              </a>
            </li>
          ))}
      </ol>
    </nav>
  );
}
