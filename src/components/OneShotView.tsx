import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type React from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Repo, SidebarRun } from "./Sidebar";
import OneShotModal, { type RunArgs } from "./OneShotModal";
import "./OneShotView.css";

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

// ── stream-json parsing ───────────────────────────────────────────────────────

type ContentBlock =
  | { type: "text"; text: string }
  | { type: "thinking"; thinking: string }
  | { type: "tool_use"; id: string; name: string; input: unknown }
  | { type: "tool_result"; tool_use_id: string; content: unknown; is_error?: boolean };

type ClaudeEvent = {
  type: string;
  message?: { role: string; content: ContentBlock[] };
  result?: string;
  total_cost_usd?: number;
  is_error?: boolean;
};

type ChatMsg =
  | { kind: "user-prompt";  text: string;   ts: number; seq: number }
  | { kind: "thinking";     text: string;   ts: number; seq: number }
  | { kind: "assistant";    text: string;   ts: number; seq: number }
  | { kind: "tool-call";   id: string; name: string; input: unknown; ts: number; seq: number }
  | { kind: "tool-result"; toolId: string; isError: boolean; content: string; ts: number; seq: number }
  | { kind: "result";      text: string; cost: number | null; isError: boolean; rawJson: string; ts: number; seq: number }
  | { kind: "stderr";      text: string;   ts: number; seq: number }
  | { kind: "raw";         text: string;   ts: number; seq: number };

function extractResultContent(content: unknown): string {
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {
    return content
      .map((b) => {
        if (typeof b === "object" && b !== null && "type" in b) {
          const block = b as { type: string; text?: string };
          return block.type === "text" ? (block.text ?? "") : JSON.stringify(b, null, 2);
        }
        return JSON.stringify(b);
      })
      .join("\n");
  }
  return JSON.stringify(content, null, 2);
}

function parseLines(lines: LogLine[]): ChatMsg[] {
  const msgs: ChatMsg[] = [];

  for (const line of lines) {
    if (!line.text.trim()) continue;

    if (line.stream === "stderr") {
      msgs.push({ kind: "stderr", text: line.text, ts: line.ts, seq: line.seq });
      continue;
    }

    try {
      const ev = JSON.parse(line.text) as ClaudeEvent;

      if (ev.type === "user" && ev.message) {
        for (const block of ev.message.content) {
          if (block.type === "text") {
            msgs.push({ kind: "user-prompt", text: block.text, ts: line.ts, seq: line.seq });
          } else if (block.type === "tool_result") {
            msgs.push({
              kind: "tool-result",
              toolId: block.tool_use_id,
              isError: block.is_error ?? false,
              content: extractResultContent(block.content),
              ts: line.ts,
              seq: line.seq,
            });
          }
        }
      } else if (ev.type === "assistant" && ev.message) {
        for (const block of ev.message.content) {
          if (block.type === "thinking") {
            msgs.push({ kind: "thinking", text: block.thinking, ts: line.ts, seq: line.seq });
          } else if (block.type === "text") {
            msgs.push({ kind: "assistant", text: block.text, ts: line.ts, seq: line.seq });
          } else if (block.type === "tool_use") {
            msgs.push({ kind: "tool-call", id: block.id, name: block.name, input: block.input, ts: line.ts, seq: line.seq });
          }
        }
      } else if (ev.type === "result") {
        msgs.push({
          kind: "result",
          text: ev.result ?? "",
          cost: ev.total_cost_usd ?? null,
          isError: ev.is_error ?? false,
          rawJson: line.text,
          ts: line.ts,
          seq: line.seq,
        });
      }
      // type === "system" (init) — skip
    } catch {
      msgs.push({ kind: "raw", text: line.text, ts: line.ts, seq: line.seq });
    }
  }

  return msgs;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function roleFromArgv(argv: string[]): string | null {
  const idx = argv.indexOf("--name");
  if (idx === -1) return null;
  const name = argv[idx + 1];
  if (!name) return null;
  // Format: {role}-{mode}-issue{N}
  const withoutIssue = name.replace(/-issue\d+$/, "");
  const role = withoutIssue.replace(/-[^-]+$/, "");
  return role || null;
}

function statusClass(s: RunInfo["status"]) {
  return `status-badge status-${s}`;
}

function fmtTs(ts: number) {
  if (!ts) return "";
  return new Date(ts * 1000).toLocaleTimeString([], {
    hour: "2-digit", minute: "2-digit", second: "2-digit",
  });
}

// ── Chat renderers ────────────────────────────────────────────────────────────

function ThinkingBubble({ text, ts }: { text: string; ts: number }) {
  return (
    <div className="chat-row thinking-row">
      <details className="thinking-block">
        <summary className="thinking-summary">
          <span className="thinking-label">thinking</span>
          <span className="thinking-len">{text.length.toLocaleString()} chars</span>
          <span className="bubble-ts">{fmtTs(ts)}</span>
        </summary>
        <pre className="thinking-body">{text}</pre>
      </details>
    </div>
  );
}

function UserBubble({ text, ts }: { text: string; ts: number }) {
  return (
    <div className="chat-row user-row">
      <div className="chat-bubble user-bubble">
        <pre className="bubble-text">{text}</pre>
        <span className="bubble-ts">{fmtTs(ts)}</span>
      </div>
    </div>
  );
}

function AssistantBubble({ text, ts }: { text: string; ts: number }) {
  return (
    <div className="chat-row assistant-row">
      <div className="chat-sender-label">Claude</div>
      <div className="chat-bubble assistant-bubble">
        <pre className="bubble-text">{text}</pre>
        <span className="bubble-ts">{fmtTs(ts)}</span>
      </div>
    </div>
  );
}

function ToolCallBlock({ name, input, ts }: { name: string; input: unknown; ts: number }) {
  return (
    <div className="chat-row tool-row">
      <details className="tool-block">
        <summary className="tool-head">
          <span className="tool-chevron" aria-hidden>▶</span>
          <span className="tool-name">{name}</span>
          <span className="tool-ts">{fmtTs(ts)}</span>
        </summary>
        <pre className="tool-body">{JSON.stringify(input, null, 2)}</pre>
      </details>
    </div>
  );
}

function ToolResultBlock({ toolId, content, isError, ts }: { toolId: string; content: string; isError: boolean; ts: number }) {
  return (
    <div className="chat-row tool-row tool-result-row">
      <details className="tool-block tool-result-block">
        <summary className="tool-head">
          <span className="tool-chevron" aria-hidden>▶</span>
          <span className={`tool-result-label${isError ? " is-error" : ""}`}>
            {isError ? "error" : "result"}
          </span>
          <span className="tool-id">…{toolId.slice(-8)}</span>
          <span className="tool-ts">{fmtTs(ts)}</span>
        </summary>
        <pre className="tool-body">{content}</pre>
      </details>
    </div>
  );
}

function ResultBar({ text, cost, isError, rawJson }: { text: string; cost: number | null; isError: boolean; rawJson: string }) {
  const pretty = (() => { try { return JSON.stringify(JSON.parse(rawJson), null, 2); } catch { return rawJson; } })();
  return (
    <div className={`result-bar${isError ? " is-error" : ""}`}>
      <div className="result-bar-head">
        <span className="result-status">{isError ? "failed" : "completed"}</span>
        {cost !== null && <span className="result-cost">${cost.toFixed(4)}</span>}
        {text.trim() && (
          <span className="result-text">{text.length > 200 ? `${text.slice(0, 200)}…` : text}</span>
        )}
      </div>
      <details className="result-raw-block">
        <summary className="result-raw-summary">structured output</summary>
        <pre className="result-raw-body">{pretty}</pre>
      </details>
    </div>
  );
}

// ── Props ─────────────────────────────────────────────────────────────────────

interface Props {
  repo: Repo;
  selectedRunId: string | null;
  onCountChange?: (count: number) => void;
  onRunsChange?: (runs: SidebarRun[]) => void;
  onRunCreated?: (id: string) => void;
  newOneShotRef?: React.MutableRefObject<(() => void) | null>;
}

// ── Component ─────────────────────────────────────────────────────────────────

export default function OneShotView({
  repo,
  selectedRunId,
  onCountChange,
  onRunsChange,
  onRunCreated,
  newOneShotRef,
}: Props) {
  const [runs, setRuns] = useState<RunInfo[]>([]);
  const [lines, setLines] = useState<LogLine[]>([]);
  const [showModal, setShowModal] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const feedRef = useRef<HTMLDivElement>(null);
  const atBottomRef = useRef(true);

  const onCountRef = useRef(onCountChange);
  onCountRef.current = onCountChange;
  const onRunsChangeRef = useRef(onRunsChange);
  onRunsChangeRef.current = onRunsChange;

  const loadRuns = useCallback(async () => {
    try {
      const list = await invoke<RunInfo[]>("one_shot_list", { args: { repoId: repo.id } });
      setRuns(list ?? []);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [repo.id]);

  useEffect(() => { void loadRuns(); }, [loadRuns]);

  // Reload when the workflow engine creates a new run for this repo.
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    listen<string>(`oneshot:created:${repo.id}`, () => { void loadRuns(); })
      .then((fn) => { unlisten = fn; })
      .catch(() => {});
    return () => { unlisten?.(); };
  }, [repo.id, loadRuns]);

  const runningCount = useMemo(() => runs.filter((r) => r.status === "running").length, [runs]);

  useEffect(() => {
    onCountRef.current?.(runningCount);
    onRunsChangeRef.current?.(
      runs.map((r) => ({
        id: r.id,
        status: r.status,
        role: roleFromArgv(r.argv),
      })),
    );
  }, [runs, runningCount]);

  // Stream log for selected run
  useEffect(() => {
    if (!selectedRunId) { setLines([]); return; }
    let unlistenLine: UnlistenFn | null = null;
    let unlistenExit: UnlistenFn | null = null;
    let cancelled = false;

    void (async () => {
      try {
        const initial = await invoke<LogLine[]>("one_shot_log", {
          args: { id: selectedRunId, sinceSeq: -1, limit: 5000 },
        });
        if (cancelled) return;
        setLines(initial ?? []);
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      }

      const ul = await listen<LogLine>(`oneshot:line:${selectedRunId}`, (ev) => {
        const ll = ev.payload;
        setLines((prev) => {
          if (prev.length > 0 && prev[prev.length - 1].seq >= ll.seq) return prev;
          return [...prev, ll];
        });
      });
      const ux = await listen<unknown>(`oneshot:exit:${selectedRunId}`, () => { void loadRuns(); });
      if (cancelled) { ul(); ux(); } else { unlistenLine = ul; unlistenExit = ux; }
    })();

    return () => { cancelled = true; unlistenLine?.(); unlistenExit?.(); };
  }, [selectedRunId, loadRuns]);

  // Auto-scroll to bottom while at bottom
  useEffect(() => {
    const el = feedRef.current;
    if (!el || !atBottomRef.current) return;
    el.scrollTop = el.scrollHeight;
  }, [lines]);

  useEffect(() => {
    if (newOneShotRef) {
      newOneShotRef.current = () => setShowModal(true);
      return () => { newOneShotRef.current = null; };
    }
  }, [newOneShotRef]);

  const handleStart = useCallback(async (raw: RunArgs) => {
    const inspection = await invoke<{ exists: boolean; repoPath: string }>(
      "inspect_local_repo", { repoName: repo.name },
    );
    if (!inspection.exists) throw new Error(`Local path not found: ${inspection.repoPath}`);
    const created = await invoke<RunInfo>("one_shot_start", { args: { ...raw, cwd: inspection.repoPath } });
    setRuns((prev) => [created, ...prev]);
    onRunCreated?.(created.id);
  }, [repo.name, onRunCreated]);

  const handleKill = useCallback(async (id: string) => {
    try { await invoke("one_shot_kill", { id }); } catch { /* already gone */ }
    await loadRuns();
    if (selectedRunId === id) {
      try {
        const refreshed = await invoke<LogLine[]>("one_shot_log", {
          args: { id, sinceSeq: -1, limit: 5000 },
        });
        setLines(refreshed ?? []);
      } catch { /* ignore */ }
    }
  }, [selectedRunId, loadRuns]);

  const activeRun = useMemo(
    () => (selectedRunId ? (runs.find((r) => r.id === selectedRunId) ?? null) : null),
    [runs, selectedRunId],
  );

  const chatMsgs = useMemo(() => parseLines(lines), [lines]);

  function handleScroll() {
    const el = feedRef.current;
    if (!el) return;
    atBottomRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
  }

  return (
    <section className="oneshot-chat">
      <header className="oneshot-chat-header">
        {activeRun ? (
          <>
            <div className="chat-hdr-meta">
              <span className="chat-run-id">{activeRun.id}</span>
              <span className="chat-run-cmd" title={activeRun.argv.join(" ")}>
                {activeRun.argv.join(" ")}
              </span>
              {activeRun.prompt.trim() && (
                <details className="chat-prompt-block">
                  <summary className="chat-prompt-summary">prompt</summary>
                  <pre className="chat-prompt-body">{activeRun.prompt}</pre>
                </details>
              )}
            </div>
            <span className={statusClass(activeRun.status)}>{activeRun.status}</span>
            {activeRun.totalCostUsd != null && (
              <span className="chat-cost">${activeRun.totalCostUsd.toFixed(4)}</span>
            )}
            <button
              type="button"
              className="chat-action-btn"
              onClick={() => handleKill(activeRun.id)}
            >
              {activeRun.status === "running" ? "Kill" : "Delete"}
            </button>
          </>
        ) : (
          <span className="chat-hdr-empty">Select a run from the sidebar</span>
        )}
      </header>

      <div className="oneshot-feed" ref={feedRef} onScroll={handleScroll}>
        {!selectedRunId && (
          <div className="chat-empty">No run selected.</div>
        )}
        {selectedRunId && chatMsgs.length === 0 && (
          <div className="chat-empty">Waiting for output…</div>
        )}

        {chatMsgs.map((msg) => {
          const key = `${msg.kind}-${msg.seq}`;
          switch (msg.kind) {
            case "thinking":
              return <ThinkingBubble key={key} text={msg.text} ts={msg.ts} />;
            case "user-prompt":
              return <UserBubble   key={key} text={msg.text} ts={msg.ts} />;
            case "assistant":
              return <AssistantBubble key={key} text={msg.text} ts={msg.ts} />;
            case "tool-call":
              return <ToolCallBlock key={key} name={msg.name} input={msg.input} ts={msg.ts} />;
            case "tool-result":
              return <ToolResultBlock key={key} toolId={msg.toolId} content={msg.content} isError={msg.isError} ts={msg.ts} />;
            case "result":
              return <ResultBar key={key} text={msg.text} cost={msg.cost} isError={msg.isError} rawJson={msg.rawJson} />;
            case "stderr":
              return <div key={key} className="chat-stderr">{msg.text}</div>;
            case "raw":
              return <div key={key} className="chat-raw">{msg.text}</div>;
          }
        })}
      </div>

      {error && <div className="oneshot-error-bar">{error}</div>}

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
