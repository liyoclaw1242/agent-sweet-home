import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
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

const REFRESH_MS = 15 * 60 * 1000;

interface Props {
  repo: Repo;
}

export default function HomeView({ repo }: Props) {
  const [inspection, setInspection] = useState<LocalInspection | null>(null);
  const [inspectionError, setInspectionError] = useState<string | null>(null);
  const [issues, setIssues] = useState<Issue[]>([]);
  const [issuesError, setIssuesError] = useState<string | null>(null);
  const [prs, setPrs] = useState<PullRequest[]>([]);
  const [prsError, setPrsError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [lastRefreshed, setLastRefreshed] = useState<Date | null>(null);

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
    if (iRes.status === "fulfilled") {
      setIssues(iRes.value ?? []);
    } else {
      setIssuesError(
        iRes.reason instanceof Error ? iRes.reason.message : String(iRes.reason),
      );
    }
    if (pRes.status === "fulfilled") {
      setPrs(pRes.value ?? []);
    } else {
      setPrsError(
        pRes.reason instanceof Error ? pRes.reason.message : String(pRes.reason),
      );
    }
    setLastRefreshed(new Date());
    setRefreshing(false);
  }, [repo.fullName]);

  useEffect(() => {
    void runInspection();
    void refreshGithub();
    const id = window.setInterval(() => {
      void refreshGithub();
    }, REFRESH_MS);
    return () => {
      window.clearInterval(id);
    };
  }, [runInspection, refreshGithub]);

  function refreshAll() {
    void runInspection();
    void refreshGithub();
  }

  return (
    <section className="home">
      <header className="home-header">
        <div>
          <h2>{repo.name}</h2>
          <p className="repo-meta">{repo.fullName}</p>
        </div>
        <button
          type="button"
          onClick={refreshAll}
          disabled={refreshing}
          aria-label="Refresh now"
        >
          {refreshing ? "Refreshing…" : "Refresh"}
        </button>
      </header>

      <dl className="info-grid">
        <dt>Repo</dt>
        <dd>{repo.name}</dd>

        <dt>Default branch</dt>
        <dd>
          <code>{repo.defaultBranch}</code>
        </dd>

        <dt>Local path</dt>
        <dd>
          {inspection ? (
            <>
              <code>{inspection.repoPath}</code>
              {!inspection.exists && (
                <span className="badge badge-warn">missing</span>
              )}
              {inspection.exists && !inspection.isGitRepo && (
                <span className="badge badge-warn">not a git repo</span>
              )}
            </>
          ) : (
            "Loading…"
          )}
          {inspectionError && (
            <span className="badge badge-error">{inspectionError}</span>
          )}
          {inspection?.error && (
            <span className="badge badge-error">{inspection.error}</span>
          )}
        </dd>

        <dt>Local branch</dt>
        <dd>
          {inspection?.exists && inspection.isGitRepo ? (
            <>
              <code>{inspection.currentBranch ?? "(unknown)"}</code>
              {inspection.isClean === true && (
                <span className="badge badge-ok">clean</span>
              )}
              {inspection.isClean === false && inspection.dirtyFiles != null && (
                <span className="badge badge-warn">
                  {inspection.dirtyFiles} change
                  {inspection.dirtyFiles !== 1 ? "s" : ""}
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
          {!issuesError && issues.length === 0 && (
            <p className="empty">No open issues.</p>
          )}
          {issues.length > 0 && (
            <ul>
              {issues.map((i) => (
                <li key={i.id}>
                  <span className="num">#{i.number}</span>
                  <span className="title" title={i.title}>
                    {i.title}
                  </span>
                  {i.labels.length > 0 && (
                    <span className="labels">
                      {i.labels.map((l) => (
                        <span
                          key={l.name}
                          className="lbl"
                          style={{
                            backgroundColor: `#${l.color}`,
                            color: textColorForBg(l.color),
                          }}
                        >
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
          {!prsError && prs.length === 0 && (
            <p className="empty">No open PRs.</p>
          )}
          {prs.length > 0 && (
            <ul>
              {prs.map((p) => (
                <li key={p.id}>
                  <span className="num">#{p.number}</span>
                  <span className="title" title={p.title}>
                    {p.title}
                  </span>
                  {p.draft && <span className="badge">draft</span>}
                </li>
              ))}
            </ul>
          )}
        </article>
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
  const yiq = (r * 299 + g * 587 + b * 114) / 1000;
  return yiq >= 128 ? "#000" : "#fff";
}
