import { useCallback, useEffect, useRef, useState } from "react";
import type React from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Repo, SidebarSession } from "./Sidebar";
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
  onSessionsChange?: (sessions: SidebarSession[]) => void;
  newTerminalRef?: React.MutableRefObject<(() => void) | null>;
}

function toSidebarSession(s: SessionInfo): SidebarSession {
  if (s.exitCode !== null) return { id: s.id, status: "exited",  meta: `exit ${s.exitCode}` };
  if (s.frozen)            return { id: s.id, status: "frozen",  meta: "idle" };
  const m = Math.floor(s.uptimeSecs / 60);
  const sec = s.uptimeSecs % 60;
  return { id: s.id, status: "running", meta: m > 0 ? `${m}m${sec}s` : `${sec}s` };
}

export default function PersistentView({ repo, onCountChange, onSessionsChange, newTerminalRef }: Props) {
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const onCountChangeRef = useRef(onCountChange);
  onCountChangeRef.current = onCountChange;
  const onSessionsChangeRef = useRef(onSessionsChange);
  onSessionsChangeRef.current = onSessionsChange;

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
    onSessionsChangeRef.current?.(sessions.map(toSidebarSession));
  }, [sessions]);

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

  // Expose handleCreate to sidebar "+" button
  useEffect(() => {
    if (newTerminalRef) {
      newTerminalRef.current = () => { void handleCreate(); };
      return () => { newTerminalRef.current = null; };
    }
  }, [newTerminalRef, handleCreate]);

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
      </div>
    </section>
  );
}
