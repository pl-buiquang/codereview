//! AI profiles: a named harness + model + settings, selected per feature or per routine. This
//! module stores them (`ai_profile`) and the feature to profile mapping (`meta` rows
//! `ai_profile.<feature>`). Profile settings never hold a secret: every CLI keeps its own auth.

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{harness_by_id, harness_for, Harness};
use crate::error::{AppError, AppResult};

/// A configured way of running an AI harness.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AiProfile {
    pub id: i64,
    pub name: String,
    pub harness: String,
    /// Empty: the harness picks its own default.
    pub model: String,
    /// Harness-specific options (a Codex sandbox, an opencode agent), a JSON object.
    pub settings: Value,
    /// The remote host that runs it; `None` runs locally.
    pub host_id: Option<i64>,
    pub is_default: bool,
}

impl AiProfile {
    fn from_row(row: &Row) -> rusqlite::Result<Self> {
        let settings: String = row.get("settings")?;
        Ok(Self {
            id: row.get("id")?,
            name: row.get("name")?,
            harness: row.get("harness")?,
            model: row.get("model")?,
            settings: serde_json::from_str(&settings).unwrap_or_else(|_| json!({})),
            host_id: row.get("host_id")?,
            is_default: row.get::<_, i64>("is_default")? != 0,
        })
    }
}

/// What the Settings editor sends. `id: None` creates a profile.
#[derive(Debug, Deserialize)]
pub struct AiProfileInput {
    pub id: Option<i64>,
    pub name: String,
    pub harness: String,
    #[serde(default)]
    pub model: String,
    #[serde(default = "empty_object")]
    pub settings: Value,
    #[serde(default)]
    pub host_id: Option<i64>,
    #[serde(default)]
    pub is_default: bool,
}

fn empty_object() -> Value {
    json!({})
}

/// The features that pick a profile. Routines pick theirs through `routine.profile_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feature {
    Chat,
    /// Live session summaries.
    Summaries,
    Theme,
    /// Interactive session launches.
    Sessions,
}

impl Feature {
    pub const ALL: [Feature; 4] = [Feature::Chat, Feature::Summaries, Feature::Theme, Feature::Sessions];

    pub fn key(self) -> &'static str {
        match self {
            Feature::Chat => "chat",
            Feature::Summaries => "summaries",
            Feature::Theme => "theme",
            Feature::Sessions => "sessions",
        }
    }

    fn meta_key(self) -> String {
        format!("ai_profile.{}", self.key())
    }
}

/// The feature to profile table of Settings → AI.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FeatureProfiles {
    pub chat: Option<i64>,
    pub summaries: Option<i64>,
    pub theme: Option<i64>,
    pub sessions: Option<i64>,
}

impl FeatureProfiles {
    fn slot(&mut self, feature: Feature) -> &mut Option<i64> {
        match feature {
            Feature::Chat => &mut self.chat,
            Feature::Summaries => &mut self.summaries,
            Feature::Theme => &mut self.theme,
            Feature::Sessions => &mut self.sessions,
        }
    }

    pub fn get(&self, feature: Feature) -> Option<i64> {
        match feature {
            Feature::Chat => self.chat,
            Feature::Summaries => self.summaries,
            Feature::Theme => self.theme,
            Feature::Sessions => self.sessions,
        }
    }
}

pub fn list(conn: &Connection) -> AppResult<Vec<AiProfile>> {
    let mut stmt = conn.prepare("SELECT * FROM ai_profile ORDER BY is_default DESC, name COLLATE NOCASE, id")?;
    let rows = stmt.query_map([], AiProfile::from_row)?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

pub fn get(conn: &Connection, id: i64) -> AppResult<Option<AiProfile>> {
    Ok(conn.query_row("SELECT * FROM ai_profile WHERE id = ?1", params![id], AiProfile::from_row).optional()?)
}

pub fn default_profile(conn: &Connection) -> AppResult<Option<AiProfile>> {
    Ok(conn.query_row("SELECT * FROM ai_profile WHERE is_default = 1", [], AiProfile::from_row).optional()?)
}

/// Keys whose name suggests a credential: profile settings are plain options.
const SECRET_HINTS: &[&str] = &["token", "secret", "password", "passwd", "api_key", "apikey", "credential"];

fn reject_secrets(settings: &Value) -> AppResult<()> {
    if let Value::Object(map) = settings {
        for (key, value) in map {
            let lower = key.to_lowercase();
            if SECRET_HINTS.iter().any(|h| lower.contains(h)) {
                return Err(AppError::Other(format!(
                    "profile setting `{key}` looks like a secret; the app stores none (each CLI keeps its own auth)"
                )));
            }
            reject_secrets(value)?;
        }
    }
    Ok(())
}

pub fn save(conn: &mut Connection, input: AiProfileInput) -> AppResult<AiProfile> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::Other("a profile needs a name".into()));
    }
    if harness_by_id(&input.harness).is_none() {
        return Err(AppError::Other(format!("unknown harness `{}`", input.harness)));
    }
    if !input.settings.is_object() {
        return Err(AppError::Other("profile settings must be a JSON object".into()));
    }
    reject_secrets(&input.settings)?;
    if let Some(h) = harness_by_id(&input.harness) {
        h.validate_settings(&input.settings)?;
    }
    if let Some(host_id) = input.host_id {
        crate::remote::host::require(conn, host_id)?;
    }
    let settings = serde_json::to_string(&input.settings).unwrap_or_else(|_| "{}".into());
    let model = input.model.trim().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let tx = conn.transaction()?;
    let id = match input.id {
        Some(id) => {
            let changed = tx.execute(
                "UPDATE ai_profile SET name = ?2, harness = ?3, model = ?4, settings = ?5, host_id = ?6, updated_at = ?7
                 WHERE id = ?1",
                params![id, name, input.harness, model, settings, input.host_id, now],
            )?;
            if changed == 0 {
                return Err(AppError::Other(format!("profile {id} not found")));
            }
            id
        }
        None => {
            tx.execute(
                "INSERT INTO ai_profile (name, harness, model, settings, host_id, is_default, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?6)",
                params![name, input.harness, model, settings, input.host_id, now],
            )?;
            tx.last_insert_rowid()
        }
    };
    // The default can be moved to another profile but never unset: only the one flag holder is "the" default.
    let has_default: bool = tx.query_row("SELECT COUNT(*) FROM ai_profile WHERE is_default = 1", [], |r| r.get::<_, i64>(0))? > 0;
    if input.is_default || !has_default {
        tx.execute("UPDATE ai_profile SET is_default = 0 WHERE is_default = 1", [])?;
        tx.execute("UPDATE ai_profile SET is_default = 1 WHERE id = ?1", params![id])?;
    }
    tx.commit()?;
    get(conn, id)?.ok_or_else(|| AppError::Other("profile vanished".into()))
}

/// Delete a profile. Routines and chats pinned to it fall back (`ON DELETE SET NULL`); a feature
/// mapped to it is unmapped. The default profile can't be deleted: make another one the default first.
pub fn delete(conn: &mut Connection, id: i64) -> AppResult<()> {
    let profile = get(conn, id)?.ok_or_else(|| AppError::Other(format!("profile {id} not found")))?;
    if profile.is_default {
        return Err(AppError::Other("the default profile can't be deleted: make another profile the default first".into()));
    }
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM ai_profile WHERE id = ?1", params![id])?;
    for feature in Feature::ALL {
        tx.execute("DELETE FROM meta WHERE key = ?1 AND value = ?2", params![feature.meta_key(), id.to_string()])?;
    }
    tx.commit()?;
    Ok(())
}

pub fn feature_profiles(conn: &Connection) -> FeatureProfiles {
    let mut out = FeatureProfiles::default();
    for feature in Feature::ALL {
        let id = conn
            .query_row("SELECT value FROM meta WHERE key = ?1", params![feature.meta_key()], |r| r.get::<_, String>(0))
            .ok()
            .and_then(|v| v.parse::<i64>().ok());
        *out.slot(feature) = id;
    }
    out
}

pub fn set_feature_profiles(conn: &mut Connection, map: &FeatureProfiles) -> AppResult<()> {
    let tx = conn.transaction()?;
    for feature in Feature::ALL {
        match map.get(feature) {
            Some(id) => {
                let exists: bool = tx.query_row("SELECT COUNT(*) FROM ai_profile WHERE id = ?1", params![id], |r| r.get::<_, i64>(0))? > 0;
                if !exists {
                    return Err(AppError::Other(format!("profile {id} not found")));
                }
                tx.execute(
                    "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![feature.meta_key(), id.to_string()],
                )?;
            }
            None => {
                tx.execute("DELETE FROM meta WHERE key = ?1", params![feature.meta_key()])?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

/// What a feature or routine runs with: the harness plus the profile's model and settings.
pub struct Resolved {
    /// The profile it came from; `None` when none exists (Claude Code with the legacy resolution).
    pub profile_id: Option<i64>,
    pub harness: &'static dyn Harness,
    /// The profile's model; empty means the harness' own default.
    pub model: String,
    pub settings: Value,
    pub host_id: Option<i64>,
    /// Picked by a routine or a feature mapping rather than the global default.
    pub explicit: bool,
}

impl Resolved {
    /// No profile at all: Claude Code, empty model.
    fn legacy() -> Self {
        Self { profile_id: None, harness: harness_for(), model: String::new(), settings: json!({}), host_id: None, explicit: false }
    }

    fn from_profile(p: AiProfile, harness: &'static dyn Harness, explicit: bool) -> Self {
        // A profile with a host runs that harness on the host.
        let harness = match p.host_id {
            Some(host) => crate::remote::harness::for_host(host, harness.id()).unwrap_or(harness),
            None => harness,
        };
        Self { profile_id: Some(p.id), harness, model: p.model, settings: p.settings, host_id: p.host_id, explicit }
    }

    /// Refuse a profile that runs on a remote host for work that can only happen on this machine
    /// (`what` names it: system routines, interactive sessions, multi-turn chats).
    pub fn ensure_local(&self, what: &str) -> AppResult<()> {
        match self.host_id {
            None => Ok(()),
            Some(_) => Err(AppError::Other(format!(
                "{what} runs on this machine only, but its profile runs on a remote host: pick a profile without a host"
            ))),
        }
    }

    /// The model of a request: `requested` (a launch override) wins, then the profile's. Claude
    /// Code keeps its env-based fallback so Bedrock setups that rely on it still work.
    pub fn model_for(&self, requested: &str) -> String {
        let requested = requested.trim();
        let model = if requested.is_empty() { self.model.as_str() } else { requested };
        // The env fallback is this machine's; a remote Claude has its own.
        if self.harness.id() == "claude" && self.host_id.is_none() {
            resolve_model(Some(model.to_string()))
        } else {
            model.to_string()
        }
    }

    /// The model of a feature that ships its own cheap default (`builtin`, a Claude model name): used
    /// unless the feature was mapped to a profile that sets a model. The global default profile does
    /// not apply, so choosing an expensive default for chat never makes summaries expensive.
    pub fn model_or_builtin(&self, builtin: &str) -> String {
        if self.explicit && !self.model.is_empty() {
            self.model.clone()
        } else if self.harness.id() == "claude" {
            builtin.to_string()
        } else {
            String::new()
        }
    }
}

/// The model of a Claude run when nothing else names one:
/// 1. the explicit model (profile, routine or launch override)
/// 2. `CODEREVIEW_CHAT_MODEL` (manual override)
/// 3. `ANTHROPIC_DEFAULT_OPUS_MODEL` (Bedrock ARN etc.)
/// 4. empty: no `--model` flag, `claude` uses its own configured default
pub fn resolve_model(model: Option<String>) -> String {
    model
        .filter(|m| !m.is_empty())
        .or_else(|| std::env::var("CODEREVIEW_CHAT_MODEL").ok())
        .or_else(|| std::env::var("ANTHROPIC_DEFAULT_OPUS_MODEL").ok())
        .unwrap_or_default()
}

/// The first usable profile among `candidates` (earlier wins), else the default profile, else
/// Claude Code with the legacy resolution. A missing or deleted profile, or one whose harness this
/// build doesn't know, is skipped.
pub fn resolve(conn: &Connection, candidates: &[Option<i64>]) -> Resolved {
    let usable = |p: AiProfile| harness_by_id(&p.harness).map(|h| (p, h));
    for id in candidates.iter().flatten() {
        if let Some((p, h)) = get(conn, *id).ok().flatten().and_then(usable) {
            return Resolved::from_profile(p, h, true);
        }
    }
    match default_profile(conn).ok().flatten().and_then(usable) {
        Some((p, h)) => Resolved::from_profile(p, h, false),
        None => Resolved::legacy(),
    }
}

/// A feature's profile: its mapping, else the default.
pub fn resolve_feature(conn: &Connection, feature: Feature) -> Resolved {
    resolve(conn, &[feature_profiles(conn).get(feature)])
}

/// A routine's profile: its own, else the default.
pub fn resolve_routine(conn: &Connection, profile_id: Option<i64>) -> Resolved {
    resolve(conn, &[profile_id])
}

const LEGACY_IMPORTED: &str = "ai_legacy_models_imported";

/// One-time import of the chat and board-overview model fields that used to live in the frontend
/// settings: the chat model goes into the default profile, a different overview model into its own
/// profile on the `board_overview` routine, so every run keeps the model it had. Returns whether
/// anything was imported (false when it already ran).
pub fn import_legacy_models(conn: &mut Connection, chat: &str, overview: &str) -> AppResult<bool> {
    let done: Option<String> =
        conn.query_row("SELECT value FROM meta WHERE key = ?1", params![LEGACY_IMPORTED], |r| r.get(0)).optional()?;
    if done.is_some() {
        return Ok(false);
    }
    let (chat, overview) = (chat.trim(), overview.trim());
    let now = chrono::Utc::now().to_rfc3339();
    let tx = conn.transaction()?;
    if !chat.is_empty() {
        tx.execute("UPDATE ai_profile SET model = ?1, updated_at = ?2 WHERE is_default = 1 AND model = ''", params![chat, now])?;
    }
    if !overview.is_empty() && overview != chat {
        let routine: Option<i64> = tx
            .query_row("SELECT id FROM routine WHERE key = 'board_overview' AND profile_id IS NULL AND model = ''", [], |r| r.get(0))
            .optional()?;
        if let Some(routine) = routine {
            tx.execute(
                "INSERT INTO ai_profile (name, harness, model, settings, is_default, created_at, updated_at)
                 VALUES (?1, 'claude', ?2, '{}', 0, ?3, ?3)",
                params![format!("Claude Code ({overview})"), overview, now],
            )?;
            let profile = tx.last_insert_rowid();
            tx.execute("UPDATE routine SET profile_id = ?1 WHERE id = ?2", params![profile, routine])?;
        }
    }
    tx.execute("INSERT INTO meta (key, value) VALUES (?1, '1')", params![LEGACY_IMPORTED])?;
    tx.commit()?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;

    fn input(name: &str) -> AiProfileInput {
        AiProfileInput {
            id: None,
            name: name.into(),
            harness: "claude".into(),
            model: "sonnet".into(),
            settings: json!({}),
            host_id: None,
            is_default: false,
        }
    }

    #[test]
    fn migration_seeds_a_default_claude_profile() {
        let conn = open_memory();
        let all = list(&conn).unwrap();
        assert_eq!(all.len(), 1);
        assert!(all[0].is_default);
        assert_eq!((all[0].harness.as_str(), all[0].model.as_str()), ("claude", ""));
    }

    #[test]
    fn save_creates_updates_and_moves_the_default() {
        let mut conn = open_memory();
        let a = save(&mut conn, input("Fast")).unwrap();
        assert!(!a.is_default);
        assert_eq!(a.model, "sonnet");

        let mut edit = input("Fast!");
        edit.id = Some(a.id);
        edit.model = "haiku".into();
        edit.is_default = true;
        let a = save(&mut conn, edit).unwrap();
        assert_eq!((a.name.as_str(), a.model.as_str(), a.is_default), ("Fast!", "haiku", true));
        let defaults: Vec<_> = list(&conn).unwrap().into_iter().filter(|p| p.is_default).collect();
        assert_eq!(defaults.len(), 1, "exactly one default");

        // Saving the default with is_default = false doesn't unset it.
        let mut again = input("Fast!");
        again.id = Some(a.id);
        assert!(save(&mut conn, again).unwrap().is_default);
    }

    #[test]
    fn save_validates() {
        let mut conn = open_memory();
        assert!(save(&mut conn, input("  ")).is_err());
        let mut bad = input("x");
        bad.harness = "nope".into();
        assert!(save(&mut conn, bad).unwrap_err().to_string().contains("unknown harness"));
        let mut arr = input("x");
        arr.settings = json!([1]);
        assert!(save(&mut conn, arr).is_err());
        let mut secret = input("x");
        secret.settings = json!({ "env": { "OPENAI_API_KEY": "sk-1" } });
        assert!(save(&mut conn, secret).unwrap_err().to_string().contains("secret"));
        let mut unsafe_args = input("x");
        unsafe_args.harness = "codex".into();
        unsafe_args.settings = json!({ "extra_args": ["-c", "\"sandbox_mode\"=\"danger-full-access\""] });
        assert!(save(&mut conn, unsafe_args).unwrap_err().to_string().contains("extra_args may not contain"));
        let mut missing = input("x");
        missing.id = Some(999);
        assert!(save(&mut conn, missing).is_err());
    }

    #[test]
    fn a_profile_may_only_point_at_a_host_that_exists() {
        let mut conn = open_memory();
        let mut remote = input("Remote");
        remote.host_id = Some(42);
        assert!(save(&mut conn, remote).unwrap_err().to_string().contains("host 42 not found"));
        let host = crate::remote::host::save(
            &conn,
            crate::remote::host::HostInput { id: None, name: "koza".into(), ssh_target: "koza".into(), remote_endpoint: "7777".into(), enabled: true },
        )
        .unwrap();
        let mut remote = input("Remote");
        remote.host_id = Some(host.id);
        assert_eq!(save(&mut conn, remote).unwrap().host_id, Some(host.id));
    }

    #[test]
    fn delete_unpins_routines_and_chats_and_unmaps_features() {
        let mut conn = open_memory();
        let p = save(&mut conn, input("Temp")).unwrap();
        conn.execute(
            "INSERT INTO routine (key, name, context_type, prompt_template, profile_id, created_at, updated_at)
             VALUES ('k', 'k', 'pr', 'p', ?1, 'n', 'n')",
            params![p.id],
        )
        .unwrap();
        set_feature_profiles(&mut conn, &FeatureProfiles { chat: Some(p.id), theme: Some(p.id), ..Default::default() }).unwrap();

        delete(&mut conn, p.id).unwrap();

        assert!(get(&conn, p.id).unwrap().is_none());
        let pinned: Option<i64> = conn.query_row("SELECT profile_id FROM routine WHERE key = 'k'", [], |r| r.get(0)).unwrap();
        assert_eq!(pinned, None);
        assert_eq!(feature_profiles(&conn), FeatureProfiles::default());
    }

    #[test]
    fn the_default_profile_cannot_be_deleted() {
        let mut conn = open_memory();
        let default = default_profile(&conn).unwrap().unwrap();
        assert!(delete(&mut conn, default.id).is_err());
        assert!(delete(&mut conn, 999).is_err());
    }

    #[test]
    fn feature_profiles_round_trip_and_reject_unknown_profiles() {
        let mut conn = open_memory();
        let p = save(&mut conn, input("A")).unwrap();
        let map = FeatureProfiles { chat: Some(p.id), summaries: None, theme: None, sessions: Some(p.id) };
        set_feature_profiles(&mut conn, &map).unwrap();
        assert_eq!(feature_profiles(&conn), map);

        set_feature_profiles(&mut conn, &FeatureProfiles::default()).unwrap();
        assert_eq!(feature_profiles(&conn), FeatureProfiles::default());

        let bad = FeatureProfiles { chat: Some(999), ..Default::default() };
        assert!(set_feature_profiles(&mut conn, &bad).is_err());
    }

    #[test]
    fn legacy_models_are_imported_once() {
        let mut conn = open_memory();
        crate::routines::seed(&conn).unwrap();
        assert!(import_legacy_models(&mut conn, "opus", "sonnet").unwrap());
        let default = default_profile(&conn).unwrap().unwrap();
        assert_eq!(default.model, "opus");
        let overview_profile: Option<i64> =
            conn.query_row("SELECT profile_id FROM routine WHERE key = 'board_overview'", [], |r| r.get(0)).unwrap();
        let overview = get(&conn, overview_profile.unwrap()).unwrap().unwrap();
        assert_eq!((overview.harness.as_str(), overview.model.as_str(), overview.is_default), ("claude", "sonnet", false));

        // A second call (another webview start) changes nothing.
        assert!(!import_legacy_models(&mut conn, "haiku", "haiku").unwrap());
        assert_eq!(default_profile(&conn).unwrap().unwrap().model, "opus");
        assert_eq!(list(&conn).unwrap().len(), 2);
    }

    #[test]
    fn legacy_overview_equal_to_chat_or_empty_adds_no_profile() {
        let mut conn = open_memory();
        crate::routines::seed(&conn).unwrap();
        import_legacy_models(&mut conn, "opus", "opus").unwrap();
        assert_eq!(list(&conn).unwrap().len(), 1);

        let mut conn = open_memory();
        crate::routines::seed(&conn).unwrap();
        import_legacy_models(&mut conn, "", "").unwrap();
        assert_eq!(list(&conn).unwrap().len(), 1);
        assert_eq!(default_profile(&conn).unwrap().unwrap().model, "");
    }

    fn named(conn: &mut Connection, name: &str, model: &str) -> i64 {
        let mut i = input(name);
        i.model = model.into();
        save(conn, i).unwrap().id
    }

    #[test]
    fn resolution_order_is_routine_then_feature_then_default() {
        let mut conn = open_memory();
        let default = default_profile(&conn).unwrap().unwrap().id;
        let routine = named(&mut conn, "R", "haiku");
        let chat = named(&mut conn, "C", "opus");
        set_feature_profiles(&mut conn, &FeatureProfiles { chat: Some(chat), ..Default::default() }).unwrap();

        // The routine's own profile beats the feature's, which beats the default.
        let r = resolve(&conn, &[Some(routine), Some(chat)]);
        assert_eq!((r.profile_id, r.model.as_str(), r.explicit), (Some(routine), "haiku", true));
        assert_eq!(resolve_feature(&conn, Feature::Chat).profile_id, Some(chat));
        // A feature with no mapping, and a routine with no profile, get the default.
        let t = resolve_feature(&conn, Feature::Theme);
        assert_eq!((t.profile_id, t.explicit), (Some(default), false));
        assert_eq!(resolve_routine(&conn, None).profile_id, Some(default));
    }

    #[test]
    fn a_profile_with_a_host_resolves_to_the_remote_harness_and_refuses_local_only_work() {
        let mut conn = open_memory();
        let host = crate::remote::host::save(
            &conn,
            crate::remote::host::HostInput { id: None, name: "koza".into(), ssh_target: "koza".into(), remote_endpoint: "7777".into(), enabled: true },
        )
        .unwrap();
        let mut remote = input("Remote");
        remote.host_id = Some(host.id);
        remote.model = "opus".into();
        let id = save(&mut conn, remote).unwrap().id;
        let r = resolve_routine(&conn, Some(id));
        assert_eq!((r.host_id, r.harness.id(), r.model_for("").as_str()), (Some(host.id), "claude", "opus"));
        assert!(!r.harness.caps().sessions, "remote harnesses have no sessions");
        let err = r.ensure_local("“Focus”").unwrap_err().to_string();
        assert!(err.contains("“Focus” runs on this machine only"), "{err}");
        let local = resolve_routine(&conn, None);
        local.ensure_local("x").unwrap();
    }

    #[test]
    fn a_deleted_or_missing_profile_falls_back_to_the_default() {
        let mut conn = open_memory();
        let default = default_profile(&conn).unwrap().unwrap().id;
        assert_eq!(resolve_routine(&conn, Some(999)).profile_id, Some(default));
        let gone = named(&mut conn, "Gone", "haiku");
        conn.execute("DELETE FROM ai_profile WHERE id = ?1", [gone]).unwrap();
        assert_eq!(resolve(&conn, &[Some(gone), None]).profile_id, Some(default));
    }

    #[test]
    fn a_profile_of_an_unknown_harness_is_skipped() {
        let conn = open_memory();
        let default = default_profile(&conn).unwrap().unwrap().id;
        conn.execute("INSERT INTO ai_profile (name, harness, created_at, updated_at) VALUES ('Future', 'zed', 'n', 'n')", []).unwrap();
        let future = conn.last_insert_rowid();
        assert_eq!(resolve_routine(&conn, Some(future)).profile_id, Some(default));
    }

    #[test]
    fn no_default_profile_means_claude_with_the_legacy_resolution() {
        let conn = open_memory();
        conn.execute("DELETE FROM ai_profile", []).unwrap();
        let r = resolve_routine(&conn, None);
        assert_eq!((r.profile_id, r.harness.id(), r.model.as_str(), r.explicit), (None, "claude", "", false));
    }

    #[test]
    fn model_for_prefers_the_request_then_the_profile_then_the_env_fallback() {
        let mut conn = open_memory();
        let id = named(&mut conn, "M", "sonnet");
        let r = resolve_routine(&conn, Some(id));
        assert_eq!(r.model_for("opus"), "opus");
        assert_eq!(r.model_for("  "), "sonnet");
        // No model anywhere: the legacy env variables (this test doesn't set them, so it is empty
        // unless the machine does).
        let empty = resolve_routine(&conn, None);
        assert_eq!(empty.model_for(""), resolve_model(None));
    }

    #[test]
    fn builtin_models_apply_unless_the_feature_profile_sets_one() {
        let mut conn = open_memory();
        // The default profile's model never leaks into a feature with a cheap built-in model.
        conn.execute("UPDATE ai_profile SET model = 'opus' WHERE is_default = 1", []).unwrap();
        assert_eq!(resolve_feature(&conn, Feature::Summaries).model_or_builtin("haiku"), "haiku");
        // A feature mapped to a profile with a model uses it; without a model, the built-in.
        let with = named(&mut conn, "With", "sonnet");
        let without = named(&mut conn, "Without", "");
        set_feature_profiles(&mut conn, &FeatureProfiles { summaries: Some(with), theme: Some(without), ..Default::default() }).unwrap();
        assert_eq!(resolve_feature(&conn, Feature::Summaries).model_or_builtin("haiku"), "sonnet");
        assert_eq!(resolve_feature(&conn, Feature::Theme).model_or_builtin("sonnet"), "sonnet");
    }
}
