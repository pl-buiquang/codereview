import { useMemo } from "react";
import { parseRepoStripPrefixes, stripRepoPrefix, useSettingsStore } from "../lib/settings";

/** Renders a repo name with any configured prefix stripped (see the inbox
 *  "Strip repo prefixes" setting). When a prefix is stripped, the full
 *  `owner/name` is shown as a hover tooltip. */
export function RepoName({ name, className }: { name: string; className?: string }) {
  const raw = useSettingsStore((s) => s.repoStripPrefixes);
  const prefixes = useMemo(() => parseRepoStripPrefixes(raw), [raw]);
  const { display, stripped } = stripRepoPrefix(name, prefixes);
  return (
    <span className={className} title={stripped ? name : undefined}>
      {display}
    </span>
  );
}
