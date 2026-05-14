import { useEffect, useRef } from "react";
import "./DocsModal.css";

interface Props {
  onClose: () => void;
}

function Method({ m }: { m: "GET" | "POST" | "DELETE" }) {
  return <span className={`dm-method dm-${m}`}>{m}</span>;
}

function Route({
  method, path, auth = true, desc, children,
}: {
  method: "GET" | "POST" | "DELETE";
  path: string;
  auth?: boolean;
  desc: string;
  children?: React.ReactNode;
}) {
  return (
    <details className="dm-route">
      <summary className="dm-route-head">
        <Method m={method} />
        <span className="dm-path">{path}</span>
        {auth
          ? <span className="dm-auth-req">bearer</span>
          : <span className="dm-no-auth">no auth</span>}
        <span className="dm-route-desc">{desc}</span>
      </summary>
      {children && <div className="dm-route-body">{children}</div>}
    </details>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="dm-section">
      <h3 className="dm-section-title">{title}</h3>
      {children}
    </section>
  );
}

function Pre({ children }: { children: string }) {
  return <pre className="dm-pre">{children.trim()}</pre>;
}

export default function DocsModal({ onClose }: Props) {
  const backdropRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div
      className="dm-backdrop"
      ref={backdropRef}
      onClick={(e) => { if (e.target === backdropRef.current) onClose(); }}
    >
      <div className="dm-modal" role="dialog" aria-modal aria-label="API Reference">
        <header className="dm-modal-header">
          <div>
            <div className="dm-modal-eyebrow">agent-sweet-home · localhost HTTP API</div>
            <h2 className="dm-modal-title">HTTP API Reference</h2>
          </div>
          <button type="button" className="dm-close" onClick={onClose} aria-label="Close">×</button>
        </header>

        <div className="dm-modal-body">
          <div className="dm-auth-box">
            <div className="dm-auth-box-title">Auth</div>
            <ul>
              <li>除 <code>GET /health</code> 外，所有端點需帶 <code>Authorization: Bearer &lt;token&gt;</code></li>
              <li>Port + token 在 <code>&lt;app_data_dir&gt;/server.json</code>，每次 App 啟動重新生成</li>
              <li>Token 錯誤 → <code>401 Unauthorized</code></li>
            </ul>
          </div>

          {/* System */}
          <Section title="System">
            <Route method="GET" path="/health" auth={false} desc="Liveness check">
              <Pre>{"200 OK  →  \"ok\""}</Pre>
            </Route>
          </Section>

          {/* Repos */}
          <Section title="Repos">
            <Route method="GET" path="/repos" desc="List all cached GitHub repos">
              <Pre>{`200 OK  →  Repo[]
{
  id, name, fullName, description, htmlUrl,
  private, defaultBranch, stargazersCount,
  language, updatedAt
}`}</Pre>
            </Route>
            <Route method="GET" path="/repos/{name}" desc="Repo detail: issues, PRs, local git state">
              <Pre>{`Path: name — repo short name (e.g. "my-repo")

200 OK  →  RepoDetail
{
  repo: Repo,
  issues: [{ id, number, title, htmlUrl, labels }],
  prs:    [{ id, number, title, htmlUrl, draft }],
  local: { repoPath, exists, isGitRepo,
           currentBranch, isClean, dirtyFiles, error }
}
404 Not Found — repo not in cache`}</Pre>
            </Route>
          </Section>

          {/* Sessions */}
          <Section title="Sessions (Persistent PTY)">
            <Route method="GET" path="/sessions" desc="Active PTY sessions (in-memory, cleared on restart)">
              <Pre>{`Query: repo (string, optional)  repoId (int, optional)

200 OK  →  SessionInfo[]
{
  id, repoName, repoId,
  exitCode,   // null = still running
  frozen,     // true if no output > 15 min
  uptimeSecs,
  lastOutputAt
}`}</Pre>
            </Route>
          </Section>

          {/* One-Shot */}
          <Section title="One-Shot Runs">
            <Route method="GET" path="/one-shot" desc="List runs (SQLite-persisted)">
              <Pre>{`Query: repo (string)  repoId (int)
       status: running | completed | failed | killed

200 OK  →  RunInfo[]`}</Pre>
            </Route>
            <Route method="POST" path="/one-shot" desc="Start a new claude -p run">
              <Pre>{`Body (JSON):
  repoId*     int       DB repo id
  repoName*   string    Repo short name
  cwd*        string    Working directory
  prompt*     string    Prompt text
  model       string    Model override
  outputFormat  stream-json | json | text  (default: stream-json)
  skipPermissions  boolean  --dangerously-skip-permissions
  systemPrompt     string
  appendSystemPrompt string
  allowedTools     string[]
  disallowedTools  string[]
  maxBudgetUsd     number
  addDir           string[]
  name             string   --name flag (workflow: {role}-{mode}-issue{N})
  mcpConfig        string[]
  resume           string   Session UUID
  continueLast     boolean  --continue
  extraArgs        string[]

200 OK  →  RunInfo
400 Bad Request — cwd missing or spawn failed
503 Service Unavailable — app not ready`}</Pre>
            </Route>
            <Route method="GET" path="/one-shot/{id}" desc="Single run metadata">
              <Pre>{"200 OK  →  RunInfo\n404 Not Found"}</Pre>
            </Route>
            <Route method="DELETE" path="/one-shot/{id}" desc="Kill (running) or delete (finished)">
              <Pre>{`204 No Content  — deleted (cascades log lines)
202 Accepted    — SIGKILL sent (was running)
404 Not Found`}</Pre>
            </Route>
            <Route method="GET" path="/one-shot/{id}/log" desc="Stream log lines (incremental)">
              <Pre>{`Query: since (int, default -1)   return lines with seq > since
       limit (int, default 1000)

200 OK  →  LogLine[]
{
  runId, seq, ts,
  stream: "stdout" | "stderr",
  text    // stream-json: each stdout line is a Claude event JSON
}

Tip: poll with since=<last_seq> for incremental updates`}</Pre>
            </Route>
          </Section>

          {/* Workflow */}
          <Section title="Workflow">
            <Route method="GET" path="/workflow" desc="Workflow engine status">
              <Pre>{`200 OK  →
{
  path: "/path/to/workflow.yaml",
  exists: true,
  loaded: true,
  error: null
}`}</Pre>
            </Route>
            <Route method="POST" path="/workflow/path" desc="Set workflow YAML path (requires restart)">
              <Pre>{`Body: { "path": "/absolute/path/to/workflow.yaml" }

200 OK  →
{
  ok: true,
  path: "...",
  note: "Restart the app for the new workflow path to take effect."
}`}</Pre>
            </Route>
          </Section>

          {/* Graph */}
          <Section title="Graph / Digital Twin">
            <Route method="GET" path="/graph/state" desc="All runs with agent label and issue link">
              <Pre>{`200 OK  →  GraphState
{
  runs: [{
    runId, repoName, status,
    startedAt, endedAt, totalCostUsd,
    eventCount, toolCallCount,
    agentLabel,   // "worker" | "whitebox-validator" | ... | null
    issueNumber   // null for manual runs
  }]
}`}</Pre>
            </Route>
            <Route method="GET" path="/graph/runs/{id}/decisions" desc="Structured call chain events for a run">
              <Pre>{`200 OK  →  RunEvent[]
{
  id, runId, seq, ts,
  eventType: "thinking" | "tool_use" | "tool_result" | "result",
  toolName, toolUseId,
  inputJson, outputJson, thinking,
  isError
}
Parsed lazily from stream-json log; historical runs supported.`}</Pre>
            </Route>
            <Route method="GET" path="/graph/issues/{n}/why" desc="Dispatch decision history for an issue">
              <Pre>{`Query: repo* (string)  e.g. "org/repo"

200 OK  →  DispatchEntry[]
{
  id, issueNumber, repoFullName, matchedAt,
  ruleIndex,      // 0-based; null = no rule matched
  directiveType,  // "spawn_fresh" | "no_action" | "wait" | "human_review"
  directiveJson,  // full directive including role and reason
  runId           // null if no spawn
}
Answers: "why was this issue dispatched (or not)?"  `}</Pre>
            </Route>
            <Route method="GET" path="/graph/issues/{n}/trace" desc="Full causal trace: issue → dispatches → runs → events">
              <Pre>{`Query: repo* (string)

200 OK  →  IssueTrace
{
  issueNumber, repoFullName,
  dispatches: [{
    dispatch: DispatchEntry,
    run: RunInfo | null,
    decisions: RunEvent[]
  }]
}`}</Pre>
            </Route>
            <Route method="GET" path="/graph/blocking" desc="All issues blocked by unresolved deps">
              <Pre>{`200 OK  →  BlockingItem[]
{
  issueId: "org/repo#7",
  title: "...",
  blockedBy: [{ issueId: "org/repo#5", title: "..." }]
}`}</Pre>
            </Route>
          </Section>
        </div>
      </div>
    </div>
  );
}
