import { useEffect, useMemo, useRef } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "../../lib/api";
import { buildTranscript } from "../../lib/routines";
import { Markdown } from "../Markdown";

/** Live transcript of a run, rebuilt from its stored stream-json lines. */
export function RunTranscript({ runId, active }: { runId: number; active: boolean }) {
  const events = useQuery({
    queryKey: ["routine-run-events", runId],
    queryFn: () => api.routineRunEvents(runId, 0),
    // Backstop for the push events; they invalidate this query as lines arrive.
    refetchInterval: active ? 5_000 : false,
  });
  const items = useMemo(() => buildTranscript((events.data ?? []).map((e) => e.json)), [events.data]);
  const endRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (active) endRef.current?.scrollIntoView?.({ block: "end" });
  }, [items.length, active]);

  if (events.isLoading) return <p className="muted">Loading transcript…</p>;
  if (items.length === 0) return <p className="muted faint">{active ? "Waiting for output…" : "No transcript."}</p>;
  return (
    <div className="routine-transcript">
      {items.map((item, i) => {
        if (item.kind === "text") return <Markdown key={i} source={item.text} />;
        if (item.kind === "tool")
          return (
            <details key={i} className={`routine-tool${item.isError ? " error" : ""}`}>
              <summary>
                <code>{item.name}</code>
                <span className="muted routine-tool-arg">{item.input.replace(/\s+/g, " ").slice(0, 100)}</span>
              </summary>
              <pre>{item.input}</pre>
              {item.result != null && <pre className="routine-tool-result">{item.result}</pre>}
            </details>
          );
        return (
          <div key={i} className={`routine-result${item.isError ? " error" : ""}`}>
            <strong>{item.isError ? "Error" : "Result"}</strong>
            <Markdown source={item.text} />
          </div>
        );
      })}
      <div ref={endRef} />
    </div>
  );
}
