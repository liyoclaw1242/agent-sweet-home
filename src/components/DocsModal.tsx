import { useEffect, useRef } from "react";
import "./DocsModal.css";

interface Props { onClose: () => void; }

// ── Primitive helpers ─────────────────────────────────────────────────────────

function Method({ m }: { m: "GET" | "POST" | "DELETE" }) {
  return <span className={`dm-method dm-${m}`}>{m}</span>;
}

function Code({ children }: { children: React.ReactNode }) {
  return <code className="dm-code">{children}</code>;
}

function StatusBadge({ code }: { code: number }) {
  const cls = code < 300 ? "dm-s2" : code < 400 ? "dm-s3" : code < 500 ? "dm-s4" : "dm-s5";
  return <span className={`dm-status-badge ${cls}`}>{code}</span>;
}

// ── Param table ───────────────────────────────────────────────────────────────

interface Param {
  name: string;
  in: "path" | "query" | "body";
  type: string;
  required?: boolean;
  description: string;
  example?: string;
}

function ParamTable({ params }: { params: Param[] }) {
  return (
    <table className="dm-param-table">
      <thead>
        <tr>
          <th>Name</th>
          <th>In</th>
          <th>Type</th>
          <th>Required</th>
          <th>Description</th>
        </tr>
      </thead>
      <tbody>
        {params.map((p) => (
          <tr key={p.name}>
            <td className="dm-pname">{p.name}</td>
            <td><span className={`dm-pin dm-pin-${p.in}`}>{p.in}</span></td>
            <td className="dm-ptype">{p.type}</td>
            <td className={p.required ? "dm-req" : "dm-opt"}>{p.required ? "●" : "○"}</td>
            <td className="dm-pdesc">
              {p.description}
              {p.example && <> &nbsp;<span className="dm-example">e.g. <Code>{p.example}</Code></span></>}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

// ── Response block ────────────────────────────────────────────────────────────

interface Response {
  code: number;
  description: string;
  schema?: string;
}

function Responses({ items }: { items: Response[] }) {
  return (
    <div className="dm-responses">
      {items.map((r) => (
        <div key={r.code} className="dm-response">
          <div className="dm-response-head">
            <StatusBadge code={r.code} />
            <span className="dm-response-desc">{r.description}</span>
          </div>
          {r.schema && <pre className="dm-pre dm-schema">{r.schema.trim()}</pre>}
        </div>
      ))}
    </div>
  );
}

// ── Route card ────────────────────────────────────────────────────────────────

function Route({
  method, path, auth = true, summary, description, params, responses, children,
}: {
  method: "GET" | "POST" | "DELETE";
  path: string;
  auth?: boolean;
  summary: string;
  description?: string;
  params?: Param[];
  responses?: Response[];
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
        <span className="dm-route-summary">{summary}</span>
      </summary>
      <div className="dm-route-body">
        {description && <p className="dm-description">{description}</p>}
        {params && params.length > 0 && (
          <div className="dm-block">
            <div className="dm-block-label">Parameters</div>
            <ParamTable params={params} />
          </div>
        )}
        {children && <div className="dm-block">{children}</div>}
        {responses && (
          <div className="dm-block">
            <div className="dm-block-label">Responses</div>
            <Responses items={responses} />
          </div>
        )}
      </div>
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

// ── Component ─────────────────────────────────────────────────────────────────

export default function DocsModal({ onClose }: Props) {
  const backdropRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    function onKey(e: KeyboardEvent) { if (e.key === "Escape") onClose(); }
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
            <div className="dm-modal-eyebrow">agent-sweet-home · v1 · axum · 127.0.0.1:&lt;port&gt;</div>
            <h2 className="dm-modal-title">HTTP API Reference</h2>
          </div>
          <button type="button" className="dm-close" onClick={onClose} aria-label="Close">×</button>
        </header>

        <div className="dm-modal-body">

          {/* Auth */}
          <div className="dm-auth-box">
            <div className="dm-auth-box-title">Authentication</div>
            <p>
              All endpoints except <Code>GET /health</Code> require{" "}
              <Code>Authorization: Bearer &lt;token&gt;</Code>.
              The server binds to a random OS-assigned port on startup and writes both{" "}
              <Code>port</Code> and <Code>token</Code> to{" "}
              <Code>&lt;app_data_dir&gt;/server.json</Code> (mode 0600 on Unix).
              The token is re-generated on every app restart. Missing or incorrect token returns{" "}
              <Code>401 Unauthorized</Code>.
            </p>
          </div>

          {/* ── System ── */}
          <Section title="System">
            <Route
              method="GET" path="/health" auth={false}
              summary="Liveness probe"
              description="Returns a plain-text 'ok'. No authentication required. Useful for checking if the server is up before reading server.json."
              responses={[
                { code: 200, description: "Server is up", schema: `"ok"` },
              ]}
            />
          </Section>

          {/* ── Repos ── */}
          <Section title="Repos">
            <Route
              method="GET" path="/repos"
              summary="List all cached GitHub repos"
              description="Returns the list of GitHub repositories fetched and cached in SQLite. The cache is updated whenever the UI refreshes repos (on startup and on demand). Does not make live GitHub API calls."
              responses={[
                { code: 200, description: "Array of cached repos", schema:
`[
  {
    "id":              1,
    "name":            "my-repo",
    "fullName":        "org/my-repo",
    "description":     "A short description",
    "htmlUrl":         "https://github.com/org/my-repo",
    "private":         false,
    "defaultBranch":   "main",
    "stargazersCount": 42,
    "language":        "TypeScript",
    "updatedAt":       "2026-05-14T00:00:00Z"
  }
]` },
              ]}
            />
            <Route
              method="GET" path="/repos/{name}"
              summary="Repo detail with issues, PRs, and local git state"
              description="Returns combined repo metadata, its cached open issues and PRs, plus a live inspection of the local clone (branch, clean state, dirty file count). The local inspection reads the filesystem at request time; issues/PRs come from SQLite cache."
              params={[
                { name: "name", in: "path", type: "string", required: true, description: "Repo short name (not full name)", example: "my-repo" },
              ]}
              responses={[
                { code: 200, description: "Full detail object", schema:
`{
  "repo": { /* Repo — see GET /repos */ },
  "issues": [
    {
      "id":      101,
      "number":  42,
      "title":   "Fix the thing",
      "htmlUrl": "https://github.com/org/my-repo/issues/42",
      "labels":  [{ "name": "bug", "color": "d73a4a" }]
    }
  ],
  "prs": [
    {
      "id":      202,
      "number":  7,
      "title":   "feat: add thing",
      "htmlUrl": "https://github.com/org/my-repo/pull/7",
      "draft":   false
    }
  ],
  "local": {
    "configuredBasePath": "/Users/you/Projects",
    "repoPath":           "/Users/you/Projects/my-repo",
    "exists":             true,
    "isGitRepo":          true,
    "currentBranch":      "main",
    "isClean":            true,
    "dirtyFiles":         0,
    "error":              null
  }
}` },
                { code: 404, description: "Repo not found in SQLite cache. Trigger a repo refresh from the UI first." },
              ]}
            />
          </Section>

          {/* ── Sessions ── */}
          <Section title="Sessions (Persistent PTY)">
            <Route
              method="GET" path="/sessions"
              summary="List active PTY sessions"
              description="Returns all live or recently-exited PTY sessions managed by the app. Sessions are in-memory only — they are cleared when the app restarts. Each session corresponds to one Persistent terminal tile in the UI."
              params={[
                { name: "repo",   in: "query", type: "string",  description: "Filter by repo short name" },
                { name: "repoId", in: "query", type: "integer", description: "Filter by repo DB id" },
              ]}
              responses={[
                { code: 200, description: "Array of session snapshots", schema:
`[
  {
    "id":           "my-repo-1715000000-ab12cd34",
    "repoName":     "my-repo",
    "repoId":       1,
    "exitCode":     null,    // null = still running; integer = exited
    "frozen":       false,   // true if no stdout/stderr output in last 15 min
    "uptimeSecs":   142,
    "lastOutputAt": 1715000142
  }
]` },
              ]}
            />
          </Section>

          {/* ── One-Shot ── */}
          <Section title="One-Shot Runs">
            <Route
              method="GET" path="/one-shot"
              summary="List claude -p runs"
              description="Returns all one-shot runs persisted in SQLite. Runs survive app restarts. Supports filtering by repo or status. The workflow engine uses this endpoint to monitor running agents."
              params={[
                { name: "repo",   in: "query", type: "string",  description: "Filter by repo short name" },
                { name: "repoId", in: "query", type: "integer", description: "Filter by repo DB id" },
                { name: "status", in: "query", type: "string",  description: "Filter: running | completed | failed | killed" },
              ]}
              responses={[
                { code: 200, description: "Array of RunInfo", schema:
`[
  {
    "id":           "my-repo-1715000000-ab12cd34",
    "repoId":       1,
    "repoName":     "my-repo",
    "cwd":          "/Users/you/Projects/my-repo",
    "prompt":       "Implement feature X according to spec #7",
    "argv":         ["claude", "-p", "--output-format", "stream-json", "..."],
    "status":       "completed",   // running | completed | failed | killed
    "startedAt":    1715000000,
    "endedAt":      1715000120,
    "exitCode":     0,
    "totalCostUsd": 0.0234,
    "outputFormat": "stream-json"
  }
]` },
              ]}
            />

            <Route
              method="POST" path="/one-shot"
              summary="Start a new claude -p run"
              description="Spawns a new claude -p process. The process runs in cwd with stdout/stderr streamed to SQLite line-by-line, emitting Tauri events per line. Returns the created RunInfo immediately; the run continues asynchronously. Returns 503 if the app handle is not yet initialised (first few ms after startup)."
              params={[
                { name: "repoId",            in: "body", type: "integer",  required: true,  description: "DB id of the repo this run belongs to" },
                { name: "repoName",          in: "body", type: "string",   required: true,  description: "Repo short name, used to construct the run id" },
                { name: "cwd",               in: "body", type: "string",   required: true,  description: "Absolute working directory for the claude process" },
                { name: "prompt",            in: "body", type: "string",   required: true,  description: "Prompt passed as the positional argument to claude -p" },
                { name: "model",             in: "body", type: "string",   description: "Model override, e.g. claude-opus-4-7. Defaults to claude's configured default." },
                { name: "outputFormat",      in: "body", type: "string",   description: "stream-json (default) | json | text. stream-json enables per-event parsing and structured output extraction." },
                { name: "skipPermissions",   in: "body", type: "boolean",  description: "--dangerously-skip-permissions flag. Required for non-interactive automation." },
                { name: "permissionMode",    in: "body", type: "string",   description: "default | acceptEdits | plan | bypassPermissions | dontAsk | auto" },
                { name: "effort",            in: "body", type: "string",   description: "low | medium | high | xhigh | max — thinking budget hint" },
                { name: "verbose",           in: "body", type: "boolean",  description: "--verbose flag. Recommended for stream-json runs to get full event stream." },
                { name: "systemPrompt",      in: "body", type: "string",   description: "Full system prompt, replaces default. Used by workflow engine to inject role skill content." },
                { name: "appendSystemPrompt",in: "body", type: "string",   description: "Appended to default system prompt without replacing it." },
                { name: "allowedTools",      in: "body", type: "string[]", description: "Tool whitelist. e.g. [\"Bash\", \"Read\", \"Edit\"]" },
                { name: "disallowedTools",   in: "body", type: "string[]", description: "Tool blacklist. Applied after allowedTools." },
                { name: "maxBudgetUsd",      in: "body", type: "number",   description: "Hard spend cap in USD. Claude stops when reached." },
                { name: "addDir",            in: "body", type: "string[]", description: "Extra directories added to context via --add-dir." },
                { name: "name",              in: "body", type: "string",   description: "--name display label. Workflow engine sets {role}-{mode}-issue{N}; used to reconstruct agent role in the twin." },
                { name: "mcpConfig",         in: "body", type: "string[]", description: "MCP config file absolute paths." },
                { name: "strictMcpConfig",   in: "body", type: "boolean",  description: "--strict-mcp-config flag." },
                { name: "resume",            in: "body", type: "string",   description: "Session UUID to resume (--resume flag)." },
                { name: "continueLast",      in: "body", type: "boolean",  description: "--continue: resume the last session in cwd." },
                { name: "forkSession",       in: "body", type: "boolean",  description: "--fork-session flag." },
                { name: "agent",             in: "body", type: "string",   description: "--agent flag override." },
                { name: "extraArgs",         in: "body", type: "string[]", description: "Raw extra flags appended to argv verbatim." },
              ]}
              responses={[
                { code: 200, description: "Run created successfully. Process is now running in background.", schema:
`{
  "id":           "my-repo-1715000000-ab12cd34",
  "repoId":       1,
  "repoName":     "my-repo",
  "cwd":          "/Users/you/Projects/my-repo",
  "prompt":       "Implement feature X",
  "argv":         ["claude", "-p", "--output-format", "stream-json", ...],
  "status":       "running",
  "startedAt":    1715000000,
  "endedAt":      null,
  "exitCode":     null,
  "totalCostUsd": null,
  "outputFormat": "stream-json"
}` },
                { code: 400, description: "cwd does not exist on disk, or the spawn syscall failed." },
                { code: 503, description: "App handle not ready. Retry after a few milliseconds." },
              ]}
            />

            <Route
              method="GET" path="/one-shot/{id}"
              summary="Get a single run"
              description="Fetches the current metadata snapshot of one run. Status and totalCostUsd update as the run progresses."
              params={[
                { name: "id", in: "path", type: "string", required: true, description: "Run id", example: "my-repo-1715000000-ab12cd34" },
              ]}
              responses={[
                { code: 200, description: "RunInfo — same shape as items in GET /one-shot" },
                { code: 404, description: "No run with this id in SQLite." },
              ]}
            />

            <Route
              method="DELETE" path="/one-shot/{id}"
              summary="Kill or delete a run"
              description="If the run is still in progress, sends SIGKILL and returns 202. If already finished, removes the run and all its log lines from SQLite and returns 204. The deletion cascades — log lines are deleted automatically."
              params={[
                { name: "id", in: "path", type: "string", required: true, description: "Run id" },
              ]}
              responses={[
                { code: 204, description: "Run and all log lines deleted from SQLite." },
                { code: 202, description: "SIGKILL sent. Run status will update to 'killed' asynchronously." },
                { code: 404, description: "No run with this id." },
              ]}
            />

            <Route
              method="GET" path="/one-shot/{id}/log"
              summary="Fetch log lines (supports incremental polling)"
              description="Returns stdout and stderr lines for a run in sequence order. For incremental polling, pass the seq of the last received line as the since parameter — only lines with seq > since are returned. For stream-json runs, each stdout line is a newline-delimited JSON event from the Claude stream."
              params={[
                { name: "id",    in: "path",  type: "string",  required: true,  description: "Run id" },
                { name: "since", in: "query", type: "integer", description: "Return only lines with seq > since. Default -1 (all lines).", example: "47" },
                { name: "limit", in: "query", type: "integer", description: "Maximum number of lines to return. Default 1000." },
              ]}
              responses={[
                { code: 200, description: "Array of log lines in seq order", schema:
`[
  {
    "runId":  "my-repo-1715000000-ab12cd34",
    "seq":    0,
    "ts":     1715000001,           // unix seconds
    "stream": "stdout",             // "stdout" | "stderr"
    "text":   "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"I'll start by...\"}]}}"
  },
  {
    "runId":  "my-repo-1715000000-ab12cd34",
    "seq":    1,
    "ts":     1715000002,
    "stream": "stderr",
    "text":   "Loaded MCP server: filesystem"
  }
]
// Incremental poll: GET /one-shot/{id}/log?since=1 → returns seq >= 2` },
              ]}
            />
          </Section>

          {/* ── Workflow ── */}
          <Section title="Workflow">
            <Route
              method="GET" path="/workflow"
              summary="Current workflow engine status"
              description="Returns the state of the loaded workflow YAML. The engine polls GitHub issues every 30 seconds when active. If the YAML has a parse error, loaded is false and error contains the message."
              responses={[
                { code: 200, description: "Workflow status snapshot", schema:
`{
  "path":   "/Users/you/agent-team/agent-team.workflow.yaml",
  "exists": true,
  "loaded": true,
  "error":  null    // parse error string if loaded is false
}` },
              ]}
            />

            <Route
              method="POST" path="/workflow/path"
              summary="Persist a new workflow YAML path"
              description="Writes the path to the settings table in SQLite. The workflow engine does NOT hot-reload — a full app restart is required for the change to take effect. Useful for CI scripts or remote reconfiguration."
              params={[
                { name: "path", in: "body", type: "string", required: true, description: "Absolute path to the workflow YAML file", example: "/Users/you/agent-team/agent-team.workflow.yaml" },
              ]}
              responses={[
                { code: 200, description: "Path saved to SQLite", schema:
`{
  "ok":   true,
  "path": "/Users/you/agent-team/agent-team.workflow.yaml",
  "note": "Restart the app for the new workflow path to take effect."
}` },
              ]}
            />
          </Section>

          {/* ── Graph ── */}
          <Section title="Graph / Digital Twin">
            <Route
              method="GET" path="/graph/state"
              summary="All runs enriched with agent role and triggered issue"
              description="Returns a summary of up to 200 most recent runs, each enriched with the agent role (derived from dispatch_log directive_json or the --name argv flag) and the GitHub issue number that triggered the spawn via the workflow engine. Useful for building external dashboards over the twin's state."
              responses={[
                { code: 200, description: "GraphState with run summaries", schema:
`{
  "runs": [
    {
      "runId":         "my-repo-1715000000-ab12cd34",
      "repoName":      "my-repo",
      "status":        "completed",
      "startedAt":     1715000000,
      "endedAt":       1715000120,
      "totalCostUsd":  0.0234,
      "eventCount":    48,        // total structured events (all types)
      "toolCallCount": 12,        // tool_use events only
      "agentLabel":    "worker",  // "worker" | "whitebox-validator" | "blackbox-validator" | "arbiter" | null
      "issueNumber":   7          // GitHub issue # that triggered this run; null for manual runs
    }
  ]
}` },
              ]}
            />

            <Route
              method="GET" path="/graph/runs/{id}/decisions"
              summary="Structured call chain events for a run"
              description="Returns the parsed event sequence for a run: thinking blocks, tool calls with their inputs, tool results, and the final result with cost. Events are parsed lazily from the raw stream-json log on first request and cached in run_events. Historical runs that completed before the feature was added are also supported."
              params={[
                { name: "id", in: "path", type: "string", required: true, description: "Run id" },
              ]}
              responses={[
                { code: 200, description: "Ordered event sequence", schema:
`[
  {
    "id":         1,
    "runId":      "my-repo-1715000000-ab12cd34",
    "seq":        0,
    "ts":         1715000001,
    "eventType":  "tool_use",      // "thinking" | "tool_use" | "tool_result" | "result"
    "toolName":   "Bash",          // null for thinking / result events
    "toolUseId":  "toolu_01abc",   // pairs tool_use with its tool_result
    "inputJson":  "{\"command\":\"pnpm test\"}",
    "outputJson": null,            // populated on tool_result and result events
    "thinking":   null,            // populated on thinking events
    "isError":    false
  },
  {
    "id":         2,
    "runId":      "my-repo-1715000000-ab12cd34",
    "seq":        1,
    "ts":         1715000005,
    "eventType":  "tool_result",
    "toolName":   null,
    "toolUseId":  "toolu_01abc",
    "inputJson":  null,
    "outputJson": "[{\"type\":\"text\",\"text\":\"PASS  src/foo.test.ts\\n\"}]",
    "thinking":   null,
    "isError":    false
  },
  {
    "id":         3,
    "seq":        2,
    "eventType":  "result",
    "outputJson": "{\"type\":\"result\",\"subtype\":\"success\",\"result\":\"Done.\",\"total_cost_usd\":0.0234}",
    "isError":    false
  }
]` },
              ]}
            />

            <Route
              method="GET" path="/graph/issues/{n}/why"
              summary="Dispatch decision audit log for an issue"
              description="Returns every dispatch evaluation that has been recorded for this issue number. Each entry corresponds to one workflow engine poll tick that evaluated the issue. Answers the question 'why was this issue dispatched (or not dispatched) at each point in time?' — essential for debugging workflow rule mismatches."
              params={[
                { name: "n",    in: "path",  type: "integer", required: true,  description: "GitHub issue number", example: "42" },
                { name: "repo", in: "query", type: "string",  required: true,  description: "Full repo name (owner/repo)", example: "org/my-repo" },
              ]}
              responses={[
                { code: 200, description: "Dispatch entries ordered by matchedAt DESC", schema:
`[
  {
    "id":             1,
    "issueNumber":    42,
    "repoFullName":   "org/my-repo",
    "matchedAt":      1715000000,        // unix seconds
    "ruleIndex":      6,                 // 0-based index of the matched dispatch rule; null if no rule matched
    "directiveType":  "spawn_fresh",     // "spawn_fresh" | "no_action" | "wait" | "human_review"
    "directiveJson":  "{\"directive\":\"spawn_fresh\",\"role\":\"worker\",\"mode\":null,\"reason\":\"\"}",
    "runId":          "my-repo-1715000000-ab12cd34"  // null if directive did not spawn
  },
  {
    "id":             2,
    "issueNumber":    42,
    "matchedAt":      1714990000,
    "ruleIndex":      9,
    "directiveType":  "no_action",
    "directiveJson":  "{\"directive\":\"no_action\",\"reason\":\"awaiting human merge\"}",
    "runId":          null
  }
]` },
              ]}
            />

            <Route
              method="GET" path="/graph/issues/{n}/trace"
              summary="Full causal trace: issue → dispatches → runs → call chain"
              description="The deepest observability endpoint. Returns the complete causal chain from a GitHub issue through every dispatch decision, through each spawned run, down to the individual tool calls and structured output of each agent. Used by external monitoring tools and the arbiter agent (which reads prior agent output via this endpoint)."
              params={[
                { name: "n",    in: "path",  type: "integer", required: true,  description: "GitHub issue number", example: "42" },
                { name: "repo", in: "query", type: "string",  required: true,  description: "Full repo name (owner/repo)", example: "org/my-repo" },
              ]}
              responses={[
                { code: 200, description: "IssueTrace — full causal chain", schema:
`{
  "issueNumber":  42,
  "repoFullName": "org/my-repo",
  "dispatches": [
    {
      "dispatch":  { /* DispatchEntry — see /graph/issues/{n}/why */ },
      "run":       { /* RunInfo — null if directive did not spawn */ },
      "decisions": [ /* RunEvent[] — full call chain, see /graph/runs/{id}/decisions */ ]
    }
  ]
}` },
              ]}
            />

            <Route
              method="GET" path="/graph/blocking"
              summary="Issues blocked by unresolved dep markers"
              description="Returns all issues that have <!-- deps: #N #M --> markers in their body where at least one referenced issue is still open. The workflow engine's unblock_pass uses this data. Useful for visualising the WorkPackage dependency graph."
              responses={[
                { code: 200, description: "All blocking relationships", schema:
`[
  {
    "issueId":  "org/my-repo#7",   // "{repoFullName}#{issueNumber}"
    "title":    "Implement feature B",
    "blockedBy": [
      {
        "issueId": "org/my-repo#5",
        "title":   "Scaffold project"
      }
    ]
  }
]` },
              ]}
            />
          </Section>
        </div>
      </div>
    </div>
  );
}
