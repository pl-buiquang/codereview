//! The model ids a profile editor offers per harness: what the harness' CLI lists (`opencode
//! models`), grouped by provider, cached until an explicit refresh.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;

use super::harness_by_id;
use crate::error::{AppError, AppResult};

/// The models of one provider (the part of the id before the first `/`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ModelGroup {
    pub provider: String,
    pub models: Vec<String>,
}

/// Group `ids` by provider, in order of first appearance. An id without a `/` goes in a group named
/// `""` (the editor shows it as "Suggestions").
pub fn group(ids: &[String]) -> Vec<ModelGroup> {
    let mut groups: Vec<ModelGroup> = Vec::new();
    for id in ids {
        let provider = id.split_once('/').map(|(p, _)| p).unwrap_or("");
        match groups.iter_mut().find(|g| g.provider == provider) {
            Some(g) => g.models.push(id.clone()),
            None => groups.push(ModelGroup { provider: provider.to_string(), models: vec![id.clone()] }),
        }
    }
    groups
}

static CACHE: Mutex<Option<HashMap<String, Vec<String>>>> = Mutex::new(None);

/// The models of harness `id`, provider-grouped. `refresh` runs the CLI again; otherwise a previous
/// non-empty answer is reused. An empty list is not cached (a provider may be added meanwhile).
pub fn for_harness(id: &str, refresh: bool) -> AppResult<Vec<ModelGroup>> {
    let harness = harness_by_id(id).ok_or_else(|| AppError::Other(format!("unknown harness `{id}`")))?;
    if !refresh {
        if let Some(ids) = CACHE.lock().unwrap().as_ref().and_then(|c| c.get(id)) {
            return Ok(group(ids));
        }
    }
    let ids = harness.models()?;
    let mut cache = CACHE.lock().unwrap();
    let cache = cache.get_or_insert_with(HashMap::new);
    if ids.is_empty() {
        cache.remove(id);
    } else {
        cache.insert(id.to_string(), ids.clone());
    }
    Ok(group(&ids))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn ids_are_grouped_by_provider_in_order_of_appearance() {
        let groups = group(&ids(&["opencode/a", "openrouter/x/y", "opencode/b", "sonnet"]));
        assert_eq!(
            groups,
            vec![
                ModelGroup { provider: "opencode".into(), models: ids(&["opencode/a", "opencode/b"]) },
                ModelGroup { provider: "openrouter".into(), models: ids(&["openrouter/x/y"]) },
                ModelGroup { provider: "".into(), models: ids(&["sonnet"]) },
            ]
        );
        assert!(group(&[]).is_empty());
    }

    #[test]
    fn claude_gets_a_static_suggestion_list() {
        let groups = for_harness("claude", false).unwrap();
        let all: Vec<_> = groups.iter().flat_map(|g| g.models.clone()).collect();
        assert!(all.contains(&"sonnet".to_string()) && all.contains(&"opus".to_string()));
    }

    #[test]
    fn an_unknown_harness_is_an_error() {
        assert!(for_harness("nope", false).is_err());
    }
}
