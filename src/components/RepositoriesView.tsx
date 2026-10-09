import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, pickFolder } from "../lib/api";
import { toast } from "../lib/toast";
import { confirmDialog } from "../lib/confirm";
import { repoLabel } from "../lib/repoLabel";
import { useUIStore } from "../store";
import type { Repository } from "../lib/types";
import { Icon } from "./icons";
import { useSettingsStore, parseRepoBasePaths } from "../lib/settings";

export function RepositoriesView() {
  const queryClient = useQueryClient();
  const openRepo = useUIStore((s) => s.openRepo);
  const forgetRepo = useUIStore((s) => s.forgetRepo);
  const repoBasePaths = useSettingsStore((s) => s.repoBasePaths);

  const reposQuery = useQuery({
    queryKey: ["repositories"],
    queryFn: api.listRepositories,
  });

  const addRepo = useMutation({
    mutationFn: async () => {
      const path = await pickFolder();
      if (!path) return null;
      return api.addRepository(path);
    },
    onSuccess: (repo) => {
      if (repo) {
        // Seed the cache before opening the repo: App's cleanup effect forgets
        // repos whose id isn't in ["repositories"], so the new repo would be
        // dropped if we opened it against the stale (pre-refetch) list.
        queryClient.setQueryData<Repository[]>(["repositories"], (old) =>
          old ? (old.some((r) => r.id === repo.id) ? old : [...old, repo]) : [repo],
        );
        openRepo(repo.id);
      }
      queryClient.invalidateQueries({ queryKey: ["repositories"] });
    },
    onError: (err) => toast.error(`Could not add repository:\n${String(err)}`),
  });

  const removeRepo = useMutation({
    mutationFn: (id: number) => api.removeRepository(id),
    onSuccess: (_data, id) => {
      queryClient.invalidateQueries({ queryKey: ["repositories"] });
      forgetRepo(id);
    },
  });

  const linkPath = useMutation({
    mutationFn: async (repoId: number) => {
      const path = await pickFolder();
      if (!path) return null;
      return api.linkLocalPath(repoId, path);
    },
    onSuccess: (repo) => {
      if (repo) {
        queryClient.invalidateQueries({ queryKey: ["repositories"] });
        toast.success(`Linked local clone for ${repoLabel(repo)}`);
      }
    },
    onError: (err) => toast.error(`Could not link path:\n${String(err)}`),
  });

  const autoLink = useMutation({
    mutationFn: () => api.autoLinkRepos(parseRepoBasePaths(repoBasePaths)),
    onSuccess: (count) => {
      queryClient.invalidateQueries({ queryKey: ["repositories"] });
      toast.success(count > 0 ? `Auto-linked ${count} repo${count === 1 ? "" : "s"}` : "No new repos to auto-link");
    },
    onError: (err) => toast.error(`Auto-link failed:\n${String(err)}`),
  });

  const repos = reposQuery.data ?? [];
  const hasRemoteOnly = repos.some((r) => !r.local_path);

  return (
    <section className="cr-main">
      <header className="cr-pagehead">
        <h1 className="cr-h1">Repositories</h1>
        <div className="cr-spacer" />
        {hasRemoteOnly && (
          <button
            className="btn btn-sm"
            onClick={() => autoLink.mutate()}
            disabled={autoLink.isPending}
            title="Scan base paths and link local clones to remote-only repos"
          >
            {autoLink.isPending ? "Scanning…" : "Auto-link"}
          </button>
        )}
        <button className="btn btn-primary" onClick={() => addRepo.mutate()} disabled={addRepo.isPending}>
          <Icon name="plus" size={13} /> {addRepo.isPending ? "Adding…" : "Add repo"}
        </button>
      </header>

      <nav className="cr-list">
        {reposQuery.isLoading && <p className="muted">Loading…</p>}
        {!reposQuery.isLoading && repos.length === 0 && (
          <p className="muted">No repositories yet. Add a local git repo to start.</p>
        )}
        {repos.map((repo) => (
          <div key={repo.id} className="card repo-row" onClick={() => openRepo(repo.id)}>
            <span className="repo-row-icon">
              <Icon name="repo" size={16} />
            </span>
            <div className="repo-row-main">
              <span className="repo-row-name">{repoLabel(repo)}</span>
              <span className="repo-row-path" title={repo.local_path ?? undefined}>
                {repo.local_path ?? <em className="muted">No local clone</em>}
              </span>
            </div>
            {!repo.local_path && (
              <button
                className="btn btn-sm"
                title="Link a local clone for this repo"
                onClick={(e) => {
                  e.stopPropagation();
                  linkPath.mutate(repo.id);
                }}
              >
                Link
              </button>
            )}
            <button
              className="btn-icon"
              title="Remove repository"
              onClick={async (e) => {
                e.stopPropagation();
                if (
                  await confirmDialog({
                    title: "Remove repository",
                    message: `Remove ${repoLabel(repo)} from codereview?`,
                    confirmLabel: "Remove",
                    danger: true,
                  })
                ) {
                  removeRepo.mutate(repo.id);
                }
              }}
            >
              <Icon name="x" size={12} />
            </button>
          </div>
        ))}
      </nav>
    </section>
  );
}
