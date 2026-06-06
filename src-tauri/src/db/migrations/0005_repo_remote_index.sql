-- Speed up resolving a repository by its GitHub owner/name, used when opening an
-- inbox PR as a review (which may create a clone-less "remote-only" repo row
-- whose `path` is a `github:owner/name` sentinel rather than a real clone path).
CREATE INDEX IF NOT EXISTS idx_repo_remote ON repository (remote_owner, remote_name);
