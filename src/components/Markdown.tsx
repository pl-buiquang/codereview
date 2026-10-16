import { isValidElement, useEffect, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { api } from "../lib/api";

const GITHUB_IMAGE_RE =
  /^https:\/\/(?:github\.com\/.*\/assets\/|user-images\.githubusercontent\.com\/|private-user-images\.githubusercontent\.com\/|github\.com\/user-attachments\/)/;

const MAX_CACHED_IMAGES = 200;
const imageCache = new Map<string, string>();
const imageInflight = new Map<string, Promise<string>>();

/** Proxied GitHub images by URL, shared across mounts; failures aren't cached. */
function loadGithubImage(src: string): Promise<string> {
  const cached = imageCache.get(src);
  if (cached) return Promise.resolve(cached);
  let pending = imageInflight.get(src);
  if (!pending) {
    pending = api
      .fetchGithubImage(src)
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

function ProxiedImage({ src, alt }: { src: string; alt?: string }) {
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
    loadGithubImage(src)
      .then((url) => {
        if (!cancelled) setLoaded({ src, url });
      })
      .catch(() => {
        if (!cancelled) setFailedSrc(src);
      });
    return () => {
      cancelled = true;
    };
  }, [src]);

  if (failedSrc === src) return <img src={src} alt={alt} />;
  if (loaded?.src !== src) return <span className="image-loading" title={alt ?? src} />;
  return <img src={loaded.url} alt={alt} />;
}

export function Markdown({ source }: { source: string }) {
  return (
    <div className="markdown-body">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
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
              return <ProxiedImage src={src} alt={alt} />;
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
            return <pre {...props}>{children}</pre>;
          },
        }}
      >
        {source}
      </ReactMarkdown>
    </div>
  );
}
