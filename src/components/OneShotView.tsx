import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Repo } from "./Sidebar";
import OneShotModal, { type RunArgs } from "./OneShotModal";
import "./OneShotView.css";

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

interface RunInfo {
  id: string;
  repoId: number;
  repoName: string;
  cwd: string;
  prompt: string;
  argv: string[];
  status: "running" | "completed" | "failed" | "killed";
  startedAt: number;
  endedAt: number | null;
  exitCode: number | null;
  totalCostUsd: number | null;
  outputFormat: string;
}

interface LogLine {
  runId: string;
  seq: number;
  ts: number;
  stream: "stdout" | "stderr";
  text: string;
}

interface Props {
  repo: Repo;
  onCountChange?: (count: number) => void;
}

function statusClass(s: RunInfo["status"]): string {
  return `status-badge status-${s}`;
}

function shortPrompt(p: string): string {
  return p.length > 100 ? `${p.slice(0, 100)}…` : p;
}

function formatTime(ts: number): string {
  if (!ts) return "—";
  return new Date(ts * 1000).toLocaleTimeString();
}

export default function OneShotView({ repo, onCountChange }: Props) {
  const [runs, setRuns] = useState<RunInfo[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [lines, setLines] = useState<LogLine[]>([]);
  const [showModal, setShowModal] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const onCountRef = useRef(onCountChange);
  onCountRef.current = onCountChange;

  const loadRuns = useCallback(async () => {
    try {
      const list = await invoke<RunInfo[]>("one_shot_list", {
        args: { repoId: repo.id },
      });
      setRuns(list ?? []);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [repo.id]);

  useEffect(() => {
    void loadRuns();
  }, [loadRuns]);

  const runningCount = useMemo(
    () => runs.filter((r) => r.status === "running").length,
    [runs],
  );
  useEffect(() => {
    onCountRef.current?.(runningCount);
  }, [runningCount]);

  // Stream the active run's log via the Tauri event channel and seed it
  // with whatever already landed in SQLite before we attached.
  useEffect(() => {
    if (!activeId) {
      setLines([]);
      return;
    }
    let unlistenLine: UnlistenFn | null = null;
    let unlistenExit: UnlistenFn | null = null;
    let cancelled = false;

    void (async () => {
      try {
        const initial = await invoke<LogLine[]>("one_shot_log", {
          args: { id: activeId, sinceSeq: -1, limit: 5000 },
        });
        if (cancelled) return;
        setLines(initial ?? []);
      } catch (e) {
        if (!cancelled) {
          setError(e instanceof Error ? e.message : String(e));
        }
      }
      const ul = await listen<LogLine>(`oneshot:line:${activeId}`, (event) => {
        const ll = event.payload;
        setLines((prev) => {
          if (prev.length > 0 && prev[prev.length - 1].seq >= ll.seq) {
            return prev;
          }
          return [...prev, ll];
        });
      });
      const ux = await listen<{ exitCode: number; status: string }>(
        `oneshot:exit:${activeId}`,
        () => {
          void loadRuns();
        },
      );
      if (cancelled) {
        ul();
        ux();
      } else {
        unlistenLine = ul;
        unlistenExit = ux;
      }
    })();

    return () => {
      cancelled = true;
      unlistenLine?.();
      unlistenExit?.();
    };
  }, [activeId, loadRuns]);

  const handleStart = useCallback(
    async (raw: RunArgs) => {
      const inspection = await invoke<LocalInspection>("inspect_local_repo", {
        repoName: repo.name,
      });
      if (!inspection.exists) {
        throw new Error(`Local path not found: ${inspection.repoPath}`);
      }
      const args: RunArgs = { ...raw, cwd: inspection.repoPath };
      const created = await invoke<RunInfo>("one_shot_start", { args });
      setRuns((prev) => [created, ...prev]);
      setActiveId(created.id);
    },
    [repo.name],
  );

  const handleKill = useCallback(
    async (id: string) => {
      try {
        await invoke("one_shot_kill", { id });
      } catch {
        // ignore — likely already gone
      }
      await loadRuns();
      if (activeId === id) {
        // Refresh the displayed log from DB after the [killed by user] line.
        try {
          const refreshed = await invoke<LogLine[]>("one_shot_log", {
            args: { id, sinceSeq: -1, limit: 5000 },
          });
          setLines(refreshed ?? []);
        } catch {
          // ignore
        }
      }
    },
    [activeId, loadRuns],
  );

  const activeRun = useMemo(
    () => runs.find((r) => r.id === activeId) ?? null,
    [runs, activeId],
  );

  return (
    <section className="oneshot" aria-label="One-shot runs">
      <aside className="oneshot-list" aria-label="Run list">
        <div className="oneshot-list-header">
          <h3>{repo.name}</h3>
          <button
            type="button"
            className="oneshot-new-btn"
            onClick={() => setShowModal(true)}
            aria-label="New one-shot run"
          >
            + New run
          </button>
        </div>
        {error && <div className="oneshot-modal-error">{error}</div>}
        {runs.length === 0 ? (
          <p className="oneshot-detail-empty">No runs yet.</p>
        ) : (
          <ul className="oneshot-runs">
            {runs.map((r) => (
              <li key={r.id}>
                <button
                  type="button"
                  className={`oneshot-run-button ${
                    activeId === r.id ? "is-active" : ""
                  }`}
                  onClick={() => setActiveId(r.id)}
                  aria-label={`Run ${r.id}`}
                >
                  <span className="oneshot-run-id">{r.id}</span>
                  <span className="oneshot-run-prompt">
                    {shortPrompt(r.prompt) || <em>(no prompt — resumed)</em>}
                  </span>
                  <span className="oneshot-run-meta">
                    <span className={statusClass(r.status)}>{r.status}</span>
                    <span>{formatTime(r.startedAt)}</span>
                    {r.totalCostUsd != null && (
                      <span>${r.totalCostUsd.toFixed(4)}</span>
                    )}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </aside>

      <article className="oneshot-detail" aria-label="Run detail">
        {activeRun ? (
          <>
            <header className="oneshot-detail-header">
              <div className="grow">
                <div className="oneshot-run-id">{activeRun.id}</div>
                <div className="oneshot-detail-cmd" title={activeRun.argv.join(" ")}>
                  {activeRun.argv.join(" ")}
                </div>
              </div>
              <span className={statusClass(activeRun.status)}>
                {activeRun.status}
              </span>
              {activeRun.status === "running" ? (
                <button
                  type="button"
                  onClick={() => handleKill(activeRun.id)}
                  aria-label={`Kill ${activeRun.id}`}
                >
                  Kill
                </button>
              ) : (
                <button
                  type="button"
                  onClick={() => handleKill(activeRun.id)}
                  aria-label={`Delete ${activeRun.id}`}
                >
                  Delete
                </button>
              )}
            </header>
            <div className="oneshot-log" aria-label="Log output">
              {lines.length === 0 ? (
                <p className="oneshot-detail-empty">
                  Waiting for output…
                </p>
              ) : (
                lines.map((l) => (
                  <div
                    key={l.seq}
                    className={`oneshot-log-line ${
                      l.stream === "stderr" ? "is-stderr" : ""
                    }`}
                  >
                    {l.text}
                  </div>
                ))
              )}
            </div>
          </>
        ) : (
          <div className="oneshot-detail-empty">
            Select a run on the left, or start a new one.
          </div>
        )}
      </article>

      {showModal && (
        <OneShotModal
          repoId={repo.id}
          repoName={repo.name}
          cwd=""
          onSubmit={handleStart}
          onClose={() => setShowModal(false)}
        />
      )}
    </section>
  );
}
