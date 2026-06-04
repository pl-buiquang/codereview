import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, pickFolder } from "../lib/api";
import { toast } from "../lib/toast";
import { confirmDialog } from "../lib/confirm";
import { useUIStore } from "../store";
import type { Repository } from "../lib/types";

function repoLabel(repo: Repository): string {
  if (repo.remote_owner && repo.remote_name) {
    return `${repo.remote_owner}/${repo.remote_name}`;
  }
  return repo.path.split("/").filter(Boolean).pop() ?? repo.path;
}

export function RepoSidebar() {
  const queryClient = useQueryClient();
  const { activeRepoId, setActiveRepo } = useUIStore();

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
      queryClient.invalidateQueries({ queryKey: ["repositories"] });
      if (repo) setActiveRepo(repo.id);
    },
    onError: (err) => toast.error(`Could not add repository:\n${String(err)}`),
  });

  const removeRepo = useMutation({
    mutationFn: (id: number) => api.removeRepository(id),
    onSuccess: (_data, id) => {
      queryClient.invalidateQueries({ queryKey: ["repositories"] });
      if (activeRepoId === id) setActiveRepo(null);
    },
  });

  const repos = reposQuery.data ?? [];

  return (
    <aside className="sidebar">
      <div className="sidebar-header">
        <span className="app-title">codereview</span>
        <button
          className="btn-primary"
          onClick={() => addRepo.mutate()}
          disabled={addRepo.isPending}
        >
          {addRepo.isPending ? "Adding…" : "+ Add repo"}
        </button>
      </div>

      <nav className="repo-list">
        {reposQuery.isLoading && <p className="muted">Loading…</p>}
        {!reposQuery.isLoading && repos.length === 0 && (
          <p className="muted">No repositories yet. Add a local git repo to start.</p>
        )}
        {repos.map((repo) => (
          <div
            key={repo.id}
            className={`repo-item ${repo.id === activeRepoId ? "active" : ""}`}
            onClick={() => setActiveRepo(repo.id)}
          >
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
    </aside>
  );
}
