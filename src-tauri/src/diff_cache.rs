//! Process-wide LRU of review diffs. A diff is immutable for a given
//! (repo, base, head, three_dot), so entries never need invalidating; the
//! bounds only keep memory in check.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const MAX_ENTRIES: usize = 16;
const MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffKey {
    pub repo_id: i64,
    pub pr_number: Option<i64>,
    pub base_sha: String,
    pub head_sha: String,
    pub three_dot: bool,
}

struct Lru {
    entries: VecDeque<(DiffKey, Arc<String>)>,
    bytes: usize,
}

impl Lru {
    const fn new() -> Self {
        Self { entries: VecDeque::new(), bytes: 0 }
    }

    fn get(&mut self, key: &DiffKey) -> Option<Arc<String>> {
        let i = self.entries.iter().position(|(k, _)| k == key)?;
        let entry = self.entries.remove(i)?;
        let diff = entry.1.clone();
        self.entries.push_front(entry);
        Some(diff)
    }

    fn put(&mut self, key: DiffKey, diff: Arc<String>) {
        if let Some(i) = self.entries.iter().position(|(k, _)| *k == key) {
            if let Some((_, old)) = self.entries.remove(i) {
                self.bytes -= old.len();
            }
        }
        if diff.len() > MAX_BYTES {
            return;
        }
        self.bytes += diff.len();
        self.entries.push_front((key, diff));
        while self.entries.len() > MAX_ENTRIES || self.bytes > MAX_BYTES {
            match self.entries.pop_back() {
                Some((_, old)) => self.bytes -= old.len(),
                None => break,
            }
        }
    }
}

static CACHE: Mutex<Lru> = Mutex::new(Lru::new());

/// The cached diff for `key`, or `load()`'s result (cached on success).
/// The cache lock is not held while `load` runs.
pub fn get_or_load(
    key: &DiffKey,
    load: impl FnOnce() -> crate::error::AppResult<String>,
) -> crate::error::AppResult<Arc<String>> {
    if let Some(hit) = CACHE.lock().unwrap().get(key) {
        return Ok(hit);
    }
    let diff = Arc::new(load()?);
    CACHE.lock().unwrap().put(key.clone(), diff.clone());
    Ok(diff)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(head: &str) -> DiffKey {
        DiffKey {
            repo_id: 1,
            pr_number: Some(7),
            base_sha: "base".into(),
            head_sha: head.into(),
            three_dot: true,
        }
    }

    #[test]
    fn hit_skips_load_and_new_head_misses() {
        let mut lru = Lru::new();
        lru.put(key("a"), Arc::new("diff a".into()));
        assert_eq!(lru.get(&key("a")).as_deref().map(String::as_str), Some("diff a"));
        assert!(lru.get(&key("b")).is_none());
    }

    #[test]
    fn evicts_least_recently_used() {
        let mut lru = Lru::new();
        for i in 0..MAX_ENTRIES {
            lru.put(key(&i.to_string()), Arc::new(String::new()));
        }
        lru.get(&key("0"));
        lru.put(key("new"), Arc::new(String::new()));
        assert!(lru.get(&key("0")).is_some(), "recently used entry survives");
        assert!(lru.get(&key("1")).is_none(), "oldest untouched entry evicted");
        assert_eq!(lru.entries.len(), MAX_ENTRIES);
    }

    #[test]
    fn replacing_a_key_keeps_byte_count_exact() {
        let mut lru = Lru::new();
        lru.put(key("a"), Arc::new("12345".into()));
        lru.put(key("a"), Arc::new("12".into()));
        assert_eq!(lru.bytes, 2);
        assert_eq!(lru.entries.len(), 1);
    }

    #[test]
    fn global_cache_loads_once() {
        let k = DiffKey { repo_id: -42, ..key("global") };
        let mut calls = 0;
        for _ in 0..2 {
            let d = get_or_load(&k, || {
                calls += 1;
                Ok("d".into())
            })
            .unwrap();
            assert_eq!(d.as_str(), "d");
        }
        assert_eq!(calls, 1);
    }
}
