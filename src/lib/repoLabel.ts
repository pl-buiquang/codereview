import type { Repository } from "./types";

/** Human-friendly repo label: `owner/name` if a GitHub remote is known, else the
 *  trailing folder name of the local path. */
export function repoLabel(repo: Repository): string {
  if (repo.remote_owner && repo.remote_name) {
    return `${repo.remote_owner}/${repo.remote_name}`;
  }
  if (repo.local_path) {
    return repo.local_path.split("/").filter(Boolean).pop() ?? repo.local_path;
  }
  return "unknown";
}

/** `repoLabel` without the `owner/` prefix. */
export function repoShortName(repo: Repository): string {
  return repo.remote_name ?? repoLabel(repo);
}

/** The name to show in the UI: the user-set friendly name if any, else `repoLabel`
 *  (so existing displays are unchanged until a friendly name is set). */
export function repoDisplayName(repo: Repository): string {
  return repo.friendly_name?.trim() || repoLabel(repo);
}

/** Find the repository matching a `owner/name` string (case-insensitive), e.g. a
 *  board card's `repo` field or an inbox item's `repo` field. */
export function findRepoByFullName(repos: Repository[], fullName: string): Repository | undefined {
  const lower = fullName.toLowerCase();
  return repos.find(
    (r) => r.remote_owner && r.remote_name && `${r.remote_owner}/${r.remote_name}`.toLowerCase() === lower,
  );
}
