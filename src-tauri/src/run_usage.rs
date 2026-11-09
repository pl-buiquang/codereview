//! The plain data of an AI run: what it is for ([`RunTag`]), what it consumed ([`RunUsage`]) and the
//! timestamp format of the usage log. Kept free of Tauri so the run engine, the harnesses and
//! `cr worker` can use it without the app's usage log (`ai_usage`).

use chrono::{DateTime, SecondsFormat, Utc};

/// What a run is for. `feature` is a stable key the Usage section groups by.
#[derive(Clone)]
pub struct RunTag {
    pub feature: &'static str,
    pub variant: Option<String>,
    pub review_id: Option<i64>,
    pub target_id: Option<i64>,
    pub label: String,
    /// Model requested with `--model`; empty when claude picked its own default.
    pub model: String,
    /// Id of the harness that ran it.
    pub harness: &'static str,
    /// The remote host that ran it; `None` for a local run.
    pub host_id: Option<i64>,
}

impl RunTag {
    pub fn new(feature: &'static str) -> Self {
        Self { feature, variant: None, review_id: None, target_id: None, label: String::new(), model: String::new(), harness: "claude", host_id: None }
    }

    pub fn variant(mut self, variant: impl Into<String>) -> Self {
        self.variant = Some(variant.into());
        self
    }

    pub fn review(mut self, review_id: i64) -> Self {
        self.review_id = Some(review_id);
        self
    }

    pub fn harness(mut self, harness: &'static str) -> Self {
        self.harness = harness;
        self
    }

    pub fn host(mut self, host_id: Option<i64>) -> Self {
        self.host_id = host_id;
        self
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct ModelTokens {
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub cost_usd: f64,
}

/// Everything the result envelope reports about one run; all zero when it never produced one.
#[derive(Default, Clone, Debug)]
pub struct RunUsage {
    pub num_turns: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub cost_usd: f64,
    pub session_id: Option<String>,
    pub models: Vec<ModelTokens>,
}

/// Fixed-width UTC timestamp, so `started_at` strings compare chronologically.
pub fn stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn now_stamp() -> String {
    stamp(Utc::now())
}
