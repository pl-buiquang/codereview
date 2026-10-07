use std::path::Path;

use crate::error::{AppError, AppResult};

use super::{GraphCommit, RefInfo, Scope};

pub fn graph_log(
    _checkout: &Path,
    _scope: Scope,
    _skip: u32,
    _limit: u32,
) -> AppResult<Vec<GraphCommit>> {
    Err(AppError::Other("not implemented".into()))
}

pub fn list_refs(_checkout: &Path) -> AppResult<Vec<RefInfo>> {
    Err(AppError::Other("not implemented".into()))
}
