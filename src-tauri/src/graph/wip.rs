use std::path::Path;

use crate::error::{AppError, AppResult};

use super::{WipKind, WipStatus};

pub fn worktree_status(_checkout: &Path) -> AppResult<WipStatus> {
    Err(AppError::Other("not implemented".into()))
}

pub fn worktree_file_diff(_checkout: &Path, _path: &str, _kind: WipKind) -> AppResult<String> {
    Err(AppError::Other("not implemented".into()))
}
