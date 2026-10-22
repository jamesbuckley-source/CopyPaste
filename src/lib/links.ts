/** Source, issues and release history. */
const PROJECT_URL = "https://github.com/jamesbuckley-source/CopyPaste";

/** Where the project publishes its installers and per-release notes. */
const RELEASES_URL = `${PROJECT_URL}/releases`;

/** The repository home page. */
export function projectUrl(): string {
  return PROJECT_URL;
}

/** Release page for one version, or the release list when it is unknown. */
export function releaseUrl(version: string | null | undefined): string {
  const trimmed = version?.trim().replace(/^v/i, "");
  return trimmed ? `${RELEASES_URL}/tag/v${trimmed}` : RELEASES_URL;
}
