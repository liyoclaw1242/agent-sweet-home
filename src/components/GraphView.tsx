import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./GraphView.css";

interface RunSummary {
  runId: string;
  repoName: string;
  status: string;
  startedAt: number;
  endedAt: number | null;
  totalCostUsd: number | null;
  eventCount: number;
  toolCallCount: number;
}

interface GraphState {
  runs: RunSummary[];
}

interface BlockerRef {
  issueId: string;
  title: string | null;
}

interface BlockingItem {
  issueId: string;
  title: string | null;
  blockedBy: BlockerRef[];
}

function statusIcon(s: string): string {
  if (s === "running") return "●";
  if (s === "completed") return "✓";
  if (s === "failed") return "✗";
  return "⊘";
}

function statusClass(s: string): string {
  if (s === "running") return "gs-status-running";
  if (s === "completed") return "gs-status-ok";
  if (s === "failed") return "gs-status-fail";
  return "gs-status-killed";
}

function formatCost(v: number | null): string {
  if (v == null) return "—";
  return `$${v.toFixed(4)}`;
}

function timeAgo(ts: number): string {
  const sec = Math.floor(Date.now() / 1000) - ts;
  if (sec < 60) return `${sec}s ago`;
  if (sec < 3600) return `${Math.floor(sec / 60)}m ago`;
  return `${Math.floor(sec / 3600)}h ago`;
}

function issueNumber(id: string): string {
  const m = id.match(/#(\d+)$/);
  return m ? `#${m[1]}` : id;
}

export default function GraphView() {
  const [runs, setRuns] = useState<RunSummary[]>([]);
  const [blocking, setBlocking] = useState<BlockingItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [lastRefresh, setLastRefresh] = useState<Date | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      const [gs, bl] = await Promise.all([
        invoke<GraphState>("graph_state_cmd"),
        invoke<BlockingItem[]>("graph_blocking_cmd"),
      ]);
      setRuns(gs.runs);
      setBlocking(bl);
      setLastRefresh(new Date());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
    const id = setInterval(() => void load(), 15_000);
    return () => clearInterval(id);
  }, [load]);

  const running = runs.filter((r) => r.status === "running");
  const recent = runs.slice(0, 30);

  return (
    <div className="graph-view">
      <header className="graph-header">
        <h2>State Graph</h2>
        <div className="graph-header-right">
          {lastRefresh && (
            <span className="graph-last-refresh">
              updated {lastRefresh.toLocaleTimeString()}
            </span>
          )}
          <button
            type="button"
            className="graph-reload-btn"
            onClick={() => void load()}
          >
            ↻ Refresh
          </button>
        </div>
      </header>

      {error && <p className="graph-error">Error: {error}</p>}

      <div className="graph-panels">
        {/* ── Left: Runs ── */}
        <section className="graph-panel">
          <div className="graph-panel-header">
            <span className="graph-panel-title">Recent Runs</span>
            {running.length > 0 && (
              <span className="graph-live-badge">{running.length} running</span>
            )}
          </div>

          {loading && runs.length === 0 ? (
            <p className="graph-empty">Loading…</p>
          ) : runs.length === 0 ? (
            <p className="graph-empty">
              No runs yet. Start a One-Shot or let the Workflow engine dispatch
              an issue.
            </p>
          ) : (
            <ul className="graph-run-list">
              {recent.map((r) => (
                <li key={r.runId} className="graph-run-item">
                  <span className={`graph-run-icon ${statusClass(r.status)}`}>
                    {statusIcon(r.status)}
                  </span>
                  <div className="graph-run-body">
                    <div className="graph-run-top">
                      <span className="graph-run-repo">{r.repoName}</span>
                      <span className={`graph-run-status ${statusClass(r.status)}`}>
                        {r.status}
                      </span>
                    </div>
                    <div className="graph-run-meta">
                      <span>{r.toolCallCount} tool calls</span>
                      <span className="graph-run-dot">·</span>
                      <span>{formatCost(r.totalCostUsd)}</span>
                      <span className="graph-run-dot">·</span>
                      <span className="graph-run-time">{timeAgo(r.startedAt)}</span>
                    </div>
                    <div className="graph-run-id">{r.runId}</div>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </section>

        {/* ── Right: Blocking ── */}
        <section className="graph-panel">
          <div className="graph-panel-header">
            <span className="graph-panel-title">Blocking</span>
            {blocking.length > 0 && (
              <span className="graph-block-count">{blocking.length} blocked</span>
            )}
          </div>

          {loading && blocking.length === 0 ? (
            <p className="graph-empty">Loading…</p>
          ) : blocking.length === 0 ? (
            <p className="graph-empty">No blocking dependencies detected.</p>
          ) : (
            <ul className="graph-block-list">
              {blocking.map((item) => (
                <li key={item.issueId} className="graph-block-item">
                  <div className="graph-block-header">
                    <span className="graph-block-num">
                      {issueNumber(item.issueId)}
                    </span>
                    <span className="graph-block-title">
                      {item.title ?? item.issueId}
                    </span>
                  </div>
                  <div className="graph-block-deps">
                    <span className="graph-block-by-label">blocked by</span>
                    <ul className="graph-block-dep-list">
                      {item.blockedBy.map((b) => (
                        <li key={b.issueId} className="graph-block-dep">
                          <span className="graph-block-dep-num">
                            {issueNumber(b.issueId)}
                          </span>
                          {b.title && (
                            <span className="graph-block-dep-title">
                              {b.title}
                            </span>
                          )}
                        </li>
                      ))}
                    </ul>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </section>
      </div>
    </div>
  );
}
