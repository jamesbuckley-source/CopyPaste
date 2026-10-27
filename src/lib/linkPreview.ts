/**
 * The offline half of a link preview: a tile made from the address itself.
 *
 * Nothing about this asks the network. A copied link gets a readable card with the
 * site's own colour, and the optional fetched picture replaces it once one exists.
 */

/** Host of a link, without a leading `www.`; the trimmed text if it cannot be parsed. */
export function linkHost(text: string): string {
  const trimmed = text.trim();
  try {
    return new URL(trimmed).hostname.replace(/^www\./i, "") || trimmed;
  } catch {
    // A truncated payload, or a link the parser rejects: fall back to the text.
  }
  const withoutScheme = trimmed.replace(/^[a-z][a-z0-9+.-]*:\/\//i, "");
  const host = withoutScheme.split(/[/?#]/)[0] ?? "";
  return host.replace(/^www\./i, "") || trimmed;
}

/** A stable hue per host, so the same site always looks the same. */
export function linkHue(host: string): number {
  let hash = 0;
  for (const char of host) hash = (hash * 31 + char.charCodeAt(0)) % 360;
  return hash;
}

/** First letter of the host, for the tile. */
export function linkInitial(host: string): string {
  const letter = [...host].find((char) => /[a-z0-9]/i.test(char));
  return (letter ?? "?").toUpperCase();
}
