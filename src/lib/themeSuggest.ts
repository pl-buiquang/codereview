import { api } from "./api";
import {
  contrastIssues,
  mergeScope,
  parseCustomTheme,
  type CustomScope,
  type CustomTheme,
} from "./customTheme";

/** Moods for "AI suggest" when the user gives no hint: each click designs something different. */
export const INSPIRATIONS = [
  "nordic dusk",
  "solarized paper",
  "terminal amber",
  "sakura in spring",
  "deep ocean",
  "espresso and cream",
  "forest canopy",
  "synthwave night",
  "newsprint and ink",
  "desert at noon",
  "arctic glacier",
  "lavender fields",
  "autumn maple",
  "midnight library",
  "citrus grove",
  "slate and copper",
  "mint chocolate",
  "old blueprint",
  "northern lights",
  "rose gold",
  "volcanic ash",
  "coastal fog",
  "retro CRT green",
  "plum wine",
  "sandstone and sage",
  "neon tokyo rain",
  "matcha latte",
  "graphite and lime",
  "twilight violet",
  "warm paper, low contrast",
] as const;

export function randomInspiration(rand: () => number = Math.random): string {
  return INSPIRATIONS[Math.floor(rand() * INSPIRATIONS.length)];
}

/** The first balanced `{…}` in a model reply (it may add prose or a code fence), parsed; null if none. */
export function extractJson(text: string): unknown | null {
  const start = text.indexOf("{");
  if (start < 0) return null;
  let depth = 0;
  let inString = false;
  for (let i = start; i < text.length; i++) {
    const c = text[i];
    if (inString) {
      if (c === "\\") i++;
      else if (c === '"') inString = false;
    } else if (c === '"') inString = true;
    else if (c === "{") depth++;
    else if (c === "}" && --depth === 0) {
      try {
        return JSON.parse(text.slice(start, i + 1));
      } catch {
        return null;
      }
    }
  }
  return null;
}

export interface Suggestion {
  /** What the model designed, normalized; apply it with `mergeScope`. */
  suggested: CustomTheme;
  /** The mood used: the user's hint, or a random one. */
  inspiration: string;
  /** Contrast problems left in the merged theme after the retry. */
  issues: string[];
}

type Ask = typeof api.themeSuggest;

const COLOUR_SCOPES: readonly CustomScope[] = ["all", "colours", "syntax"];

/**
 * Ask the model for a theme in `scope`, built on `current`. If the result has contrast
 * problems, ask once more with those problems as feedback and keep the retry when it
 * parses. Throws when the reply holds no usable theme.
 */
export async function suggestTheme(opts: {
  scope: CustomScope;
  current: CustomTheme;
  hint?: string;
  ask?: Ask;
}): Promise<Suggestion> {
  const { scope, current, ask = api.themeSuggest } = opts;
  const inspiration = opts.hint?.trim() || randomInspiration();
  const colours = COLOUR_SCOPES.includes(scope);

  const attempt = async (feedback?: string): Promise<{ suggested: CustomTheme; issues: string[] }> => {
    const json = extractJson(await ask(current, scope, inspiration, feedback));
    if (json === null) throw new Error("The model did not return a theme. Try again.");
    const suggested = parseCustomTheme(json);
    if (colours && Object.keys(suggested.dark).length + Object.keys(suggested.light).length === 0) {
      throw new Error("The model's theme had no usable colors. Try again.");
    }
    return { suggested, issues: colours ? contrastIssues(mergeScope(current, suggested, scope)) : [] };
  };

  let result = await attempt();
  if (result.issues.length > 0) {
    try {
      const retry = await attempt(result.issues.join("\n"));
      if (retry.issues.length <= result.issues.length) result = retry;
    } catch {
      // keep the first design: it still applies, with a warning
    }
  }
  return { ...result, inspiration };
}
