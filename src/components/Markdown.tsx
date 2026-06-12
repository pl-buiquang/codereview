import { isValidElement } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { api } from "../lib/api";

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
