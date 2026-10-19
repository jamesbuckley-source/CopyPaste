/**
 * Search-bar operator parsing.
 *
 * The bar accepts free text plus `key:value` operators that narrow the history the
 * same way the sidebar does. Anything the parser does not recognise stays in the
 * text query, so a stray colon can never make results vanish silently.
 */

import type { ClipFilter, ClipKind, Collection } from "./types";

/** One recognised operator, rendered as a removable chip under the input. */
export interface SearchChip {
  /** Stable key: the operator name, e.g. `kind`. */
  key: string;
  /** What the chip reads, e.g. `Image`. */
  label: string;
  /** The raw token that was typed, so removing it is exact. */
  token: string;
}

export interface ParsedSearch {
  /** Free text left after the operators were pulled out. */
  text: string;
  filter: ClipFilter;
  chips: SearchChip[];
}

const KIND_ALIASES: Record<string, ClipKind> = {
  text: "text",
  image: "image",
  images: "image",
  files: "files",
  file: "files",
};

const KIND_LABELS: Record<ClipKind, string> = {
  text: "Text",
  image: "Images",
  files: "Files",
};

const CONTENT_KIND_ALIASES: Record<string, string> = {
  url: "url",
  link: "url",
  links: "url",
  email: "email",
  mail: "email",
  path: "path",
  file: "path",
  json: "json",
  color: "color",
  secret: "secret",
  secrets: "secret",
};

function parseDate(raw: string): number | null {
  // `before:2024-05-01` and `before:2024-05-01T12:00` both work; anything else is
  // treated as text rather than silently dropping clips.
  const value = raw.trim();
  if (!/^\d{4}-\d{2}-\d{2}([T ]\d{2}:\d{2})?$/.test(value)) return null;
  const normalised = value.includes("T") || value.includes(" ") ? value : `${value}T00:00`;
  const stamp = new Date(normalised).getTime();
  return Number.isFinite(stamp) ? stamp : null;
}

function normalizeHex(raw: string): string | null {
  const digits = raw.replace(/^#/, "").trim();
  if (/^[0-9a-f]{3}$/i.test(digits)) {
    return `#${digits
      .split("")
      .map((char) => char + char)
      .join("")
      .toLowerCase()}`;
  }
  return /^[0-9a-f]{6}$/i.test(digits) ? `#${digits.toLowerCase()}` : null;
}

/**
 * Splits `query` into a plain-text remainder, backend filter fields and the chips
 * that describe them. `collections` resolves `collection:` names to ids.
 */
export function parseSearch(query: string, collections: Collection[] = []): ParsedSearch {
  const filter: ClipFilter = {};
  const chips: SearchChip[] = [];
  const words: string[] = [];

  for (const token of query.split(/\s+/).filter(Boolean)) {
    const separator = token.indexOf(":");
    if (separator <= 0) {
      words.push(token);
      continue;
    }
    const key = token.slice(0, separator).toLowerCase();
    const value = token.slice(separator + 1);
    if (!value) {
      words.push(token);
      continue;
    }

    switch (key) {
      case "kind": {
        const kind = KIND_ALIASES[value.toLowerCase()];
        if (!kind) {
          words.push(token);
          break;
        }
        filter.kind = kind;
        chips.push({ key: "kind", label: KIND_LABELS[kind], token });
        break;
      }
      case "in": {
        if (value.toLowerCase() !== "archive" && value.toLowerCase() !== "archives") {
          words.push(token);
          break;
        }
        filter.includeArchived = true;
        chips.push({ key: "in", label: "Including archives", token });
        break;
      }
      case "type": {
        const contentKind = CONTENT_KIND_ALIASES[value.toLowerCase()];
        if (!contentKind) {
          words.push(token);
          break;
        }
        // `type:url` and friends narrow by the derived payload shape; colors have
        // their own dedicated filter, as do credential flags.
        if (contentKind === "color") filter.colorsOnly = true;
        else if (contentKind === "secret") filter.secretsOnly = true;
        else filter.contentKind = contentKind;
        chips.push({ key: "type", label: contentKind === "secret" ? "Secrets" : contentKind, token });
        break;
      }
      case "tag": {
        filter.tag = value;
        chips.push({ key: "tag", label: `#${value}`, token });
        break;
      }
      case "app": {
        filter.app = value;
        chips.push({ key: "app", label: `from ${value}`, token });
        break;
      }
      case "collection":
      case "board": {
        const match = collections.find(
          (collection) => collection.name.toLowerCase() === value.toLowerCase(),
        );
        if (!match) {
          words.push(token);
          break;
        }
        filter.collectionId = match.id;
        chips.push({ key: "collection", label: match.name, token });
        break;
      }
      case "color":
      case "color": {
        const hex = normalizeHex(value);
        if (!hex) {
          words.push(token);
          break;
        }
        filter.color = hex;
        chips.push({ key: "color", label: hex, token });
        break;
      }
      case "before": {
        const stamp = parseDate(value);
        if (stamp === null) {
          words.push(token);
          break;
        }
        filter.before = stamp;
        chips.push({ key: "before", label: `before ${value}`, token });
        break;
      }
      case "pinned": {
        const on = !/^(false|no|0)$/i.test(value);
        filter.pinnedOnly = on;
        chips.push({ key: "pinned", label: on ? "Pinned" : "Not pinned", token });
        break;
      }
      default:
        words.push(token);
    }
  }

  return { text: words.join(" "), filter, chips };
}

/** Removes one operator token from a query, leaving the rest untouched. */
export function removeToken(query: string, token: string): string {
  const tokens = query.split(/\s+/).filter(Boolean);
  const index = tokens.indexOf(token);
  if (index === -1) return query;
  tokens.splice(index, 1);
  return tokens.join(" ");
}
