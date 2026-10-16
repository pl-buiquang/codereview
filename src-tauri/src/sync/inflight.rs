//! In-flight dedup per (facet, id): two syncs that need the same fetch at the
//! same time do it once. The first one claims the key; the other skips it and
//! waits for the claim to drop, after which the result is in the mirror.

use std::collections::BTreeSet;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// A claimed fetch that outlives this is assumed stuck and no longer waited for.
const MAX_WAIT: Duration = Duration::from_secs(180);

pub struct InFlight {
    keys: Mutex<BTreeSet<String>>,
    cv: Condvar,
}

/// Keys this caller owns until dropped.
pub struct Claim<'a> {
    owner: &'a InFlight,
    keys: Vec<String>,
}

fn key(facet: &str, id: &str) -> String {
    format!("{facet}\u{0}{id}")
}

impl InFlight {
    pub const fn new() -> Self {
        Self { keys: Mutex::new(BTreeSet::new()), cv: Condvar::new() }
    }

    /// Claim `ids` for `facet`: returns the claim, the ids this caller must
    /// fetch, and the ids another caller is already fetching.
    pub fn claim(&self, facet: &str, ids: &[String]) -> (Claim<'_>, Vec<String>, Vec<String>) {
        let mut keys = self.keys.lock().unwrap();
        let (mut mine, mut others) = (Vec::new(), Vec::new());
        let mut claimed = Vec::new();
        for id in ids {
            let k = key(facet, id);
            if keys.insert(k.clone()) {
                claimed.push(k);
                mine.push(id.clone());
            } else if !mine.contains(id) && !others.contains(id) {
                others.push(id.clone());
            }
        }
        (Claim { owner: self, keys: claimed }, mine, others)
    }

    /// Block until no other caller holds any of `ids` for `facet`.
    pub fn wait(&self, facet: &str, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        let wanted: Vec<String> = ids.iter().map(|id| key(facet, id)).collect();
        let deadline = Instant::now() + MAX_WAIT;
        let mut keys = self.keys.lock().unwrap();
        while wanted.iter().any(|k| keys.contains(k)) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                crate::logging::warn(&format!("[sync] gave up waiting for {facet} fetches"));
                return;
            }
            keys = self.cv.wait_timeout(keys, left).unwrap().0;
        }
    }
}

impl Default for InFlight {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        if self.keys.is_empty() {
            return;
        }
        let mut keys = self.owner.keys.lock().unwrap();
        for k in &self.keys {
            keys.remove(k);
        }
        self.owner.cv.notify_all();
    }
}

static GLOBAL: InFlight = InFlight::new();

pub fn global() -> &'static InFlight {
    &GLOBAL
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_second_claim_gets_only_the_free_ids() {
        let f = InFlight::new();
        let (_a, mine_a, others_a) = f.claim("node", &ids(&["1", "2"]));
        assert_eq!((mine_a, others_a), (ids(&["1", "2"]), vec![]));
        let (_b, mine_b, others_b) = f.claim("node", &ids(&["2", "3"]));
        assert_eq!((mine_b, others_b), (ids(&["3"]), ids(&["2"])));
        let (_c, mine_c, _) = f.claim("threads", &ids(&["2"]));
        assert_eq!(mine_c, ids(&["2"]), "facets are independent");
    }

    #[test]
    fn dropping_a_claim_frees_and_wakes_waiters() {
        let f = Arc::new(InFlight::new());
        let (claim, _, _) = f.claim("node", &ids(&["1"]));
        let waiter = {
            let f = f.clone();
            std::thread::spawn(move || {
                f.wait("node", &ids(&["1"]));
                f.claim("node", &ids(&["1"])).1
            })
        };
        std::thread::sleep(Duration::from_millis(50));
        drop(claim);
        assert_eq!(waiter.join().unwrap(), ids(&["1"]));
    }
}
