export type FileCategory = "code" | "tests" | "config";

export const FILE_CATEGORIES: { key: FileCategory; label: string }[] = [
  { key: "code", label: "Code" },
  { key: "tests", label: "Tests" },
  { key: "config", label: "Config" },
];

export const DEFAULT_TEST_PATTERNS = [
  "**/test/**",
  "**/tests/**",
  "**/spec/**",
  "**/__tests__/**",
  "**/__mocks__/**",
  "**/e2e/**",
  "**/fixtures/**",
  "test_*.py",
  "*_test.py",
  "conftest.py",
  "*_spec.rb",
  "*_test.rb",
  "*.test.ts",
  "*.test.tsx",
  "*.spec.ts",
  "*.spec.tsx",
  "*.test.js",
  "*.spec.js",
  "*.snap",
].join("\n");

export const DEFAULT_CONFIG_PATTERNS = [
  "# docs",
  "*.md",
  "*.mdx",
  "*.rst",
  "*.txt",
  "**/docs/**",
  "LICENSE*",
  "CHANGELOG*",
  "# config / data",
  "*.json",
  "*.yml",
  "*.yaml",
  "*.toml",
  "*.ini",
  "*.cfg",
  "*.lock",
  ".*",
  ".github/**",
  "config/**",
  "Dockerfile*",
  "Makefile",
  "Gemfile",
  "Rakefile",
  "*.gemspec",
  "*.config.js",
  "*.config.ts",
  "*.config.mjs",
].join("\n");

/** Split a pattern list on newlines/commas; blank lines and `#` comments are dropped. */
export function parsePatternList(raw: string): string[] {
  return raw
    .split(/[\n,]/)
    .map((s) => s.trim())
    .filter((s) => s && !s.startsWith("#"));
}

/** Gitignore-ish glob: `**` spans directories, `*`/`?` stay within a segment.
 *  A pattern without `/` matches the basename; otherwise the whole path. */
export function globToRegExp(pattern: string): RegExp {
  const basenameOnly = !pattern.includes("/");
  let re = "";
  for (let i = 0; i < pattern.length; i++) {
    const c = pattern[i];
    if (c === "*") {
      if (pattern[i + 1] === "*") {
        const atSegmentStart = i === 0 || pattern[i - 1] === "/";
        i++;
        if (atSegmentStart && pattern[i + 1] === "/") {
          i++;
          re += "(?:.*/)?";
        } else {
          re += ".*";
        }
      } else {
        re += "[^/]*";
      }
    } else if (c === "?") {
      re += "[^/]";
    } else {
      re += c.replace(/[.+^${}()|[\]\\]/g, "\\$&");
    }
  }
  return new RegExp(basenameOnly ? `(?:^|/)${re}$` : `^${re}$`);
}

export interface CategoryRules {
  tests: RegExp[];
  config: RegExp[];
}

export function compileCategoryRules(testsRaw: string, configRaw: string): CategoryRules {
  return {
    tests: parsePatternList(testsRaw).map(globToRegExp),
    config: parsePatternList(configRaw).map(globToRegExp),
  };
}

export function categorize(path: string, rules: CategoryRules): FileCategory {
  if (rules.tests.some((r) => r.test(path))) return "tests";
  if (rules.config.some((r) => r.test(path))) return "config";
  return "code";
}
