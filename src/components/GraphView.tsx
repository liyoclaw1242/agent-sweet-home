import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./GraphView.css";

// ── Types ─────────────────────────────────────────────────────────────────────

interface RunSummary {
  runId: string;
  repoName: string;
  status: string;
  startedAt: number;
  endedAt: number | null;
  totalCostUsd: number | null;
  eventCount: number;
  toolCallCount: number;
  agentLabel: string | null;
}

interface RunEvent {
  id: number;
  runId: string;
  seq: number;
  ts: number;
  eventType: string;
  toolName: string | null;
  toolUseId: string | null;
  inputJson: string | null;
  outputJson: string | null;
  thinking: string | null;
  isError: boolean;
}

// ── Trace building ────────────────────────────────────────────────────────────

type TraceNode =
  | { kind: "thinking"; text: string; seq: number }
  | {
      kind: "call";
      toolName: string;
      toolUseId: string;
      input: string;
      seq: number;
      duration: number | null;
      result: { output: string; isError: boolean } | null;
    }
  | { kind: "result"; text: string; cost: number | null; isError: boolean };

function prettyJson(s: string | null): string {
  if (!s) return "";
  try { return JSON.stringify(JSON.parse(s), null, 2); }
  catch { return s; }
}

function extractOutput(outputJson: string | null): string {
  if (!outputJson) return "";
  try {
    const v = JSON.parse(outputJson);
    if (typeof v === "string") return v;
    if (Array.isArray(v)) {
      return v.map((b: unknown) => {
        if (typeof b === "object" && b !== null) {
          const block = b as { type?: string; text?: string };
          if (block.type === "text") return block.text ?? "";
        }
        return JSON.stringify(b);
      }).join("\n");
    }
    return JSON.stringify(v, null, 2);
  } catch { return outputJson; }
}

function extractFinalResult(outputJson: string | null): { text: string; cost: number | null } {
  if (!outputJson) return { text: "", cost: null };
  try {
    const v = JSON.parse(outputJson) as Record<string, unknown>;
    return { text: String(v.result ?? ""), cost: (v.total_cost_usd as number) ?? null };
  } catch { return { text: outputJson, cost: null }; }
}

function buildTrace(events: RunEvent[]): TraceNode[] {
  const resultMap = new Map<string, RunEvent>();
  for (const ev of events) {
    if (ev.eventType === "tool_result" && ev.toolUseId) {
      resultMap.set(ev.toolUseId, ev);
    }
  }

  const seen = new Set<string>();
  const nodes: TraceNode[] = [];

  for (const ev of events) {
    if (ev.eventType === "tool_result") continue;

    if (ev.eventType === "thinking") {
      nodes.push({ kind: "thinking", text: ev.thinking ?? "", seq: ev.seq });
    } else if (ev.eventType === "tool_use" && ev.toolName && ev.toolUseId) {
      if (seen.has(ev.toolUseId)) continue;
      seen.add(ev.toolUseId);
      const res = resultMap.get(ev.toolUseId) ?? null;
      nodes.push({
        kind: "call",
        toolName: ev.toolName,
        toolUseId: ev.toolUseId,
        input: prettyJson(ev.inputJson),
        seq: ev.seq,
        duration: res ? res.ts - ev.ts : null,
        result: res ? { output: extractOutput(res.outputJson), isError: res.isError } : null,
      });
    } else if (ev.eventType === "result") {
      const { text, cost } = extractFinalResult(ev.outputJson);
      nodes.push({ kind: "result", text, cost, isError: ev.isError });
    }
  }

  return nodes;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function statusIcon(s: string) {
  if (s === "running") return "●";
  if (s === "completed") return "✓";
  if (s === "failed") return "✗";
  return "⊘";
}

function statusCls(s: string) {
  if (s === "running")   return "gs-running";
  if (s === "completed") return "gs-ok";
  if (s === "failed")    return "gs-fail";
  return "gs-killed";
}

// Map workflow role slug → short display label
const ROLE_LABELS: Record<string, string> = {
  worker:               "worker",
  implementer:          "worker",
  "whitebox-validator": "whitebox",
  whitebox_validator:   "whitebox",
  "blackbox-validator": "blackbox",
  blackbox_validator:   "blackbox",
  validator:            "validator",
  arbiter:              "arbiter",
  dispatcher:           "dispatch",
};

function agentBadge(label: string | null): { text: string; cls: string } | null {
  if (!label) return null;
  const key = label.toLowerCase();
  const text = ROLE_LABELS[key] ?? label.split(/[-_]/)[0];
  if (key.includes("whitebox"))  return { text, cls: "agent-whitebox" };
  if (key.includes("blackbox"))  return { text, cls: "agent-blackbox" };
  if (key.includes("validator")) return { text, cls: "agent-validator" };
  if (key.includes("arbiter"))   return { text, cls: "agent-arbiter" };
  return { text, cls: "agent-worker" };
}

function fmtCost(v: number | null) {
  return v == null ? "—" : `$${v.toFixed(4)}`;
}

function timeAgo(ts: number) {
  const sec = Math.floor(Date.now() / 1000) - ts;
  if (sec < 60) return `${sec}s ago`;
  if (sec < 3600) return `${Math.floor(sec / 60)}m ago`;
  return `${Math.floor(sec / 3600)}h ago`;
}

function fmtDur(sec: number | null) {
  if (sec === null || sec < 0) return "";
  if (sec < 1)  return "<1s";
  if (sec < 60) return `${sec}s`;
  return `${Math.floor(sec / 60)}m${sec % 60}s`;
}

// ── Trace renderers ───────────────────────────────────────────────────────────

function ThinkingNode({ text }: { text: string }) {
  return (
    <div className="trace-node trace-thinking-node">
      <div className="trace-dot trace-dot-think" />
      <details className="trace-think-block">
        <summary className="trace-think-sum">
          <span className="trace-think-label">thinking</span>
          <span className="trace-think-len">{text.length.toLocaleString()} chars</span>
        </summary>
        <pre className="trace-io-body">{text}</pre>
      </details>
    </div>
  );
}

function CallNode({ node }: { node: Extract<TraceNode, { kind: "call" }> }) {
  const err = node.result?.isError ?? false;
  const shortOutput = node.result?.output?.slice(0, 80).replace(/\n/g, " ") ?? "";
  const autoOpen = (node.result?.output?.length ?? 0) < 300;

  return (
    <div className={`trace-node trace-call-node${err ? " trace-err" : ""}`}>
      <div className={`trace-dot${err ? " trace-dot-err" : ""}`} />
      <div className="trace-call-body">
        <div className="trace-call-head">
          <span className="trace-tool-name">{node.toolName}</span>
          {node.duration !== null && (
            <span className="trace-dur">{fmtDur(node.duration)}</span>
          )}
        </div>

        {node.input.trim() && (
          <details className="trace-io-block">
            <summary className="trace-io-sum">
              <span className="trace-io-tag">input</span>
            </summary>
            <pre className="trace-io-body">{node.input}</pre>
          </details>
        )}

        {node.result && (
          <details className="trace-io-block trace-output-block" open={autoOpen}>
            <summary className="trace-io-sum">
              <span className={`trace-io-tag${err ? " trace-io-err" : " trace-io-out"}`}>
                {err ? "error" : "output"}
              </span>
              {!err && shortOutput && (
                <span className="trace-io-preview">
                  {shortOutput}{node.result.output.length > 80 ? "…" : ""}
                </span>
              )}
            </summary>
            <pre className="trace-io-body">{node.result.output}</pre>
          </details>
        )}
      </div>
    </div>
  );
}

function ResultNode({ node }: { node: Extract<TraceNode, { kind: "result" }> }) {
  return (
    <div className={`trace-result-bar${node.isError ? " trace-result-err" : ""}`}>
      <span className="trace-result-status">{node.isError ? "failed" : "completed"}</span>
      {node.cost !== null && <span className="trace-result-cost">{fmtCost(node.cost)}</span>}
      {node.text.trim() && (
        <span className="trace-result-text">
          {node.text.length > 240 ? `${node.text.slice(0, 240)}…` : node.text}
        </span>
      )}
    </div>
  );
}

// ── Main ──────────────────────────────────────────────────────────────────────

export default function GraphView() {
  const [runs, setRuns]               = useState<RunSummary[]>([]);
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const [events, setEvents]           = useState<RunEvent[]>([]);
  const [loading, setLoading]         = useState(true);
  const [evLoading, setEvLoading]     = useState(false);
  const [error, setError]             = useState<string | null>(null);
  const [lastRefresh, setLastRefresh] = useState<Date | null>(null);

  const loadRuns = useCallback(async () => {
    setError(null);
    try {
      const gs = await invoke<{ runs: RunSummary[] }>("graph_state_cmd");
      setRuns(gs.runs);
      setLastRefresh(new Date());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadRuns();
    const id = setInterval(() => void loadRuns(), 15_000);
    return () => clearInterval(id);
  }, [loadRuns]);

  useEffect(() => {
    if (!selectedRunId) { setEvents([]); return; }
    setEvLoading(true);
    invoke<RunEvent[]>("graph_run_events_cmd", { runId: selectedRunId })
      .then((evs) => setEvents(evs ?? []))
      .catch((e) => setError(e instanceof Error ? e.message : String(e)))
      .finally(() => setEvLoading(false));
  }, [selectedRunId]);

  const trace = useMemo(() => buildTrace(events), [events]);
  const selectedRun = runs.find((r) => r.runId === selectedRunId) ?? null;
  const liveCount   = runs.filter((r) => r.status === "running").length;

  return (
    <div className="graph-view">
      <header className="graph-header">
        <h2>Call Chain</h2>
        <div className="graph-header-right">
          {lastRefresh && (
            <span className="graph-last-refresh">updated {lastRefresh.toLocaleTimeString()}</span>
          )}
          <button type="button" className="graph-reload-btn" onClick={() => void loadRuns()}>
            ↻ Refresh
          </button>
        </div>
      </header>

      {error && <p className="graph-error">{error}</p>}

      <div className="graph-body">
        {/* ── Run list ── */}
        <aside className="run-list-panel">
          <div className="panel-hdr">
            <span className="panel-title">Runs</span>
            {liveCount > 0 && (
              <span className="graph-live-badge">{liveCount} live</span>
            )}
          </div>

          {loading && runs.length === 0 ? (
            <p className="graph-empty">Loading…</p>
          ) : runs.length === 0 ? (
            <p className="graph-empty">No runs yet.</p>
          ) : (
            <ul className="run-list">
              {runs.slice(0, 50).map((r) => (
                <li key={r.runId}>
                  <button
                    type="button"
                    className={`run-item${selectedRunId === r.runId ? " is-selected" : ""}`}
                    onClick={() => setSelectedRunId(r.runId)}
                  >
                    <span className={`run-icon ${statusCls(r.status)}`}>{statusIcon(r.status)}</span>
                    <div className="run-item-body">
                      <div className="run-item-top">
                        <span className="run-repo">{r.repoName}</span>
                        <span className={`run-status ${statusCls(r.status)}`}>{r.status}</span>
                      </div>
                      <div className="run-item-meta">
                        {(() => { const b = agentBadge(r.agentLabel); return b ? <span className={`agent-badge ${b.cls}`}>{b.text}</span> : null; })()}
                        <span>{r.toolCallCount} calls</span>
                        <span className="run-sep">·</span>
                        <span>{fmtCost(r.totalCostUsd)}</span>
                        <span className="run-sep">·</span>
                        <span>{timeAgo(r.startedAt)}</span>
                      </div>
                      <div className="run-item-id">{r.runId}</div>
                    </div>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </aside>

        {/* ── Trace panel ── */}
        <section className="trace-panel">
          {!selectedRunId ? (
            <div className="trace-empty">Select a run to view its call chain.</div>
          ) : evLoading ? (
            <div className="trace-empty">Loading trace…</div>
          ) : (
            <>
              {selectedRun && (
                <div className="trace-hdr">
                  <div className="trace-hdr-left">
                    <span className="trace-hdr-id">{selectedRun.runId}</span>
                    <span className="trace-hdr-repo">{selectedRun.repoName}</span>
                  </div>
                  <div className="trace-hdr-right">
                    {(() => { const b = agentBadge(selectedRun.agentLabel); return b ? <span className={`agent-badge ${b.cls}`}>{b.text}</span> : null; })()}
                    <span className={`trace-hdr-status ${statusCls(selectedRun.status)}`}>
                      {selectedRun.status}
                    </span>
                    <span className="trace-hdr-cost">{fmtCost(selectedRun.totalCostUsd)}</span>
                    <span className="trace-hdr-calls">{selectedRun.toolCallCount} calls</span>
                  </div>
                </div>
              )}

              <div className="trace-scroll">
                {trace.length === 0 ? (
                  <div className="trace-empty">
                    No structured events.{" "}
                    {selectedRun?.status === "running"
                      ? "Events are parsed after the run completes."
                      : "Run may have used plain-text output format."}
                  </div>
                ) : (
                  <div className="trace-chain">
                    {trace.map((node, i) => {
                      const key = `${node.kind}-${i}`;
                      if (node.kind === "thinking") return <ThinkingNode key={key} text={node.text} />;
                      if (node.kind === "call")     return <CallNode     key={key} node={node} />;
                      if (node.kind === "result")   return <ResultNode   key={key} node={node} />;
                      return null;
                    })}
                  </div>
                )}
              </div>
            </>
          )}
        </section>
      </div>
    </div>
  );
}
