/**
 * The Markdown formatting actions shared by the "New clip" window and the quick
 * preview's editor.
 *
 * Each action is a pure text transform: it takes the current value plus the
 * caret/selection and reports the value and caret/selection that should follow,
 * so the caller can restore the selection once the new value is in the DOM.
 */

/** How a toolbar button edits the text around the current selection. */
export type Format =
  /** Wraps the selection, or places the caret between the markers. */
  | { kind: "wrap"; marker: string }
  /** Prefixes the touched lines, which is how headings and lists are made. */
  | { kind: "prefix"; marker: string }
  | { kind: "link" };

export interface ToolbarButton {
  label: string;
  title: string;
  format: Format;
  className?: string;
}

export interface TextEdit {
  value: string;
  start: number;
  end: number;
}

export const MARKDOWN_BUTTONS: ToolbarButton[] = [
  { label: "B", title: "Bold  **text**", format: { kind: "wrap", marker: "**" }, className: "font-bold" },
  { label: "I", title: "Italic  *text*", format: { kind: "wrap", marker: "*" }, className: "italic" },
  { label: "</>", title: "Inline code  `code`", format: { kind: "wrap", marker: "`" } },
  { label: "H1", title: "Heading  # text", format: { kind: "prefix", marker: "# " } },
  { label: "• List", title: "Bullet list  - text", format: { kind: "prefix", marker: "- " } },
  { label: "Link", title: "Link  [text](url)", format: { kind: "link" } },
];

/**
 * Applies one formatting action to `text` and reports the caret/selection that
 * should follow, so the caller can restore it after the value updates.
 */
export function applyFormat(text: string, start: number, end: number, format: Format): TextEdit {
  const selected = text.slice(start, end);

  if (format.kind === "wrap") {
    const { marker } = format;
    return {
      value: `${text.slice(0, start)}${marker}${selected}${marker}${text.slice(end)}`,
      start: start + marker.length,
      end: end + marker.length,
    };
  }

  if (format.kind === "link") {
    const label = selected || "text";
    const urlStart = start + 1 + label.length + 2;
    return {
      value: `${text.slice(0, start)}[${label}](url)${text.slice(end)}`,
      // Selecting the placeholder means the next keystroke replaces it.
      start: urlStart,
      end: urlStart + 3,
    };
  }

  // A prefix applies to every line the selection touches, or to the caret's own.
  const lineStart = text.lastIndexOf("\n", start - 1) + 1;
  const lineEnd = text.indexOf("\n", end);
  const blockEnd = lineEnd === -1 ? text.length : lineEnd;
  const lines = text.slice(lineStart, blockEnd).split("\n");
  const prefixed = lines.map((line) => `${format.marker}${line}`).join("\n");
  // Every marker inserted before a position pushes that position along, which is
  // what keeps the caret (or selection) on the same text after the edit.
  const lineStarts: number[] = [];
  let cursor = lineStart;
  for (const line of lines) {
    lineStarts.push(cursor);
    cursor += line.length + 1;
  }
  const shift = (position: number) =>
    lineStarts.filter((linePosition) => linePosition <= position).length * format.marker.length;
  return {
    value: `${text.slice(0, lineStart)}${prefixed}${text.slice(blockEnd)}`,
    start: start + shift(start),
    end: end + shift(end),
  };
}
