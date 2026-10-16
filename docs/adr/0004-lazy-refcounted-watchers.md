# Git file watchers are lazy and refcounted by open views

Local git changes are picked up by `notify` watchers on `.git` (HEAD, refs, packed-refs, FETCH_HEAD, worktree HEADs, index) instead of polling or manual reloads. We don't watch every registered clone from startup. A watcher is held by the views that need it: the Graph view of a repo, a local-target Review, a PR Review with a local clone, and the Worktrees view (which holds all repos while open). It's released about 60 s after the last holder goes away. Repos without a watcher simply Reload on view mount and window focus.

## Considered Options

- Watch every repository with a local clone, always. Rejected: we preferred not to keep watches on dozens of idle clones, and accepted the extra lifecycle bookkeeping.
