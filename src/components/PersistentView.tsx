import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Repo } from "./Sidebar";
import TerminalTile, { type SessionInfo } from "./TerminalTile";
import "./PersistentView.css";

interface LocalInspection {
  configuredBasePath: string;
  repoPath: string;
  exists: boolean;
  isGitRepo: boolean;
  currentBranch: string | null;
  isClean: boolean | null;
  dirtyFiles: number | null;
  error: string | null;
}

interface Props {
  repo: Repo;
  onCountChange?: (count: number) => void;
}

export default function PersistentView({ repo, onCountChange }: Props) {
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const onCountChangeRef = useRef(onCountChange);
  onCountChangeRef.current = onCountChange;

  // Hydrate from backend on mount (and whenever the repo changes) so that
  // sessions that survived a tab switch reappear.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const list = await invoke<SessionInfo[]>("pty_list", {
          args: { repoId: repo.id },
        });
        if (!cancelled) setSessions(list ?? []);
      } catch (e) {
        if (!cancelled) {
          setError(e instanceof Error ? e.message : String(e));
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [repo.id]);

  useEffect(() => {
    onCountChangeRef.current?.(sessions.length);
  }, [sessions.length]);

  const handleCreate = useCallback(async () => {
    setCreating(true);
    setError(null);
    try {
      const inspection = await invoke<LocalInspection>("inspect_local_repo", {
        repoName: repo.name,
      });
      if (!inspection.exists) {
        throw new Error(`Local path not found: ${inspection.repoPath}`);
      }
      const created = await invoke<SessionInfo>("pty_create", {
        args: {
          repoId: repo.id,
          repoName: repo.name,
          cwd: inspection.repoPath,
        },
      });
      setSessions((prev) => [...prev, created]);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setCreating(false);
    }
  }, [repo.id, repo.name]);

  const handleClose = useCallback(async (id: string) => {
    setSessions((prev) => prev.filter((s) => s.id !== id));
    try {
      await invoke("pty_kill", { args: { id } });
    } catch {
      // The session was likely already gone — refresh on next render.
    }
  }, []);

  return (
    <section className="persistent" aria-label="Persistent terminals">
      <header className="persistent-header">
        <div>
          <h2>{repo.name}</h2>
          <p className="repo-meta">{repo.fullName}</p>
        </div>
        <span aria-label="terminal count">
          {sessions.length} terminal{sessions.length === 1 ? "" : "s"}
        </span>
      </header>
      {error && (
        <div role="alert" className="terminal-tile-error">
          {error}
        </div>
      )}
      <div className="persistent-grid">
        {sessions.map((s) => (
          <TerminalTile key={s.id} session={s} onClose={handleClose} />
        ))}
        <button
          type="button"
          className="terminal-new-tile"
          onClick={handleCreate}
          disabled={creating}
          aria-label="Open new terminal"
        >
          <span className="terminal-new-tile-plus" aria-hidden="true">
            +
          </span>
          <span className="terminal-new-tile-label">
            {creating ? "Starting…" : "New terminal"}
          </span>
        </button>
      </div>
    </section>
  );
}
