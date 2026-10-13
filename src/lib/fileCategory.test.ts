import { describe, expect, it } from "vitest";
import {
  DEFAULT_CONFIG_PATTERNS,
  DEFAULT_TEST_PATTERNS,
  categorize,
  compileCategoryRules,
  globToRegExp,
  parsePatternList,
} from "./fileCategory";

describe("globToRegExp", () => {
  it("matches basename-only patterns at any depth", () => {
    const re = globToRegExp("*.test.ts");
    expect(re.test("foo.test.ts")).toBe(true);
    expect(re.test("src/lib/foo.test.ts")).toBe(true);
    expect(re.test("src/lib/foo.ts")).toBe(false);
  });

  it("anchors patterns containing a slash to the full path", () => {
    const re = globToRegExp("config/**");
    expect(re.test("config/routes.rb")).toBe(true);
    expect(re.test("config/initializers/x.rb")).toBe(true);
    expect(re.test("app/config/x.rb")).toBe(false);
  });

  it("lets a leading **/ match zero or more directories", () => {
    const re = globToRegExp("**/tests/**");
    expect(re.test("tests/test_x.py")).toBe(true);
    expect(re.test("pkg/a/tests/helpers.py")).toBe(true);
    expect(re.test("pkg/mytests/x.py")).toBe(false);
  });

  it("keeps * and ? within a segment and escapes regex chars", () => {
    expect(globToRegExp("src/*.ts").test("src/a/b.ts")).toBe(false);
    expect(globToRegExp("a?.py").test("ab.py")).toBe(true);
    expect(globToRegExp("a+b.md").test("a+b.md")).toBe(true);
    expect(globToRegExp("*.md").test("readme_md")).toBe(false);
  });
});

describe("parsePatternList", () => {
  it("splits on newlines and commas, dropping blanks and comments", () => {
    expect(parsePatternList("# docs\n*.md, *.rst\n\n  *.txt  ")).toEqual([
      "*.md",
      "*.rst",
      "*.txt",
    ]);
  });
});

describe("categorize with defaults", () => {
  const rules = compileCategoryRules(DEFAULT_TEST_PATTERNS, DEFAULT_CONFIG_PATTERNS);

  it.each([
    ["tests/test_api.py", "tests"],
    ["pkg/foo_test.py", "tests"],
    ["conftest.py", "tests"],
    ["spec/models/user_spec.rb", "tests"],
    ["src/lib/diff.test.ts", "tests"],
    ["src/__tests__/a.tsx", "tests"],
    ["tests/fixtures/data.json", "tests"],
    ["README.md", "config"],
    ["docs/guide/setup.py", "config"],
    ["pyproject.toml", "config"],
    ["Gemfile.lock", "config"],
    [".github/workflows/ci.yml", "config"],
    ["config/routes.rb", "config"],
    [".rubocop.yml", "config"],
    ["vite.config.ts", "config"],
    ["src/lib/diff.ts", "code"],
    ["app/models/user.rb", "code"],
    ["pkg/core.py", "code"],
  ])("%s → %s", (path, expected) => {
    expect(categorize(path, rules)).toBe(expected);
  });
});
