import type { ReactNode } from "react";

/**
 * A small Markdown renderer that produces React elements.
 *
 * Deliberately not an HTML string: rendering user content through
 * `dangerouslySetInnerHTML` would put every clip one mistake away from injecting
 * markup into the app. Producing elements means the text can never become markup,
 * whatever it contains, and a hostile clip is just awkward characters on screen.
 *
 * The subset is the one the Markdown badge implies — headings, emphasis, code,
 * lists, quotes, rules, links — because a preview is for reading before pasting,
 * not a typesetting engine.
 */

/** Matches one inline construct at a time, in priority order. */
const INLINE =
  /(`[^`]+`)|(\*\*[^*]+\*\*)|(__[^_]+__)|(~~[^~]+~~)|(\*[^*\n]+\*)|(_[^_\n]+_)|(\[[^\]]*\]\([^)\s]+\))/;

/** Only these schemes become links; anything else stays as plain text. */
const SAFE_HREF = /^(https?:|mailto:)/i;

/** `# ` … `###### ` at the start of a line. */
const HEADING = /^(#{1,6})\s+(.*)$/;
/** A fence, with or without a language tag. */
const FENCE = /^\s*```\s*(\S*)\s*$/;
/** Three or more of the same rule character, and nothing else. */
const RULE = /^\s*([-*_])(?:\s*\1){2,}\s*$/;
/** `- `, `* ` or `+ `. */
const BULLET = /^\s*[-*+]\s+(.*)$/;
/** `1.`, `2)`, and other ordered markers. */
const ORDERED = /^\s*\d+[.)]\s+(.*)$/;
/** `> ` at the start of a line. */
const QUOTE = /^\s*>\s?(.*)$/;

/** Renders a whole document. */
export function renderMarkdown(source: string): ReactNode {
  return renderBlocks(source.replace(/\r\n?/g, "\n").split("\n"));
}

function renderBlocks(lines: string[]): ReactNode[] {
  const nodes: ReactNode[] = [];
  let index = 0;
  let key = 0;

  while (index < lines.length) {
    const line = lines[index];

    if (line.trim() === "") {
      index += 1;
      continue;
    }

    const fence = FENCE.exec(line);
    if (fence) {
      const language = fence[1];
      const body: string[] = [];
      index += 1;
      // An unclosed fence runs to the end rather than swallowing the rest as a
      // paragraph, which is what every other renderer does.
      while (index < lines.length && !FENCE.test(lines[index])) {
        body.push(lines[index]);
        index += 1;
      }
      index += 1;
      nodes.push(
        <pre
          key={key++}
          className="overflow-auto rounded bg-panel-3 p-2.5 text-[12px] leading-relaxed"
        >
          <code>{body.join("\n")}</code>
        </pre>,
      );
      if (language) {
        // The tag keeps the language visible without a highlighter.
        const last = nodes.length - 1;
        nodes[last] = (
          <div key={key++} className="relative">
            <span className="absolute right-1.5 top-1 rounded bg-panel px-1 text-[9px] uppercase tracking-wide text-ink-faint">
              {language}
            </span>
            {nodes[last]}
          </div>
        );
      }
      continue;
    }

    const heading = HEADING.exec(line);
    if (heading) {
      const level = heading[1].length;
      const sizes: Record<number, string> = {
        1: "text-[19px] font-semibold",
        2: "text-[16px] font-semibold",
        3: "text-[14px] font-semibold",
        4: "text-[13px] font-semibold",
        5: "text-[12px] font-semibold",
        6: "text-[11px] font-semibold uppercase tracking-wide",
      };
      const content = renderInline(heading[2], `h${key}`);
      const className = `mt-1 mb-1.5 text-ink ${sizes[level]}`;
      nodes.push(
        level === 1 ? (
          <h1 key={key++} className={className}>
            {content}
          </h1>
        ) : level === 2 ? (
          <h2 key={key++} className={className}>
            {content}
          </h2>
        ) : level === 3 ? (
          <h3 key={key++} className={className}>
            {content}
          </h3>
        ) : level === 4 ? (
          <h4 key={key++} className={className}>
            {content}
          </h4>
        ) : level === 5 ? (
          <h5 key={key++} className={className}>
            {content}
          </h5>
        ) : (
          <h6 key={key++} className={className}>
            {content}
          </h6>
        ),
      );
      index += 1;
      continue;
    }

    if (RULE.test(line)) {
      nodes.push(<hr key={key++} className="my-2 border-edge-soft" />);
      index += 1;
      continue;
    }

    if (QUOTE.test(line)) {
      const quoted: string[] = [];
      while (index < lines.length) {
        const part = QUOTE.exec(lines[index]);
        if (!part) break;
        quoted.push(part[1]);
        index += 1;
      }
      nodes.push(
        <blockquote
          key={key++}
          className="my-1.5 border-l-2 border-accent-dim pl-2.5 text-ink-dim"
        >
          {renderBlocks(quoted)}
        </blockquote>,
      );
      continue;
    }

    if (BULLET.test(line) || ORDERED.test(line)) {
      const ordered = !BULLET.test(line);
      const pattern = ordered ? ORDERED : BULLET;
      const items: string[] = [];
      while (index < lines.length) {
        const item = pattern.exec(lines[index]);
        if (item) {
          items.push(item[1]);
          index += 1;
          continue;
        }
        // A continuation line belongs to the item above it.
        if (items.length > 0 && lines[index].trim() !== "" && !isBlockStart(lines[index])) {
          items[items.length - 1] += ` ${lines[index].trim()}`;
          index += 1;
          continue;
        }
        break;
      }
      const children = items.map((item, at) => (
        <li key={at} className="ml-4 list-outside">
          {renderInline(item, `li${at}`)}
        </li>
      ));
      nodes.push(
        ordered ? (
          <ol key={key++} className="my-1.5 list-decimal space-y-0.5">
            {children}
          </ol>
        ) : (
          <ul key={key++} className="my-1.5 list-disc space-y-0.5">
            {children}
          </ul>
        ),
      );
      continue;
    }

    // Anything else is a paragraph, continuing until a blank line or the start of
    // a block that cannot be part of one.
    const paragraph: string[] = [line];
    index += 1;
    while (index < lines.length && lines[index].trim() !== "" && !isBlockStart(lines[index])) {
      paragraph.push(lines[index]);
      index += 1;
    }
    nodes.push(
      <p key={key++} className="my-1 leading-relaxed">
        {renderInline(paragraph.join(" "), `p${key}`)}
      </p>,
    );
  }

  return nodes;
}

/** True for a line that begins a block of its own rather than continuing one. */
function isBlockStart(line: string): boolean {
  return (
    FENCE.test(line) ||
    HEADING.test(line) ||
    RULE.test(line) ||
    QUOTE.test(line) ||
    BULLET.test(line) ||
    ORDERED.test(line)
  );
}

/** Turns one line of text into elements and strings, with no HTML involved. */
function renderInline(text: string, keyPrefix: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  let rest = text;
  let key = 0;

  while (rest.length > 0) {
    const match = INLINE.exec(rest);
    if (!match) {
      nodes.push(rest);
      break;
    }
    if (match.index > 0) {
      nodes.push(rest.slice(0, match.index));
    }
    const token = match[0];
    const k = `${keyPrefix}-${key++}`;

    if (token.startsWith("`")) {
      nodes.push(
        <code key={k} className="rounded bg-panel-3 px-1 py-0.5 font-mono text-[0.92em]">
          {token.slice(1, -1)}
        </code>,
      );
    } else if (token.startsWith("**") || token.startsWith("__")) {
      nodes.push(
        <strong key={k} className="font-semibold text-ink">
          {renderInline(token.slice(2, -2), k)}
        </strong>,
      );
    } else if (token.startsWith("~~")) {
      nodes.push(
        <s key={k} className="text-ink-faint">
          {token.slice(2, -2)}
        </s>,
      );
    } else if (token.startsWith("*") || token.startsWith("_")) {
      nodes.push(<em key={k}>{renderInline(token.slice(1, -1), k)}</em>);
    } else if (token.startsWith("[")) {
      const cut = token.indexOf("](");
      const label = token.slice(1, cut);
      const href = token.slice(cut + 2, -1);
      nodes.push(
        SAFE_HREF.test(href) ? (
          <a
            key={k}
            href={href}
            target="_blank"
            rel="noreferrer noopener"
            className="text-accent-hi underline decoration-accent-dim underline-offset-2 hover:decoration-accent"
          >
            {label || href}
          </a>
        ) : (
          // A target that is not http(s) or mailto keeps the whole construct as
          // text: `file:` and `javascript:` have no business being clickable, and
          // showing only the label would hide where it pointed.
          <span key={k}>{token}</span>
        ),
      );
    }

    rest = rest.slice(match.index + token.length);
  }

  return nodes;
}
