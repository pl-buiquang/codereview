import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";
import { useUIStore } from "../store";
import { useSettingsStore } from "../lib/settings";
import type { AiRun, UsageRange, UsageTotals } from "../lib/types";
import { featureLabel, formatCost, formatDuration, formatTokens, runSubject, shortModel } from "../lib/usage";

const RANGES: { value: UsageRange; label: string }[] = [
  { value: "today", label: "Today" },
  { value: "7d", label: "7 days" },
  { value: "30d", label: "30 days" },
  { value: "all", label: "All time" },
];

function Tile({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="usage-tile" title={hint}>
      <span className="usage-tile-label">{label}</span>
      <span className="usage-tile-value">{value}</span>
    </div>
  );
}

function TotalsCells({ t }: { t: UsageTotals }) {
  return (
    <>
      <td className="num">{t.runs}</td>
      <td className="num">{formatTokens(t.input_tokens)}</td>
      <td className="num">{formatTokens(t.output_tokens)}</td>
      <td className="num">{formatTokens(t.cache_read_tokens + t.cache_creation_tokens)}</td>
      <td className="num">{formatCost(t.cost_usd)}</td>
    </>
  );
}

const totalsHead = (runs: string) => (
  <>
    <th className="num">{runs}</th>
    <th className="num">In</th>
    <th className="num">Out</th>
    <th className="num">Cache</th>
    <th className="num">Cost</th>
  </>
);
const TOTALS_HEAD = totalsHead("Runs");

function formatStarted(iso: string): string {
  return new Date(iso).toLocaleString([], { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

function RunRow({ run }: { run: AiRun }) {
  const openReview = useUIStore((s) => s.openReview);
  const subject = runSubject(run);
  const models = (run.models.length ? run.models : run.requested_model ? [run.requested_model] : []).map(shortModel);
  return (
    <tr>
      <td className="nowrap">{formatStarted(run.started_at)}</td>
      <td className="nowrap">{featureLabel(run.feature, run.variant)}</td>
      <td className="usage-subject" title={run.target_title ?? undefined}>
        {run.review_id != null ? (
          <button className="link-button" onClick={() => openReview(run.review_id!)}>
            {subject}
          </button>
        ) : (
          subject
        )}
      </td>
      <td className="usage-models" title={models.join(", ")}>
        {models.join(", ") || "default"}
      </td>
      <td className="num">{formatTokens(run.input_tokens)}</td>
      <td className="num">{formatTokens(run.output_tokens)}</td>
      <td className="num">{formatTokens(run.cache_read_tokens + run.cache_creation_tokens)}</td>
      <td className="num">{formatCost(run.cost_usd)}</td>
      <td className="num nowrap">{formatDuration(run.duration_ms)}</td>
      <td>{run.status === "error" && <span className="badge badge-draft" title={run.error ?? undefined}>failed</span>}</td>
    </tr>
  );
}

export function UsageSection() {
  const [range, setRange] = useState<UsageRange>("7d");
  const [feature, setFeature] = useState<string | null>(null);

  const summary = useQuery({ queryKey: ["ai-usage", "summary", range], queryFn: () => api.aiUsageSummary(range) });
  const runs = useQuery({ queryKey: ["ai-usage", "runs", range, feature], queryFn: () => api.aiUsageRuns(range, feature) });

  const summaryDir = useSettingsStore((s) => s.sessionSummaryDir);
  const sessions = useQuery({
    queryKey: ["ai-usage", "sessions", range, summaryDir],
    queryFn: () => api.sessionUsageSummary(range, summaryDir),
    enabled: summaryDir.trim() !== "",
  });
  const st = sessions.data;

  const features = [...new Set([...(summary.data?.by_feature.map((f) => f.feature) ?? []), ...(feature ? [feature] : [])])];
  const totals = summary.data?.totals;

  return (
    <div className="settings-section-wide">
      <div className="settings-row">
        <span className="view-toggle">
          {RANGES.map((r) => (
            <button key={r.value} className={range === r.value ? "active" : ""} onClick={() => setRange(r.value)}>
              {r.label}
            </button>
          ))}
        </span>
      </div>

      {summary.isError && <p className="error">Could not load usage: {String(summary.error)}</p>}

      {totals && (
        <section className="settings-group">
          <h3>Totals</h3>
          <div className="usage-tiles">
            <Tile label="Cost" value={formatCost(totals.cost_usd)} hint="API-equivalent price reported by claude" />
            <Tile label="Runs" value={totals.errors ? `${totals.runs} (${totals.errors} failed)` : String(totals.runs)} />
            <Tile label="Input" value={formatTokens(totals.input_tokens)} hint="Uncached input tokens" />
            <Tile label="Output" value={formatTokens(totals.output_tokens)} />
            <Tile
              label="Cache"
              value={formatTokens(totals.cache_read_tokens + totals.cache_creation_tokens)}
              hint={`${totals.cache_read_tokens.toLocaleString()} read · ${totals.cache_creation_tokens.toLocaleString()} written`}
            />
            <Tile label="Run time" value={formatDuration(totals.duration_ms)} />
          </div>
        </section>
      )}

      <section className="settings-group">
        <h3>Terminal sessions</h3>
        {summaryDir.trim() === "" && (
          <p className="muted">Set a session summary folder in Settings to see what your terminal sessions cost.</p>
        )}
        {sessions.isError && <p className="error">Could not load session costs: {String(sessions.error)}</p>}
        {st && (
          <>
            <div className="usage-tiles">
              <Tile label="Cost" value={formatCost(st.totals.cost_usd)} hint="Estimated from transcript tokens by the SessionEnd hook" />
              <Tile label="Sessions" value={String(st.totals.runs)} />
              <Tile label="Main" value={formatCost(st.main_usd)} />
              <Tile label="Subagents" value={formatCost(st.subagents_usd)} />
              <Tile label="Output" value={formatTokens(st.totals.output_tokens)} />
              <Tile
                label="Cache"
                value={formatTokens(st.totals.cache_read_tokens + st.totals.cache_creation_tokens)}
                hint={`${st.totals.cache_read_tokens.toLocaleString()} read · ${st.totals.cache_creation_tokens.toLocaleString()} written`}
              />
            </div>
            {st.by_model.length > 0 && (
              <table className="usage-table">
                <thead>
                  <tr>
                    <th>Model</th>
                    {totalsHead("Sessions")}
                  </tr>
                </thead>
                <tbody>
                  {st.by_model.map((m) => (
                    <tr key={m.model}>
                      <td className="usage-models" title={m.model}>
                        {shortModel(m.model)}
                      </td>
                      <TotalsCells t={m} />
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
            {st.totals.runs === 0 && <p className="muted">No priced sessions in this period.</p>}
          </>
        )}
      </section>

      {summary.data && totals && totals.runs === 0 ? (
        <p className="muted">No AI runs in this period.</p>
      ) : (
        summary.data && (
          <>
            <section className="settings-group">
              <h3>By feature</h3>
              <table className="usage-table">
                <thead>
                  <tr>
                    <th>Feature</th>
                    {TOTALS_HEAD}
                  </tr>
                </thead>
                <tbody>
                  {summary.data.by_feature.map((f) => (
                    <tr key={`${f.feature}/${f.variant ?? ""}`}>
                      <td>{featureLabel(f.feature, f.variant)}</td>
                      <TotalsCells t={f} />
                    </tr>
                  ))}
                </tbody>
              </table>
            </section>

            <section className="settings-group">
              <h3>By model</h3>
              <table className="usage-table">
                <thead>
                  <tr>
                    <th>Model</th>
                    {TOTALS_HEAD}
                  </tr>
                </thead>
                <tbody>
                  {summary.data.by_model.map((m) => (
                    <tr key={m.model}>
                      <td className="usage-models" title={m.model}>
                        {shortModel(m.model)}
                      </td>
                      <TotalsCells t={m} />
                    </tr>
                  ))}
                </tbody>
              </table>
              <p className="settings-hint">A run that used several models counts once under each.</p>
            </section>
          </>
        )
      )}

      <section className="settings-group">
        <div className="settings-row">
          <h3>Recent runs</h3>
          <select
            className="select"
            value={feature ?? ""}
            onChange={(e) => setFeature(e.target.value || null)}
            aria-label="Filter by feature"
          >
            <option value="">All features</option>
            {features.map((f) => (
              <option key={f} value={f}>
                {featureLabel(f)}
              </option>
            ))}
          </select>
        </div>
        {runs.isError && <p className="error">Could not load runs: {String(runs.error)}</p>}
        {runs.data && runs.data.length === 0 && <p className="muted">No runs.</p>}
        {runs.data && runs.data.length > 0 && (
          <div className="usage-table-scroll">
            <table className="usage-table">
              <thead>
                <tr>
                  <th>Started</th>
                  <th>Feature</th>
                  <th>Subject</th>
                  <th>Model</th>
                  <th className="num">In</th>
                  <th className="num">Out</th>
                  <th className="num">Cache</th>
                  <th className="num">Cost</th>
                  <th className="num">Time</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {runs.data.map((r) => (
                  <RunRow key={r.id} run={r} />
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

      <p className="settings-hint">
        Costs are the API-equivalent prices claude reports; on a subscription they are not billed. Only runs the app
        starts itself with <code>claude -p</code> are counted above. Terminal sessions are listed separately, as
        estimated by your SessionEnd hook, dated by their last summary. Codex runs
        (<code>/local-review --codex</code>) are not included. A routine run is logged when it ends, so one still running
        does not appear yet.
      </p>
    </div>
  );
}
