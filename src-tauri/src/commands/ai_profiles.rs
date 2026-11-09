//! Tauri commands for AI profiles (Settings → AI): profile CRUD, the feature to profile table and
//! the harness availability panel.

use serde::Serialize;
use tauri::State;

use crate::ai::profiles::{self, AiProfile, AiProfileInput, FeatureProfiles};
use crate::ai::{self, Caps};
use crate::db::Db;
use crate::error::{AppError, AppResult};

#[tauri::command]
pub fn ai_profile_list(db: State<Db>) -> AppResult<Vec<AiProfile>> {
    profiles::list(&db.0.lock().unwrap())
}

#[tauri::command]
pub fn ai_profile_save(profile: AiProfileInput, db: State<Db>) -> AppResult<AiProfile> {
    profiles::save(&mut db.0.lock().unwrap(), profile)
}

#[tauri::command]
pub fn ai_profile_delete(id: i64, db: State<Db>) -> AppResult<()> {
    profiles::delete(&mut db.0.lock().unwrap(), id)
}

#[tauri::command]
pub fn ai_feature_profiles_get(db: State<Db>) -> FeatureProfiles {
    profiles::feature_profiles(&db.0.lock().unwrap())
}

#[tauri::command]
pub fn ai_feature_profiles_set(map: FeatureProfiles, db: State<Db>) -> AppResult<()> {
    profiles::set_feature_profiles(&mut db.0.lock().unwrap(), &map)
}

/// One-time import of the chat / overview model fields of the old frontend settings.
#[tauri::command]
pub fn ai_import_legacy_models(chat: String, overview: String, db: State<Db>) -> AppResult<bool> {
    profiles::import_legacy_models(&mut db.0.lock().unwrap(), &chat, &overview)
}

#[derive(Debug, Serialize)]
pub struct HarnessInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub caps: Caps,
    /// The harness' binary was found.
    pub available: bool,
    pub version: Option<String>,
}

fn harness_infos() -> Vec<HarnessInfo> {
    ai::harnesses()
        .into_iter()
        .map(|h| {
            let available = h.available();
            HarnessInfo { id: h.id(), name: h.name(), caps: h.caps(), available, version: available.then(|| h.version()).flatten() }
        })
        .collect()
}

/// The known harnesses with their capabilities, availability and version.
#[tauri::command]
pub async fn ai_harnesses() -> AppResult<Vec<HarnessInfo>> {
    tauri::async_runtime::spawn_blocking(harness_infos)
        .await
        .map_err(|e| AppError::Other(format!("harness probe failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_infos_list_claude_with_caps() {
        let infos = harness_infos();
        let claude = infos.iter().find(|i| i.id == "claude").unwrap();
        assert_eq!(claude.name, "Claude Code");
        assert!(claude.caps.tool_allowlist);
        let json = serde_json::to_value(claude).unwrap();
        assert_eq!(json["caps"]["access_levels"], serde_json::json!(["default", "read_only", "edit", "full"]));
    }
}
