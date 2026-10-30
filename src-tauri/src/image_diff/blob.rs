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

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &std::path::Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    #[test]
    fn reads_binary_blobs_from_the_local_clone_byte_for_byte() {
        let dir = std::env::temp_dir().join(format!("cr-blob-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q"]);
        // Not valid UTF-8: a lossy string conversion would corrupt it.
        let bytes: Vec<u8> = vec![0x89, b'P', b'N', b'G', 0xff, 0xfe, 0x00, 0x80];
        std::fs::write(dir.join("a.png"), &bytes).unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "x"]);

        let blobs = GitBlobs { local: Some(dir.clone()), owner_name: None, left: Some("HEAD".into()), right: None };
        assert_eq!(blobs.read(Side::Left, "a.png").unwrap(), bytes);
        assert!(blobs.read(Side::Left, "missing.png").is_err());
        assert!(blobs.read(Side::Right, "a.png").is_err(), "a side without a source");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
