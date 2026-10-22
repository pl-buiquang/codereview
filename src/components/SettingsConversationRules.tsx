import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";
import { DEFAULT_IGNORE_RULES, parseBotLogins, useSettingsStore } from "../lib/settings";
import { EMPTY_CONVERSATION_FILTERS } from "../store";
import type { IgnoreRule } from "../lib/types";
import { Icon } from "./icons";

type Tri = "any" | "yes" | "no";
const toTri = (v: boolean | undefined): Tri => (v === undefined ? "any" : v ? "yes" : "no");
const fromTri = (v: string): boolean | undefined => (v === "any" ? undefined : v === "yes");

function TriSelect({ label, value, onChange }: { label: string; value: boolean | undefined; onChange: (v: boolean | undefined) => void }) {
  return (
    <label className="rule-field">
      <span>{label}</span>
      <select className="input" value={toTri(value)} onChange={(e) => onChange(fromTri(e.target.value))}>
        <option value="any">any</option>
        <option value="yes">yes</option>
        <option value="no">no</option>
      </select>
    </label>
  );
}

/** Drops unset conditions so a rule keeps only what it filters on. */
function patch(rule: IgnoreRule, change: Partial<IgnoreRule>): IgnoreRule {
  const next = { ...rule, ...change } as Record<string, unknown>;
  for (const k of Object.keys(next)) {
    const v = next[k];
    if (v === undefined || v === "" || (Array.isArray(v) && v.length === 0)) delete next[k];
  }
  return next as unknown as IgnoreRule;
}

export function ConversationRulesEditor() {
  const rules = useSettingsStore((s) => s.conversationIgnoreRules);
  const setRules = useSettingsStore((s) => s.setConversationIgnoreRules);
  const botLogins = useSettingsStore((s) => s.botLogins);
  const [editing, setEditing] = useState<string | null>(null);

  const hitsQuery = useQuery({
    queryKey: ["conversations", "rule-hits", botLogins, rules],
    queryFn: () => api.listConversations(EMPTY_CONVERSATION_FILTERS, [...parseBotLogins(botLogins)], rules, 1),
  });
  const hits = hitsQuery.data?.facets.ruleHits ?? {};

  const update = (id: string, change: Partial<IgnoreRule>) =>
    setRules(rules.map((r) => (r.id === id ? patch(r, change) : r)));
  const add = () => {
    const id = `rule-${Date.now()}`;
    setRules([...rules, { id, name: "New rule", enabled: false }]);
    setEditing(id);
  };
  const missingPresets = useMemo(
    () => DEFAULT_IGNORE_RULES.filter((p) => !rules.some((r) => r.id === p.id)),
    [rules],
  );

  return (
    <div className="settings-row settings-row-stack">
      <span>Hide conversations</span>
      <span className="settings-hint muted">
        A conversation is hidden when it matches every condition of an enabled rule. Hidden ones
        can be shown with &quot;Ignored by rules&quot; in the Conversations tab.
      </span>
      <div className="rule-list">
        {rules.map((r) => (
          <div key={r.id} className="rule-item">
            <div className="rule-head">
              <input
                type="checkbox"
                checked={r.enabled}
                title={r.enabled ? "Rule on" : "Rule off"}
                onChange={(e) => update(r.id, { enabled: e.target.checked })}
              />
              <input className="input rule-name" value={r.name} onChange={(e) => update(r.id, { name: e.target.value })} />
              <span className="muted rule-hits" title="Conversations this rule matches now">
                {hits[r.id] ?? 0} match{(hits[r.id] ?? 0) === 1 ? "" : "es"}
              </span>
              <button className="btn btn-sm" onClick={() => setEditing(editing === r.id ? null : r.id)}>
                {editing === r.id ? "Done" : "Edit"}
              </button>
              <button className="btn-icon" title="Delete rule" onClick={() => setRules(rules.filter((x) => x.id !== r.id))}>
                <Icon name="trash" size={13} />
              </button>
            </div>
            {editing === r.id && <RuleConditions rule={r} onChange={(c) => update(r.id, c)} />}
          </div>
        ))}
      </div>
      <div className="rule-actions">
        <button className="btn btn-sm" onClick={add}>
          Add rule
        </button>
        {missingPresets.length > 0 && (
          <button className="btn btn-sm" onClick={() => setRules([...rules, ...missingPresets])}>
            Restore presets
          </button>
        )}
      </div>
    </div>
  );
}

function RuleConditions({ rule, onChange }: { rule: IgnoreRule; onChange: (c: Partial<IgnoreRule>) => void }) {
  return (
    <div className="rule-conditions">
      <label className="rule-field">
        <span>Kind</span>
        <select className="input" value={rule.kind ?? ""} onChange={(e) => onChange({ kind: (e.target.value || undefined) as IgnoreRule["kind"] })}>
          <option value="">any</option>
          <option value="thread">review thread</option>
          <option value="discussion">discussion</option>
        </select>
      </label>
      <TriSelect label="Resolved" value={rule.resolved} onChange={(resolved) => onChange({ resolved })} />
      <TriSelect label="Outdated" value={rule.outdated} onChange={(outdated) => onChange({ outdated })} />
      <label className="rule-field">
        <span>Last comment by</span>
        <select
          className="input"
          value={rule.lastAuthor ?? ""}
          onChange={(e) => onChange({ lastAuthor: (e.target.value || undefined) as IgnoreRule["lastAuthor"] })}
        >
          <option value="">anyone</option>
          <option value="me">me</option>
          <option value="bot">a bot</option>
          <option value="other">someone else</option>
        </select>
      </label>
      <TriSelect label="PR / issue closed or merged" value={rule.subjectClosed} onChange={(subjectClosed) => onChange({ subjectClosed })} />
      <TriSelect label="PR / issue is mine" value={rule.subjectMine} onChange={(subjectMine) => onChange({ subjectMine })} />
      <TriSelect label="I commented in it" value={rule.iCommented} onChange={(iCommented) => onChange({ iCommented })} />
      <TriSelect
        label="I'm involved in the PR / issue (author, direct reviewer, assignee, mentioned, commented, on Board)"
        value={rule.involved}
        onChange={(involved) => onChange({ involved })}
      />
      <TriSelect label="Approvals only (discussion)" value={rule.approvalsOnly} onChange={(approvalsOnly) => onChange({ approvalsOnly })} />
      <label className="rule-field">
        <span>Quiet for at least (days)</span>
        <input
          type="number"
          min={0}
          className="input"
          value={rule.olderThanDays ?? ""}
          onChange={(e) => onChange({ olderThanDays: e.target.value === "" ? undefined : Math.max(0, Number(e.target.value)) })}
        />
      </label>
      <label className="rule-field">
        <span>Last comment contains</span>
        <input className="input" value={rule.bodyContains ?? ""} onChange={(e) => onChange({ bodyContains: e.target.value })} />
      </label>
      <label className="rule-field">
        <span>Last comment author (logins, comma-separated)</span>
        <input
          className="input"
          value={(rule.authors ?? []).join(", ")}
          onChange={(e) =>
            onChange({
              authors: e.target.value
                .split(",")
                .map((x) => x.trim())
                .filter(Boolean),
            })
          }
        />
      </label>
    </div>
  );
}
