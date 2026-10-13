import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";
import { parseRepoStripPrefixes, stripRepoPrefix, useSettingsStore } from "../lib/settings";
import { findRepoByFullName } from "../lib/repoLabel";

/** Renders a repo name given as `owner/name`. Prefers the repository's
 *  user-set friendly name when one is set; otherwise falls back to any
 *  configured prefix stripped (see the inbox "Strip repo prefixes" setting).
 *  The full `owner/name` is always shown as a hover tooltip. */
export function RepoName({ name, className }: { name: string; className?: string }) {
  const raw = useSettingsStore((s) => s.repoStripPrefixes);
  const prefixes = useMemo(() => parseRepoStripPrefixes(raw), [raw]);
  const reposQuery = useQuery({ queryKey: ["repositories"], queryFn: api.listRepositories });
  const match = useMemo(
    () => findRepoByFullName(reposQuery.data ?? [], name),
    [reposQuery.data, name],
  );

  if (match?.friendly_name) {
    return (
      <span className={className} title={name}>
        {match.friendly_name}
      </span>
    );
  }

  const { display, stripped } = stripRepoPrefix(name, prefixes);
  return (
    <span className={className} title={stripped ? name : undefined}>
      {display}
    </span>
  );
}
