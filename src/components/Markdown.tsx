import { isValidElement, useEffect, useId, useState, type ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import rehypeRaw from "rehype-raw";
import rehypeSanitize, { defaultSchema } from "rehype-sanitize";
import remarkGfm from "remark-gfm";
import { api } from "../lib/api";
import { languageForFence } from "../lib/diff";
import { highlightToHast, isLanguageReady, useSyntaxReady } from "../lib/highlight";

/**
 * GitHub's own sanitize allow-list (`details`, `summary`, `kbd`, … but no scripts or event handlers;
 * HTML comments are dropped), plus `open` on `details`.
 */
const SANITIZE_SCHEMA = {
  ...defaultSchema,
  attributes: {
    ...defaultSchema.attributes,
    details: [...(defaultSchema.attributes?.details ?? []), "open"],
  },
};

const GITHUB_IMAGE_RE =
  /^https:\/\/(?:github\.com\/.*\/assets\/|user-images\.githubusercontent\.com\/|private-user-images\.githubusercontent\.com\/|github\.com\/user-attachments\/)/;

const MAX_CACHED_IMAGES = 200;
const imageCache = new Map<string, string>();
const imageInflight = new Map<string, Promise<string>>();

/** Proxied GitHub images by URL, shared across mounts; failures aren't cached. */
function loadGithubImage(src: string, fetchUrl: string = src): Promise<string> {
  const cached = imageCache.get(src);
  if (cached) return Promise.resolve(cached);
  let pending = imageInflight.get(src);
  if (!pending) {
    pending = api
      .fetchGithubImage(fetchUrl)
      .then((url) => {
        imageCache.set(src, url);
        if (imageCache.size > MAX_CACHED_IMAGES) {
          imageCache.delete(imageCache.keys().next().value as string);
        }
        return url;
      })
      .finally(() => imageInflight.delete(src));
    imageInflight.set(src, pending);
  }
  return pending;
}

function ProxiedImage({ src, alt, fetchUrl }: { src: string; alt?: string; fetchUrl?: string }) {
  const [loaded, setLoaded] = useState<{ src: string; url: string } | null>(() => {
    const url = imageCache.get(src);
    return url ? { src, url } : null;
  });
  const [failedSrc, setFailedSrc] = useState<string | null>(null);

  useEffect(() => {
    const cached = imageCache.get(src);
    if (cached) {
      setLoaded((prev) => (prev?.src === src ? prev : { src, url: cached }));
      return;
    }
    let cancelled = false;
    setFailedSrc(null);
    loadGithubImage(src, fetchUrl)
      .then((url) => {
        if (!cancelled) setLoaded({ src, url });
      })
      .catch(() => {
        if (!cancelled) setFailedSrc(src);
      });
    return () => {
      cancelled = true;
    };
  }, [src, fetchUrl]);

  if (failedSrc === src) return <img src={src} alt={alt} />;
  if (loaded?.src !== src) return <span className="image-loading" title={alt ?? src} />;
  return <img src={loaded.url} alt={alt} />;
}

type Hast = ReturnType<typeof highlightToHast>[number];

function renderHast(nodes: Hast[]): ReactNode {
  return nodes.map((n, i) =>
    n.type === "text" ? (
      n.value
    ) : (
      <span key={i} className={n.properties.className.join(" ")}>
        {n.children[0].value}
      </span>
    ),
  );
}

function CodeBlock({ lang, code }: { lang: string | undefined; code: string }) {
  useSyntaxReady(lang);
  let content: ReactNode = code;
  if (lang && isLanguageReady(lang)) {
    try {
      content = renderHast(highlightToHast(code, lang));
    } catch {
      /* plain text */
    }
  }
  return (
    <pre>
      <code>{content}</code>
    </pre>
  );
}

function DiffBlock({ code }: { code: string }) {
  const lines = code.replace(/\n$/, "").split("\n");
  return (
    <pre className="md-diff">
      <code>
        {lines.map((line, i) => {
          const kind = line.startsWith("@@")
            ? "hunk"
            : line.startsWith("+") && !line.startsWith("+++")
              ? "add"
              : line.startsWith("-") && !line.startsWith("---")
                ? "del"
                : "ctx";
          return (
            <span key={i} className={`md-diff-line md-diff-${kind}`}>
              {line || " "}
              {"\n"}
            </span>
          );
        })}
      </code>
    </pre>
  );
}

const svgCache = new Map<string, string>();

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

let colorCtx: CanvasRenderingContext2D | null | undefined;

/** A theme token as plain `rgb()/rgba()`: mermaid's color parser rejects `color-mix()` and friends. */
function cssColor(name: string): string | undefined {
  const raw = cssVar(name);
  if (!raw) return undefined;
  if (colorCtx === undefined) {
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 1;
    colorCtx = canvas.getContext("2d", { willReadFrequently: true });
  }
  const probe = document.createElement("span");
  probe.style.color = `var(${name})`;
  document.body.appendChild(probe);
  const resolved = getComputedStyle(probe).color;
  probe.remove();
  if (!colorCtx) return resolved || undefined;
  colorCtx.clearRect(0, 0, 1, 1);
  colorCtx.fillStyle = "#000";
  colorCtx.fillStyle = resolved;
  colorCtx.fillRect(0, 0, 1, 1);
  const [r, g, b, a] = colorCtx.getImageData(0, 0, 1, 1).data;
  return a === 255 ? `rgb(${r}, ${g}, ${b})` : `rgba(${r}, ${g}, ${b}, ${+(a / 255).toFixed(3)})`;
}

function MermaidBlock({ code }: { code: string }) {
  const id = `mmd-${useId().replace(/[^a-zA-Z0-9]/g, "")}`;
  const [svg, setSvg] = useState<string | null>(() => svgCache.get(code) ?? null);
  const [error, setError] = useState<string | null>(null);
  const [showCode, setShowCode] = useState(false);

  useEffect(() => {
    if (svgCache.has(code)) {
      setSvg(svgCache.get(code)!);
      return;
    }
    let cancelled = false;
    import("mermaid")
      .then(async ({ default: mermaid }) => {
        mermaid.initialize({
          startOnLoad: false,
          securityLevel: "strict",
          theme: "base",
          themeVariables: {
            fontFamily: cssVar("--font-sans") || undefined,
            primaryColor: cssColor("--surface-2"),
            primaryTextColor: cssColor("--text"),
            primaryBorderColor: cssColor("--border"),
            lineColor: cssColor("--text-2"),
            secondaryColor: cssColor("--surface-3"),
            tertiaryColor: cssColor("--bg-sunken"),
            background: cssColor("--bg-sunken"),
          },
        });
        const { svg } = await mermaid.render(id, code);
        svgCache.set(code, svg);
        if (!cancelled) setSvg(svg);
      })
      .catch((e) => {
        document.getElementById(`d${id}`)?.remove();
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [code, id]);

  const codeView = (
    <pre>
      <code>{code}</code>
    </pre>
  );
  if (error) {
    return (
      <div className="md-mermaid">
        <span className="md-mermaid-error">Mermaid error: {error.split("\n")[0]}</span>
        {codeView}
      </div>
    );
  }
  if (!svg) return <div className="md-mermaid">{codeView}</div>;
  return (
    <div className="md-mermaid">
      <button className="md-mermaid-toggle" onClick={() => setShowCode((v) => !v)}>
        {showCode ? "Diagram" : "Code"}
      </button>
      {showCode ? codeView : <div dangerouslySetInnerHTML={{ __html: svg }} />}
    </div>
  );
}

const ASSET_UUID_RE = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i;

/**
 * `imageUrls` maps an attachment's asset uuid to a signed URL that loads without
 * SSO (from GitHub's rendered HTML); the original URL stays the cache key.
 */
export function Markdown({
  source,
  imageUrls,
}: {
  source: string;
  imageUrls?: Record<string, string>;
}) {
  return (
    <div className="markdown-body">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[rehypeRaw, [rehypeSanitize, SANITIZE_SCHEMA]]}
        components={{
          a: ({ href, children }) => (
            <a
              href={href}
              onClick={(e) => {
                e.preventDefault();
                if (href) api.openUrl(href);
              }}
            >
              {children}
            </a>
          ),
          img: ({ src, alt }) => {
            if (src && GITHUB_IMAGE_RE.test(src)) {
              const uuid = ASSET_UUID_RE.exec(src)?.[0].toLowerCase();
              return <ProxiedImage src={src} alt={alt} fetchUrl={uuid ? imageUrls?.[uuid] : undefined} />;
            }
            return <img src={src} alt={alt} />;
          },
          pre: ({ children, ...props }) => {
            const child = Array.isArray(children) ? children[0] : children;
            if (
              isValidElement(child) &&
              typeof (child.props as { className?: string }).className === "string" &&
              (child.props as { className: string }).className.includes(
                "language-suggestion",
              )
            ) {
              const code = (child.props as { children?: React.ReactNode }).children;
              const empty =
                code == null || (typeof code === "string" && code.trim() === "");
              return (
                <div className="suggestion-block">
                  <div className="suggestion-block-header">Suggested change</div>
                  {empty ? (
                    <p className="muted suggestion-block-empty">
                      (removes the selected lines)
                    </p>
                  ) : (
                    <pre className="suggestion-block-new">
                      <code>{code}</code>
                    </pre>
                  )}
                </div>
              );
            }
            if (isValidElement(child)) {
              const { className, children: text } = child.props as {
                className?: string;
                children?: ReactNode;
              };
              const info = /language-(\S+)/.exec(className ?? "")?.[1] ?? "";
              const code = typeof text === "string" ? text : "";
              if (info === "mermaid") return <MermaidBlock code={code} />;
              if (info === "diff" || info === "patch") return <DiffBlock code={code} />;
              if (info) return <CodeBlock lang={languageForFence(info)} code={code} />;
            }
            return <pre {...props}>{children}</pre>;
          },
        }}
      >
        {source}
      </ReactMarkdown>
    </div>
  );
}
