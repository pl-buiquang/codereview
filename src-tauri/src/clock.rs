//! Wall-clock helpers on top of `jiff`.
//!
//! Timestamps are stored in SQLite as RFC 3339 text and compared as strings, so
//! [`rfc3339`] keeps the exact format the app wrote before (chrono's
//! `to_rfc3339()`): UTC with a `+00:00` offset and 0, 3, 6 or 9 fraction digits.

pub use jiff::{SignedDuration, Timestamp};

/// The current instant.
pub fn now() -> Timestamp {
    Timestamp::now()
}

/// `2026-11-25T10:12:23.758226+00:00` (fraction trimmed to 0/3/6/9 digits).
pub fn rfc3339(t: Timestamp) -> String {
    let nanos = t.subsec_nanosecond();
    let frac = if nanos == 0 {
        String::new()
    } else if nanos % 1_000_000 == 0 {
        format!(".{:03}", nanos / 1_000_000)
    } else if nanos % 1_000 == 0 {
        format!(".{:06}", nanos / 1_000)
    } else {
        format!(".{nanos:09}")
    };
    format!("{}{frac}+00:00", t.strftime("%Y-%m-%dT%H:%M:%S"))
}

/// [`rfc3339`] of [`now`].
pub fn now_rfc3339() -> String {
    rfc3339(now())
}

/// Parse an RFC 3339 timestamp with any offset (`Z`, `+00:00`, `+02:00`…).
pub fn parse_rfc3339(s: &str) -> Option<Timestamp> {
    s.parse().ok()
}

/// How long before `now` the instant `t` was; `None` when `t` is in the future.
pub fn age(t: Timestamp, now: Timestamp) -> Option<std::time::Duration> {
    std::time::Duration::try_from(now.duration_since(t)).ok()
}

/// A duration of `n` whole days (24 h each).
pub const fn days(n: i64) -> SignedDuration {
    SignedDuration::from_hours(n * 24)
}

/// Whole milliseconds since the Unix epoch.
pub fn millis(t: Timestamp) -> i64 {
    t.as_millisecond()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn formats_like_chrono() {
        assert_eq!(
            rfc3339(ts("2026-11-25T10:12:23Z")),
            "2026-11-25T10:12:23+00:00"
        );
        assert_eq!(
            rfc3339(ts("2026-11-25T10:12:23.5Z")),
            "2026-11-25T10:12:23.500+00:00"
        );
        assert_eq!(
            rfc3339(ts("2026-11-25T10:12:23.758226Z")),
            "2026-11-25T10:12:23.758226+00:00"
        );
        assert_eq!(
            rfc3339(ts("2026-11-25T10:12:23.000000001Z")),
            "2026-11-25T10:12:23.000000001+00:00"
        );
        // Other offsets are normalised to UTC.
        assert_eq!(
            rfc3339(ts("2026-11-25T12:00:00+02:00")),
            "2026-11-25T10:00:00+00:00"
        );
    }

    #[test]
    fn parses_any_offset() {
        let a = parse_rfc3339("2026-11-25T10:00:00+00:00").unwrap();
        assert_eq!(parse_rfc3339("2026-11-25T10:00:00Z"), Some(a));
        assert_eq!(parse_rfc3339("2026-11-25T12:00:00+02:00"), Some(a));
        assert_eq!(parse_rfc3339("garbage"), None);
    }

    #[test]
    fn days_are_24h() {
        assert_eq!(days(2).as_secs(), 172_800);
    }
}
