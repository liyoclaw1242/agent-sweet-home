import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Repo } from "./Sidebar";
import "./HomeView.css";

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

interface IssueLabel {
  name: string;
  color: string;
}

interface Issue {
  id: number;
  number: number;
  title: string;
  htmlUrl: string;
  labels: IssueLabel[];
}

interface PullRequest {
  id: number;
  number: number;
  title: string;
  htmlUrl: string;
  draft: boolean;
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

interface DispatchEntry {
  id: number;
  issueNumber: number;
  repoFullName: string;
  matchedAt: number;
  ruleIndex: number | null;
  directiveType: string;
  directiveJson: string;
  runId: string | null;
}

interface DirectiveJson {
  directive: string;
  role?: string;
  mode?: string;
  reason?: string;
}

const REFRESH_MS = 15 * 60 * 1000;

interface Props {
  repo: Repo;
}

// ── Dispatch log helpers ──────────────────────────────────────────────────────

function parseDirective(json: string): DirectiveJson {
  try { return JSON.parse(json) as DirectiveJson; }
  catch { return { directive: "unknown" }; }
}

function directiveCls(type: string): string {
  if (type === "spawn_fresh")  return "dl-spawn";
  if (type === "wait")         return "dl-wait";
  if (type === "human_review") return "dl-human";
  return "dl-noop";
}

function directiveLabel(type: string): string {
  if (type === "spawn_fresh")  return "spawn";
  if (type === "no_action")    return "skip";
  if (type === "wait")         return "wait";
  if (type === "human_review") return "human";
  return type;
}

function timeAgo(ts: number): string {
  const sec = Math.floor(Date.now() / 1000) - ts;
  if (sec < 60)   return `${sec}s ago`;
  if (sec < 3600) return `${Math.floor(sec / 60)}m ago`;
  if (sec < 86400) return `${Math.floor(sec / 3600)}h ago`;
  return `${Math.floor(sec / 86400)}d ago`;
}

// ── Component ─────────────────────────────────────────────────────────────────

export default function HomeView({ repo }: Props) {
  const [inspection, setInspection] = useState<LocalInspection | null>(null);
  const [inspectionError, setInspectionError] = useState<string | null>(null);
  const [issues, setIssues] = useState<Issue[]>([]);
  const [issuesError, setIssuesError] = useState<string | null>(null);
  const [prs, setPrs] = useState<PullRequest[]>([]);
  const [prsError, setPrsError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [lastRefreshed, setLastRefreshed] = useState<Date | null>(null);
  const [dispatchLog, setDispatchLog] = useState<DispatchEntry[]>([]);
  const [blocking, setBlocking] = useState<BlockingItem[]>([]);

  const runInspection = useCallback(async () => {
    setInspectionError(null);
    try {
      const result = await invoke<LocalInspection>("inspect_local_repo", {
        repoName: repo.name,
      });
      setInspection(result);
    } catch (e) {
      setInspectionError(e instanceof Error ? e.message : String(e));
    }
  }, [repo.name]);

  const refreshGithub = useCallback(async () => {
    setRefreshing(true);
    setIssuesError(null);
    setPrsError(null);
    const [iRes, pRes] = await Promise.allSettled([
      invoke<Issue[]>("fetch_issues", { repoFullName: repo.fullName }),
      invoke<PullRequest[]>("fetch_prs", { repoFullName: repo.fullName }),
    ]);
    if (iRes.status === "fulfilled") setIssues(iRes.value ?? []);
    else setIssuesError(iRes.reason instanceof Error ? iRes.reason.message : String(iRes.reason));
    if (pRes.status === "fulfilled") setPrs(pRes.value ?? []);
    else setPrsError(pRes.reason instanceof Error ? pRes.reason.message : String(pRes.reason));
    setLastRefreshed(new Date());
    setRefreshing(false);
  }, [repo.fullName]);

  const loadDispatchLog = useCallback(async () => {
    try {
      const entries = await invoke<DispatchEntry[]>("dispatch_log_recent_cmd", {
        repoFullName: repo.fullName,
      });
      setDispatchLog(entries ?? []);
    } catch {
      // non-critical
    }
  }, [repo.fullName]);

  const loadBlocking = useCallback(async () => {
    try {
      const items = await invoke<BlockingItem[]>("graph_blocking_cmd");
      // filter to this repo only
      setBlocking(
        (items ?? []).filter((b) => b.issueId.startsWith(repo.fullName + "#")),
      );
    } catch {
      // non-critical
    }
  }, [repo.fullName]);

  useEffect(() => {
    void runInspection();
    void refreshGithub();
    void loadDispatchLog();
    void loadBlocking();
    const id = window.setInterval(() => { void refreshGithub(); }, REFRESH_MS);
    return () => { window.clearInterval(id); };
  }, [runInspection, refreshGithub, loadDispatchLog]);

  // Live update: workflow engine emits "dispatch:logged" after each decision.
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    listen<string>("dispatch:logged", (ev) => {
      if (ev.payload === repo.fullName) void loadDispatchLog();
    }).then((fn) => { unlisten = fn; }).catch(() => {});
    return () => { unlisten?.(); };
  }, [repo.fullName, loadDispatchLog]);

  function refreshAll() {
    void runInspection();
    void refreshGithub();
    void loadDispatchLog();
    void loadBlocking();
  }

  return (
    <section className="home">
      <header className="home-header">
        <div>
          <h2>{repo.name}</h2>
          <p className="repo-meta">{repo.fullName}</p>
        </div>
        <button type="button" onClick={refreshAll} disabled={refreshing} aria-label="Refresh now">
          {refreshing ? "Refreshing…" : "Refresh"}
        </button>
      </header>

      <dl className="info-grid">
        <dt>Repo</dt>
        <dd>{repo.name}</dd>

        <dt>Default branch</dt>
        <dd><code>{repo.defaultBranch}</code></dd>

        <dt>Local path</dt>
        <dd>
          {inspection ? (
            <>
              <code>{inspection.repoPath}</code>
              {!inspection.exists && <span className="badge badge-warn">missing</span>}
              {inspection.exists && !inspection.isGitRepo && <span className="badge badge-warn">not a git repo</span>}
            </>
          ) : "Loading…"}
          {inspectionError && <span className="badge badge-error">{inspectionError}</span>}
          {inspection?.error && <span className="badge badge-error">{inspection.error}</span>}
        </dd>

        <dt>Local branch</dt>
        <dd>
          {inspection?.exists && inspection.isGitRepo ? (
            <>
              <code>{inspection.currentBranch ?? "(unknown)"}</code>
              {inspection.isClean === true && <span className="badge badge-ok">clean</span>}
              {inspection.isClean === false && inspection.dirtyFiles != null && (
                <span className="badge badge-warn">
                  {inspection.dirtyFiles} change{inspection.dirtyFiles !== 1 ? "s" : ""}
                </span>
              )}
            </>
          ) : (
            <span className="muted">—</span>
          )}
        </dd>
      </dl>

      <section className="lists">
        <article>
          <h3>Open issues ({issues.length})</h3>
          {issuesError && <p className="error">{issuesError}</p>}
          {!issuesError && issues.length === 0 && <p className="empty">No open issues.</p>}
          {issues.length > 0 && (
            <ul>
              {issues.map((i) => (
                <li key={i.id}>
                  <span className="num">#{i.number}</span>
                  <span className="title" title={i.title}>{i.title}</span>
                  {i.labels.length > 0 && (
                    <span className="labels">
                      {i.labels.map((l) => (
                        <span key={l.name} className="lbl"
                          style={{ backgroundColor: `#${l.color}`, color: textColorForBg(l.color) }}>
                          {l.name}
                        </span>
                      ))}
                    </span>
                  )}
                </li>
              ))}
            </ul>
          )}
        </article>

        <article>
          <h3>Open PRs ({prs.length})</h3>
          {prsError && <p className="error">{prsError}</p>}
          {!prsError && prs.length === 0 && <p className="empty">No open PRs.</p>}
          {prs.length > 0 && (
            <ul>
              {prs.map((p) => (
                <li key={p.id}>
                  <span className="num">#{p.number}</span>
                  <span className="title" title={p.title}>{p.title}</span>
                  {p.draft && <span className="badge">draft</span>}
                </li>
              ))}
            </ul>
          )}
        </article>
      </section>

      {/* ── Blocking Graph ── */}
      {blocking.length > 0 && (
        <section className="blocking-section">
          <h3 className="dispatch-log-title">
            Blocked Issues
            <span className="dispatch-log-count">{blocking.length}</span>
          </h3>
          <ul className="blocking-list">
            {blocking.map((item) => {
              const num = item.issueId.split("#")[1] ?? item.issueId;
              return (
                <li key={item.issueId} className="blocking-item">
                  <span className="blocking-issue">#{num}</span>
                  <span className="blocking-title">{item.title ?? item.issueId}</span>
                  <span className="blocking-by">blocked by</span>
                  <span className="blocking-deps">
                    {item.blockedBy.map((b) => {
                      const bn = b.issueId.split("#")[1] ?? b.issueId;
                      return (
                        <span key={b.issueId} className="blocking-dep" title={b.title ?? b.issueId}>
                          #{bn}
                        </span>
                      );
                    })}
                  </span>
                </li>
              );
            })}
          </ul>
        </section>
      )}

      {/* ── Dispatch Log ── */}
      <section className="dispatch-log-section">
        <h3 className="dispatch-log-title">
          Dispatch Log
          {dispatchLog.length > 0 && (
            <span className="dispatch-log-count">{dispatchLog.length}</span>
          )}
        </h3>
        {dispatchLog.length === 0 ? (
          <p className="empty">No dispatch decisions recorded yet.</p>
        ) : (
          <ul className="dispatch-log-list">
            {dispatchLog.map((entry) => {
              const dir = parseDirective(entry.directiveJson);
              return (
                <li key={entry.id} className="dispatch-log-item">
                  <span className={`dl-badge ${directiveCls(entry.directiveType)}`}>
                    {directiveLabel(entry.directiveType)}
                  </span>
                  <span className="dl-issue">#{entry.issueNumber}</span>
                  {entry.ruleIndex != null && (
                    <span className="dl-rule">rule {entry.ruleIndex}</span>
                  )}
                  {dir.role && (
                    <span className="dl-role">{dir.role}{dir.mode ? ` / ${dir.mode}` : ""}</span>
                  )}
                  {dir.reason && (
                    <span className="dl-reason" title={dir.reason}>
                      {dir.reason.length > 60 ? `${dir.reason.slice(0, 60)}…` : dir.reason}
                    </span>
                  )}
                  {entry.runId && (
                    <span className="dl-run-id" title={entry.runId}>
                      → {entry.runId.slice(0, 12)}…
                    </span>
                  )}
                  <span className="dl-time">{timeAgo(entry.matchedAt)}</span>
                </li>
              );
            })}
          </ul>
        )}
      </section>

      {lastRefreshed && (
        <footer className="home-footer">
          Last refreshed {lastRefreshed.toLocaleTimeString()} · auto every 15 min
        </footer>
      )}
    </section>
  );
}

function textColorForBg(hex: string): string {
  if (hex.length < 6) return "#000";
  const r = parseInt(hex.substring(0, 2), 16);
  const g = parseInt(hex.substring(2, 4), 16);
  const b = parseInt(hex.substring(4, 6), 16);
  return (r * 299 + g * 587 + b * 114) / 1000 >= 128 ? "#000" : "#fff";
}
