//! Bytes of an image at a commit: the local clone first, the GitHub contents API otherwise.

use std::path::PathBuf;

use crate::error::{AppError, AppResult};
use crate::{gh, git};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

pub trait BlobSource: Sync {
    fn read(&self, side: Side, path: &str) -> AppResult<Vec<u8>>;
}

pub struct GitBlobs {
    pub local: Option<PathBuf>,
    pub owner_name: Option<(String, String)>,
    /// The commit (or tree) each side is read at; `None` when that side has no source.
    pub left: Option<String>,
    pub right: Option<String>,
}

impl BlobSource for GitBlobs {
    fn read(&self, side: Side, path: &str) -> AppResult<Vec<u8>> {
        let rev = match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
        .as_deref()
        .ok_or_else(|| AppError::Other("this side has no source".into()))?;
        let local = self.local.as_deref().map(|repo| git::show_blob_bytes(repo, rev, path));
        match (local, &self.owner_name) {
            (Some(Ok(bytes)), _) => Ok(bytes),
            (_, Some((owner, name))) => gh::file_bytes_at_ref(owner, name, path, rev),
            (Some(Err(e)), None) => Err(e),
            (None, None) => Err(AppError::Other("no local clone and no GitHub remote".into())),
        }
    }
}
