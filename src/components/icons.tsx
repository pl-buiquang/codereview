import type { CSSProperties, ReactNode } from "react";

/**
 * The design-system icon set — inline 16×16 stroke SVGs (1.4px, currentColor),
 * ported verbatim from `CRIcon` in the design handoff
 * (specs/design_handoff_codereview_redesign/reference/cr/chrome.jsx).
 */
export type IconName =
  | "menu"
  | "home"
  | "x"
  | "plus"
  | "min"
  | "max"
  | "inbox"
  | "review"
  | "archive"
  | "repo"
  | "gear"
  | "refresh"
  | "chev"
  | "check"
  | "back"
  | "ext"
  | "comment"
  | "eye"
  | "branch"
  | "file"
  | "folder"
  | "bot"
  | "person"
  | "flame"
  | "team"
  | "closed"
  | "sort"
  | "dot";

const STROKE: Record<Exclude<IconName, "dot">, ReactNode> = {
  menu: <path d="M2.5 4.5h11M2.5 8h11M2.5 11.5h11" />,
  home: (
    <>
      <path d="M2.5 7.5 8 2.8l5.5 4.7" />
      <path d="M3.8 7v6h8.4V7" />
    </>
  ),
  x: <path d="m4 4 8 8M12 4l-8 8" />,
  plus: <path d="M8 3v10M3 8h10" />,
  min: <path d="M3 11.5h10" />,
  max: <rect x="3.5" y="3.5" width="9" height="9" rx="1" />,
  inbox: (
    <>
      <path d="M2.5 9.5h3.2l1 1.8h2.6l1-1.8h3.2" />
      <path d="M3.6 3.5h8.8l1.1 6v3h-11v-3z" />
    </>
  ),
  review: (
    <>
      <path d="M3.5 2.5h9v11h-9z" />
      <path d="m5.8 8.2 1.5 1.5 3-3.4" />
    </>
  ),
  archive: (
    <>
      <rect x="2.5" y="3" width="11" height="3" rx="0.8" />
      <path d="M3.5 6v7h9V6M6.5 8.5h3" />
    </>
  ),
  repo: (
    <>
      <path d="M4.5 2.5h8v11h-8a1.5 1.5 0 0 1-1.5-1.5V4a1.5 1.5 0 0 1 1.5-1.5Z" />
      <path d="M3 10.5h9.5" />
    </>
  ),
  gear: (
    <>
      <circle cx="8" cy="8" r="2.2" />
      <path d="M8 1.8v2M8 12.2v2M1.8 8h2M12.2 8h2M3.6 3.6l1.4 1.4M11 11l1.4 1.4M12.4 3.6 11 5M5 11l-1.4 1.4" />
    </>
  ),
  refresh: (
    <>
      <path d="M13 8a5 5 0 1 1-1.6-3.7" />
      <path d="M13 2.8v2.8h-2.8" />
    </>
  ),
  chev: <path d="m4.5 6.5 3.5 3.5L11.5 6.5" />,
  check: <path d="m3.5 8.5 3 3 6-6.5" />,
  back: <path d="M9.5 3.5 5 8l4.5 4.5" />,
  ext: (
    <>
      <path d="M6.5 3.5h-3v9h9v-3" />
      <path d="M9 2.5h4.5V7M13 3 7.5 8.5" />
    </>
  ),
  comment: <path d="M2.5 3.5h11v7h-6L4 13v-2.5H2.5z" />,
  eye: (
    <>
      <path d="M1.8 8S4 4.2 8 4.2 14.2 8 14.2 8 12 11.8 8 11.8 1.8 8 1.8 8Z" />
      <circle cx="8" cy="8" r="1.8" />
    </>
  ),
  branch: (
    <>
      <circle cx="4.5" cy="3.8" r="1.5" />
      <circle cx="4.5" cy="12.2" r="1.5" />
      <circle cx="11.5" cy="6" r="1.5" />
      <path d="M4.5 5.3v5.4M11.5 7.5c0 2.5-4 2-6.4 2.6" />
    </>
  ),
  file: (
    <>
      <path d="M4 1.8h5.5L12 4.3v9.9H4z" />
      <path d="M9.2 2v2.6H12" />
    </>
  ),
  folder: <path d="M1.8 3.5h4.4l1.2 1.6h6.8v7.4H1.8z" />,
  bot: (
    <>
      <rect x="3" y="5" width="10" height="7.5" rx="1.5" />
      <path d="M8 2.5V5M6 8.2h.01M10 8.2h.01" />
    </>
  ),
  person: (
    <>
      <circle cx="8" cy="5.2" r="2.6" />
      <path d="M2.8 13.6a5.3 5.3 0 0 1 10.4 0" />
    </>
  ),
  flame: (
    <path d="M8 1.8C9 4 12 5.5 12 9a4 4 0 0 1-8 0c0-1.5.6-2.5 1.4-3.6C5.8 6.6 7 7 7 7c-.4-2 .2-3.8 1-5.2Z" />
  ),
  team: (
    <>
      <circle cx="5.5" cy="5.5" r="2" />
      <circle cx="10.8" cy="6.2" r="1.6" />
      <path d="M1.8 12.8a4 4 0 0 1 7.4 0M9.8 12.8a3.4 3.4 0 0 1 4.4-2.4" />
    </>
  ),
  closed: (
    <>
      <circle cx="8" cy="8" r="5.7" />
      <path d="m5.6 8.3 1.7 1.7 3.2-3.6" />
    </>
  ),
  sort: <path d="M4.5 3v10M4.5 13 2.5 11M4.5 13l2-2M11.5 13V3M11.5 3l-2 2M11.5 3l2 2" />,
};

export function Icon({
  name,
  size = 14,
  className,
  style,
}: {
  name: IconName;
  size?: number;
  className?: string;
  style?: CSSProperties;
}) {
  const common = {
    width: size,
    height: size,
    viewBox: "0 0 16 16",
    className,
    style: { flex: "none", ...style } as CSSProperties,
    "aria-hidden": true,
  };
  if (name === "dot") {
    return (
      <svg {...common} fill="currentColor">
        <circle cx="8" cy="8" r="3" />
      </svg>
    );
  }
  return (
    <svg
      {...common}
      fill="none"
      stroke="currentColor"
      strokeWidth={1.4}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {STROKE[name]}
    </svg>
  );
}
