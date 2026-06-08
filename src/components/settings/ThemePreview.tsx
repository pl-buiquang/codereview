import type { CSSProperties } from "react";
import { tokenVar, UI_VAR, type Theme, type TokenRole } from "../../lib/settings";

/**
 * A small fixed code snippet rendered with the given theme's tokens applied as
 * inline CSS variables, so a non-active theme can be previewed without changing
 * the rest of the app. The token spans are colored by `.theme-preview .token.*`
 * rules that read the `--tok-*` variables set inline here.
 */
export function ThemePreview({ theme }: { theme: Theme }) {
  const style: Record<string, string> = {
    [UI_VAR.bg]: theme.ui.bg,
    [UI_VAR.text]: theme.ui.text,
    [UI_VAR.border]: theme.ui.border,
  };
  (Object.keys(theme.syntax) as TokenRole[]).forEach((role) => {
    style[tokenVar(role)] = theme.syntax[role];
  });
  if (theme.codeFont) style["--code-font"] = theme.codeFont;

  return (
    <div className="theme-preview" style={{ borderColor: theme.ui.border }}>
      <pre style={style as CSSProperties}>
        <code>
          <span className="token comment">{"// greet a user"}</span>
          {"\n"}
          <span className="token keyword">export</span>{" "}
          <span className="token keyword">const</span>{" "}
          <span className="token function">greet</span>{" "}
          <span className="token operator">=</span> (
          <span className="token variable">name</span>){" "}
          <span className="token operator">{"=>"}</span> {"{"}
          {"\n  "}
          <span className="token keyword">const</span> tries{" "}
          <span className="token operator">=</span>{" "}
          <span className="token literal">42</span>
          <span className="token punctuation">;</span>
          {"\n  "}
          <span className="token keyword">return</span>{" "}
          <span className="token string">{"`Hello, ${name}`"}</span>
          <span className="token punctuation">;</span>
          {"\n"}
          {"}"}
        </code>
      </pre>
    </div>
  );
}
