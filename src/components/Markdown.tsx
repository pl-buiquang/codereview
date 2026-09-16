import { isValidElement, useEffect, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { api } from "../lib/api";

const GITHUB_IMAGE_RE =
  /^https:\/\/(?:github\.com\/.*\/assets\/|user-images\.githubusercontent\.com\/|private-user-images\.githubusercontent\.com\/|github\.com\/user-attachments\/)/;

function ProxiedImage({ src, alt }: { src: string; alt?: string }) {
  const [dataUrl, setDataUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let cancelled = false;
    api
      .fetchGithubImage(src)
      .then((url) => {
        if (!cancelled) setDataUrl(url);
      })
      .catch(() => {
        if (!cancelled) setFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, [src]);

  if (failed) return <img src={src} alt={alt} />;
  if (!dataUrl) return <span className="image-loading" title={alt ?? src} />;
  return <img src={dataUrl} alt={alt} />;
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
