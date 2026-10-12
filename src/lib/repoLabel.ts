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
