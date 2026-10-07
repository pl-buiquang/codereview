use std::path::Path;

use crate::error::{AppError, AppResult};

use super::CommitDetail;

pub fn commit_detail(_checkout: &Path, _sha: &str) -> AppResult<CommitDetail> {
    Err(AppError::Other("not implemented".into()))
}

pub fn commit_file_diff(
    _checkout: &Path,
    _sha: &str,
    _path: &str,
    _old_path: Option<&str>,
) -> AppResult<String> {
    Err(AppError::Other("not implemented".into()))
}
