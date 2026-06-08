/** Web URL of a GitHub PR. Assumes github.com (the only host this app targets). */
export function githubPrUrl(owner: string, name: string, prNumber: number): string {
  return `https://github.com/${owner}/${name}/pull/${prNumber}`;
}
