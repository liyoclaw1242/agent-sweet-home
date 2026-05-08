import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./Sidebar.css";

export interface Repo {
  id: number;
  name: string;
  fullName: string;
  description: string | null;
  htmlUrl: string;
  private: boolean;
  defaultBranch: string;
  stargazersCount: number;
  language: string | null;
  updatedAt: string;
}

interface Props {
  reloadKey: number;
  selectedRepoId: number | null;
  onSelect: (repo: Repo) => void;
}

export default function Sidebar({ reloadKey, selectedRepoId, onSelect }: Props) {
  const [repos, setRepos] = useState<Repo[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await invoke<Repo[]>("fetch_repos");
      setRepos(data);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setRepos([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load, reloadKey]);

  return (
    <aside className="sidebar">
      <div className="sidebar-header">
        <h2>Repositories</h2>
        <button
          type="button"
          onClick={load}
          disabled={loading}
          aria-label="Refresh repositories"
          title="Refresh"
        >
          ↻
        </button>
      </div>
      {loading && <p className="sidebar-status">Loading…</p>}
      {error && !loading && <p className="sidebar-status sidebar-error">{error}</p>}
      {!loading && !error && repos.length === 0 && (
        <p className="sidebar-status">No repositories yet.</p>
      )}
      <ul className="repo-list">
        {repos.map((r) => {
          const isSelected = selectedRepoId === r.id;
          return (
            <li
              key={r.id}
              className={`repo-item ${isSelected ? "is-selected" : ""}`}
            >
              <button
                type="button"
                className="repo-button"
                onClick={() => onSelect(r)}
                aria-current={isSelected ? "page" : undefined}
                title={r.fullName}
              >
                <span className="repo-name">
                  {r.name}
                  {r.private && <span className="repo-badge">private</span>}
                </span>
                {r.language && <span className="repo-lang">{r.language}</span>}
              </button>
            </li>
          );
        })}
      </ul>
    </aside>
  );
}
