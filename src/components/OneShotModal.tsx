import { useState } from "react";

export interface RunArgs {
  repoId: number;
  repoName: string;
  cwd: string;
  prompt: string;
  model?: string;
  outputFormat?: "text" | "json" | "stream-json";
  permissionMode?:
    | "default"
    | "acceptEdits"
    | "plan"
    | "bypassPermissions"
    | "dontAsk"
    | "auto";
  skipPermissions: boolean;
  effort?: "low" | "medium" | "high" | "xhigh" | "max";
  verbose: boolean;
  includePartialMessages: boolean;
  systemPrompt?: string;
  appendSystemPrompt?: string;
  addDir: string[];
  allowedTools: string[];
  disallowedTools: string[];
  tools?: string;
  agent?: string;
  maxBudgetUsd?: number;
  mcpConfig: string[];
  strictMcpConfig: boolean;
  resume?: string;
  continueLast: boolean;
  forkSession: boolean;
  name?: string;
  extraArgs: string[];
}

interface Props {
  repoId: number;
  repoName: string;
  cwd: string;
  onSubmit: (args: RunArgs) => Promise<void>;
  onClose: () => void;
}

function splitLines(s: string): string[] {
  return s
    .split(/\r?\n/)
    .map((x) => x.trim())
    .filter(Boolean);
}

function splitArgs(s: string): string[] {
  return s
    .split(/\s+/)
    .map((x) => x.trim())
    .filter(Boolean);
}

export default function OneShotModal({
  repoId,
  repoName,
  cwd,
  onSubmit,
  onClose,
}: Props) {
  const [prompt, setPrompt] = useState("");
  const [model, setModel] = useState("");
  const [outputFormat, setOutputFormat] = useState<
    "stream-json" | "json" | "text"
  >("stream-json");
  const [skipPermissions, setSkipPermissions] = useState(false);
  const [permissionMode, setPermissionMode] = useState("");
  const [effort, setEffort] = useState("");
  const [verbose, setVerbose] = useState(true);
  const [includePartial, setIncludePartial] = useState(false);
  const [appendSystem, setAppendSystem] = useState("");
  const [systemPrompt, setSystemPrompt] = useState("");
  const [addDir, setAddDir] = useState("");
  const [allowedTools, setAllowedTools] = useState("");
  const [disallowedTools, setDisallowedTools] = useState("");
  const [maxBudget, setMaxBudget] = useState("");
  const [agent, setAgent] = useState("");
  const [mcpConfig, setMcpConfig] = useState("");
  const [strictMcp, setStrictMcp] = useState(false);
  const [resume, setResume] = useState("");
  const [continueLast, setContinueLast] = useState(false);
  const [forkSession, setForkSession] = useState(false);
  const [name, setName] = useState("");
  const [extraArgs, setExtraArgs] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function blank<T extends string | undefined>(s: string): T {
    return (s.trim() === "" ? undefined : s.trim()) as T;
  }

  async function handleSubmit() {
    if (!prompt.trim() && !continueLast && !resume.trim()) {
      setError("Prompt 不能空白（除非選 Continue 或 Resume）。");
      return;
    }
    setSubmitting(true);
    setError(null);
    const args: RunArgs = {
      repoId,
      repoName,
      cwd,
      prompt: prompt.trim(),
      model: blank(model),
      outputFormat,
      permissionMode: blank<RunArgs["permissionMode"]>(permissionMode),
      skipPermissions,
      effort: blank<RunArgs["effort"]>(effort),
      verbose,
      includePartialMessages: includePartial,
      systemPrompt: blank(systemPrompt),
      appendSystemPrompt: blank(appendSystem),
      addDir: splitLines(addDir),
      allowedTools: splitArgs(allowedTools),
      disallowedTools: splitArgs(disallowedTools),
      tools: undefined,
      agent: blank(agent),
      maxBudgetUsd: maxBudget.trim() ? Number(maxBudget) : undefined,
      mcpConfig: splitLines(mcpConfig),
      strictMcpConfig: strictMcp,
      resume: blank(resume),
      continueLast,
      forkSession,
      name: blank(name),
      extraArgs: splitArgs(extraArgs),
    };
    try {
      await onSubmit(args);
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setSubmitting(false);
    }
  }

  return (
    <div
      className="oneshot-modal-backdrop"
      role="presentation"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        className="oneshot-modal"
        role="dialog"
        aria-modal="true"
        aria-label="New one-shot run"
      >
        <header className="oneshot-modal-header">
          <h3>New one-shot run</h3>
          <button
            type="button"
            aria-label="Close"
            onClick={onClose}
            className="terminal-tile-close"
          >
            ×
          </button>
        </header>
        <div className="oneshot-modal-body">
          {error && <div className="oneshot-modal-error">{error}</div>}

          <label>
            Prompt
            <textarea
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              rows={5}
              placeholder="Describe what you want claude -p to do…"
              autoFocus
            />
          </label>

          <label>
            Model
            <input
              type="text"
              placeholder="(default — leave blank to inherit)"
              value={model}
              onChange={(e) => setModel(e.target.value)}
            />
          </label>

          <label>
            Output format
            <select
              value={outputFormat}
              onChange={(e) =>
                setOutputFormat(e.target.value as typeof outputFormat)
              }
            >
              <option value="stream-json">stream-json (default)</option>
              <option value="json">json (single result)</option>
              <option value="text">text</option>
            </select>
          </label>

          <label className="oneshot-modal-checkbox">
            <input
              type="checkbox"
              checked={skipPermissions}
              onChange={(e) => setSkipPermissions(e.target.checked)}
            />
            --dangerously-skip-permissions
          </label>

          <details className="oneshot-modal-section">
            <summary>進階：權限 / 工具 / 預算</summary>
            <div className="oneshot-modal-section-body">
              <label>
                Permission mode
                <select
                  value={permissionMode}
                  onChange={(e) => setPermissionMode(e.target.value)}
                >
                  <option value="">(inherit)</option>
                  <option value="default">default</option>
                  <option value="acceptEdits">acceptEdits</option>
                  <option value="plan">plan</option>
                  <option value="bypassPermissions">bypassPermissions</option>
                  <option value="dontAsk">dontAsk</option>
                  <option value="auto">auto</option>
                </select>
              </label>
              <label>
                Effort
                <select value={effort} onChange={(e) => setEffort(e.target.value)}>
                  <option value="">(inherit)</option>
                  <option value="low">low</option>
                  <option value="medium">medium</option>
                  <option value="high">high</option>
                  <option value="xhigh">xhigh</option>
                  <option value="max">max</option>
                </select>
              </label>
              <label>
                Allowed tools (whitespace-separated, e.g. "Bash Edit Read")
                <input
                  type="text"
                  value={allowedTools}
                  onChange={(e) => setAllowedTools(e.target.value)}
                />
              </label>
              <label>
                Disallowed tools
                <input
                  type="text"
                  value={disallowedTools}
                  onChange={(e) => setDisallowedTools(e.target.value)}
                />
              </label>
              <label>
                Max budget (USD)
                <input
                  type="number"
                  step="0.01"
                  min="0"
                  value={maxBudget}
                  onChange={(e) => setMaxBudget(e.target.value)}
                />
              </label>
              <label className="oneshot-modal-checkbox">
                <input
                  type="checkbox"
                  checked={verbose}
                  onChange={(e) => setVerbose(e.target.checked)}
                />
                --verbose
              </label>
              <label className="oneshot-modal-checkbox">
                <input
                  type="checkbox"
                  checked={includePartial}
                  onChange={(e) => setIncludePartial(e.target.checked)}
                  disabled={outputFormat !== "stream-json"}
                />
                --include-partial-messages（需 stream-json）
              </label>
            </div>
          </details>

          <details className="oneshot-modal-section">
            <summary>進階：System prompt / Add dir</summary>
            <div className="oneshot-modal-section-body">
              <label>
                System prompt
                <textarea
                  value={systemPrompt}
                  onChange={(e) => setSystemPrompt(e.target.value)}
                  rows={3}
                />
              </label>
              <label>
                Append system prompt
                <textarea
                  value={appendSystem}
                  onChange={(e) => setAppendSystem(e.target.value)}
                  rows={3}
                />
              </label>
              <label>
                Add dirs (one per line)
                <textarea
                  value={addDir}
                  onChange={(e) => setAddDir(e.target.value)}
                  rows={2}
                />
              </label>
            </div>
          </details>

          <details className="oneshot-modal-section">
            <summary>進階：Session / MCP / 逃生口</summary>
            <div className="oneshot-modal-section-body">
              <label>
                Display name
                <input
                  type="text"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                />
              </label>
              <label>
                Agent
                <input
                  type="text"
                  value={agent}
                  onChange={(e) => setAgent(e.target.value)}
                />
              </label>
              <label className="oneshot-modal-checkbox">
                <input
                  type="checkbox"
                  checked={continueLast}
                  onChange={(e) => setContinueLast(e.target.checked)}
                />
                --continue（接最後一次同 cwd 對話）
              </label>
              <label>
                Resume session id
                <input
                  type="text"
                  value={resume}
                  onChange={(e) => setResume(e.target.value)}
                  placeholder="claude session UUID"
                />
              </label>
              <label className="oneshot-modal-checkbox">
                <input
                  type="checkbox"
                  checked={forkSession}
                  onChange={(e) => setForkSession(e.target.checked)}
                />
                --fork-session
              </label>
              <label>
                MCP config files (one per line)
                <textarea
                  value={mcpConfig}
                  onChange={(e) => setMcpConfig(e.target.value)}
                  rows={2}
                />
              </label>
              <label className="oneshot-modal-checkbox">
                <input
                  type="checkbox"
                  checked={strictMcp}
                  onChange={(e) => setStrictMcp(e.target.checked)}
                />
                --strict-mcp-config
              </label>
              <label>
                Extra args (raw flags, whitespace-separated)
                <input
                  type="text"
                  value={extraArgs}
                  onChange={(e) => setExtraArgs(e.target.value)}
                  placeholder="e.g. --debug api --betas xyz"
                />
              </label>
            </div>
          </details>
        </div>
        <footer className="oneshot-modal-footer">
          <button type="button" onClick={onClose} disabled={submitting}>
            Cancel
          </button>
          <button
            type="button"
            className="oneshot-new-btn"
            onClick={handleSubmit}
            disabled={submitting}
          >
            {submitting ? "Starting…" : "Start"}
          </button>
        </footer>
      </div>
    </div>
  );
}
