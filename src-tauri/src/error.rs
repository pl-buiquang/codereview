use serde::{Serialize, Serializer};

/// Application-wide error type. Serializes to a plain string so it can cross the
/// Tauri command boundary and surface as a rejected promise on the frontend.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("git error: {0}")]
    Git(String),

    #[error("gh error: {0}")]
    Gh(String),

    #[error("not a git repository: {0}")]
    NotARepo(String),

    #[error("{0}")]
    Other(String),
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_to_plain_string() {
        let err = AppError::Other("boom".into());
        let json = serde_json::to_string(&err).unwrap();
        assert_eq!(json, "\"boom\"");
    }

    #[test]
    fn display_includes_category_prefix() {
        assert_eq!(
            AppError::NotARepo("/tmp/x".into()).to_string(),
            "not a git repository: /tmp/x"
        );
        assert_eq!(AppError::Git("bad ref".into()).to_string(), "git error: bad ref");
        assert_eq!(AppError::Gh("nope".into()).to_string(), "gh error: nope");
    }

    #[test]
    fn rusqlite_error_converts_via_from() {
        let app: AppError = rusqlite::Error::QueryReturnedNoRows.into();
        assert!(matches!(app, AppError::Db(_)));
    }
}
