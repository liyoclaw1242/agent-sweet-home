import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./WorkflowView.css";

interface WorkflowStatus {
  path: string;
  exists: boolean;
  loaded: boolean;
  error: string | null;
  content: string | null;
}

export default function WorkflowView() {
  const [status, setStatus] = useState<WorkflowStatus | null>(null);
  const [fetchError, setFetchError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setFetchError(null);
    try {
      const s = await invoke<WorkflowStatus>("workflow_status");
      setStatus(s);
    } catch (e) {
      setFetchError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const stateLabel = !status
    ? "loading…"
    : !status.exists
    ? "not found"
    : status.loaded
    ? "loaded"
    : "parse error";

  const stateClass = !status
    ? "wf-state-pending"
    : !status.exists
    ? "wf-state-missing"
    : status.loaded
    ? "wf-state-loaded"
    : "wf-state-error";

  return (
    <div className="workflow-view">
      <header className="workflow-header">
        <h2>Workflow</h2>
        <button type="button" onClick={load} className="workflow-reload">
          Reload
        </button>
      </header>

      {fetchError && <p className="workflow-error">Failed: {fetchError}</p>}

      {status && (
        <>
          <dl className="workflow-meta">
            <dt>Status</dt>
            <dd>
              <span className={`workflow-badge ${stateClass}`}>{stateLabel}</span>
            </dd>
            <dt>Path</dt>
            <dd>
              <code>{status.path}</code>
            </dd>
          </dl>

          {status.error && (
            <pre className="workflow-error-block">{status.error}</pre>
          )}

          {status.content !== null ? (
            <>
              <h3 className="workflow-section">YAML preview</h3>
              <pre className="workflow-yaml">{status.content}</pre>
            </>
          ) : (
            <p className="workflow-empty">
              No YAML at this path. Drop a <code>workflow.yaml</code> there or
              set the <code>WORKFLOW_FILE</code> env var, then restart the app.
            </p>
          )}
        </>
      )}
    </div>
  );
}
