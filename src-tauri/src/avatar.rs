use std::sync::{Condvar, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{AppError, AppResult};
use crate::gh::{self, GhRepo};

const HIT_TTL_SECS: i64 = 7 * 24 * 3600;
const MISS_TTL_SECS: i64 = 24 * 3600;
const AVATAR_SIZE: &str = "64";
const MAX_CONCURRENT_GH: usize = 4;

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `Some(entry)` when a fresh cache row exists; the entry itself is `None` for a
/// cached "no GitHub user for this email".
pub fn cached(conn: &Connection, email: &str, now: i64) -> AppResult<Option<Option<String>>> {
    let row: Option<(Option<String>, i64)> = conn
        .query_row(
            "SELECT data_url, fetched_at FROM avatar_cache WHERE email = ?1",
            params![email],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(row.and_then(|(data_url, fetched_at)| {
        let ttl = if data_url.is_some() { HIT_TTL_SECS } else { MISS_TTL_SECS };
        (now - fetched_at < ttl).then_some(data_url)
    }))
}

pub fn store(conn: &Connection, email: &str, data_url: Option<&str>, now: i64) -> AppResult<()> {
    conn.execute(
        "INSERT INTO avatar_cache (email, data_url, fetched_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(email) DO UPDATE SET data_url = excluded.data_url, fetched_at = excluded.fetched_at",
        params![email, data_url, now],
    )?;
    Ok(())
}

enum Noreply {
    Id(String),
    Login(String),
}

fn is_login(s: &str) -> bool {
    !s.is_empty() && s.len() <= 39 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn parse_noreply(email: &str) -> Option<Noreply> {
    let local = email.strip_suffix("@users.noreply.github.com")?;
    match local.split_once('+') {
        Some((id, login)) if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) && is_login(login) => {
            Some(Noreply::Id(id.to_string()))
        }
        None if is_login(local) => Some(Noreply::Login(local.to_string())),
        _ => None,
    }
}

static GH_SLOTS: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());

fn with_gh_slot<T>(f: impl FnOnce() -> T) -> T {
    let (lock, cvar) = &GH_SLOTS;
    let mut used = cvar
        .wait_while(lock.lock().unwrap(), |n| *n >= MAX_CONCURRENT_GH)
        .unwrap();
    *used += 1;
    drop(used);
    let out = f();
    *lock.lock().unwrap() -= 1;
    cvar.notify_one();
    out
}

fn gh_api_string(endpoint: &str, jq: &str) -> AppResult<String> {
    let ctx = GhRepo::Remote { owner: String::new(), name: String::new() };
    let out = with_gh_slot(|| gh::run_gh(&ctx, &["api", endpoint, "--jq", jq]))?;
    Ok(out.trim().to_string())
}

/// Avatar URL for `email`: `Ok(None)` means GitHub has no user linked to it;
/// `Err` is a transient failure that must not be cached.
fn avatar_url(email: &str, sha: &str, owner_name: Option<&(String, String)>) -> AppResult<Option<String>> {
    match parse_noreply(email) {
        Some(Noreply::Id(id)) => return Ok(Some(format!("https://avatars.githubusercontent.com/u/{id}?v=4"))),
        Some(Noreply::Login(login)) => {
            let url = gh_api_string(&format!("users/{login}"), ".avatar_url // \"\"")?;
            return Ok((!url.is_empty()).then_some(url));
        }
        None => {}
    }
    let Some((owner, name)) = owner_name else {
        return Ok(None);
    };
    let url = gh_api_string(
        &format!("repos/{owner}/{name}/commits/{sha}"),
        ".author.avatar_url // \"\"",
    )?;
    Ok((!url.is_empty()).then_some(url))
}

fn sized(url: &str) -> AppResult<String> {
    let mut parsed = url::Url::parse(url).map_err(|_| AppError::Gh("invalid avatar URL".into()))?;
    let pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(k, _)| k != "s")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    parsed.query_pairs_mut().clear().extend_pairs(pairs).append_pair("s", AVATAR_SIZE);
    Ok(parsed.into())
}

pub async fn resolve(email: String, sha: String, owner_name: Option<(String, String)>) -> AppResult<Option<String>> {
    let url = tauri::async_runtime::spawn_blocking(move || avatar_url(&email, &sha, owner_name.as_ref()))
        .await
        .map_err(|e| AppError::Other(format!("avatar task failed: {e}")))??;
    let Some(url) = url else {
        return Ok(None);
    };
    let bytes = gh::fetch_authenticated_url(&sized(&url)?).await?;
    Ok(Some(gh::image_data_url(&bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        crate::db::migrate(&c).unwrap();
        c
    }

    #[test]
    fn noreply_with_id_maps_to_avatar_url_without_gh() {
        let url = avatar_url("123+octo-cat@users.noreply.github.com", "abc", None).unwrap();
        assert_eq!(url.as_deref(), Some("https://avatars.githubusercontent.com/u/123?v=4"));
    }

    #[test]
    fn non_github_email_without_remote_has_no_avatar() {
        assert_eq!(avatar_url("dev@example.com", "abc", None).unwrap(), None);
    }

    #[test]
    fn noreply_parsing_rejects_path_tricks() {
        assert!(parse_noreply("../x@users.noreply.github.com").is_none());
        assert!(parse_noreply("1+a/b@users.noreply.github.com").is_none());
        assert!(matches!(parse_noreply("octo@users.noreply.github.com"), Some(Noreply::Login(l)) if l == "octo"));
    }

    #[test]
    fn sized_replaces_size_param() {
        assert_eq!(
            sized("https://avatars.githubusercontent.com/u/1?v=4&s=460").unwrap(),
            "https://avatars.githubusercontent.com/u/1?v=4&s=64"
        );
    }

    #[test]
    fn cache_respects_hit_and_miss_ttls() {
        let c = conn();
        store(&c, "a@x", Some("data:x"), 1000).unwrap();
        store(&c, "b@x", None, 1000).unwrap();
        assert_eq!(cached(&c, "a@x", 1000 + MISS_TTL_SECS).unwrap(), Some(Some("data:x".into())));
        assert_eq!(cached(&c, "b@x", 1000 + 10).unwrap(), Some(None));
        assert_eq!(cached(&c, "b@x", 1000 + MISS_TTL_SECS).unwrap(), None);
        assert_eq!(cached(&c, "a@x", 1000 + HIT_TTL_SECS).unwrap(), None);
        assert_eq!(cached(&c, "missing@x", 1000).unwrap(), None);
    }
}
