import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, pickFolder } from "../lib/api";
import { toast } from "../lib/toast";
import { confirmDialog } from "../lib/confirm";
import { repoLabel } from "../lib/repoLabel";
import { useUIStore } from "../store";
import type { Repository } from "../lib/types";

export function RepositoriesView() {
  const queryClient = useQueryClient();
  const openRepoTab = useUIStore((s) => s.openRepoTab);
  const closeTab = useUIStore((s) => s.closeTab);

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
        // Seed the cache before opening the tab: App's cleanup effect closes
        // repo/review tabs whose repoId isn't in ["repositories"], so the new
        // tab would be killed if we opened it against the stale (pre-refetch)
        // list — landing the user back on the previously active tab.
        queryClient.setQueryData<Repository[]>(["repositories"], (old) =>
          old ? (old.some((r) => r.id === repo.id) ? old : [...old, repo]) : [repo],
        );
        openRepoTab(repo.id);
      }
      queryClient.invalidateQueries({ queryKey: ["repositories"] });
    },
    onError: (err) => toast.error(`Could not add repository:\n${String(err)}`),
  });

  const removeRepo = useMutation({
    mutationFn: (id: number) => api.removeRepository(id),
    onSuccess: (_data, id) => {
      queryClient.invalidateQueries({ queryKey: ["repositories"] });
      closeTab(`repo-${id}`);
    },
  });

  const repos = reposQuery.data ?? [];

  return (
    <section className="main-panel repos-panel">
      <header className="inbox-header">
        <h2 className="inbox-h">Repositories</h2>
        <button className="btn-primary" onClick={() => addRepo.mutate()} disabled={addRepo.isPending}>
          {addRepo.isPending ? "Adding…" : "+ Add repo"}
        </button>
      </header>

      <nav className="repo-list">
        {reposQuery.isLoading && <p className="muted">Loading…</p>}
        {!reposQuery.isLoading && repos.length === 0 && (
          <p className="muted">No repositories yet. Add a local git repo to start.</p>
        )}
        {repos.map((repo) => (
          <div key={repo.id} className="repo-item" onClick={() => openRepoTab(repo.id)}>
            <div className="repo-item-main">
              <span className="repo-name">{repoLabel(repo)}</span>
              <span className="repo-path" title={repo.path}>
                {repo.path}
              </span>
            </div>
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
              ✕
            </button>
          </div>
        ))}
      </nav>
    </section>
  );
}
