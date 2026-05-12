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

interface SavedSettings {
  githubUsername: string;
  githubToken: string;
  localBasePath: string;
  workflowPath: string;
}

export default function WorkflowView() {
  const [status, setStatus] = useState<WorkflowStatus | null>(null);
  const [fetchError, setFetchError] = useState<string | null>(null);
  const [savedPath, setSavedPath] = useState<string>("");
  const [pathDraft, setPathDraft] = useState<string>("");
  const [saveState, setSaveState] = useState<
    | { kind: "idle" }
    | { kind: "saving" }
    | { kind: "saved" }
    | { kind: "error"; message: string }
  >({ kind: "idle" });

  const load = useCallback(async () => {
    setFetchError(null);
    try {
      const [s, settings] = await Promise.all([
        invoke<WorkflowStatus>("workflow_status"),
        invoke<SavedSettings>("get_settings"),
      ]);
      setStatus(s);
      setSavedPath(settings.workflowPath ?? "");
      setPathDraft(settings.workflowPath ?? "");
    } catch (e) {
      setFetchError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const onSavePath = useCallback(async () => {
    setSaveState({ kind: "saving" });
    try {
      await invoke("save_workflow_path", { path: pathDraft.trim() });
      setSavedPath(pathDraft.trim());
      setSaveState({ kind: "saved" });
    } catch (e) {
      setSaveState({
        kind: "error",
        message: e instanceof Error ? e.message : String(e),
      });
    }
  }, [pathDraft]);

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

  const dirty = pathDraft.trim() !== savedPath;

  return (
    <div className="workflow-view">
      <header className="workflow-header">
        <h2>Workflow</h2>
        <button type="button" onClick={load} className="workflow-reload">
          Reload
        </button>
      </header>

      {fetchError && <p className="workflow-error">Failed: {fetchError}</p>}

      <section className="workflow-path-form">
        <label htmlFor="workflow-path-input" className="workflow-path-label">
          YAML path
        </label>
        <div className="workflow-path-row">
          <input
            id="workflow-path-input"
            type="text"
            value={pathDraft}
            placeholder="/abs/path/to/workflow.yaml (leave empty to use app_data_dir/workflow.yaml)"
            onChange={(e) => {
              setPathDraft(e.target.value);
              if (saveState.kind !== "idle") setSaveState({ kind: "idle" });
            }}
            className="workflow-path-input"
            spellCheck={false}
            autoCorrect="off"
            autoCapitalize="off"
          />
          <button
            type="button"
            onClick={onSavePath}
            disabled={!dirty || saveState.kind === "saving"}
            className="workflow-path-save"
          >
            {saveState.kind === "saving" ? "Saving…" : "Save"}
          </button>
        </div>
        <p className="workflow-path-hint">
          Priority: <code>WORKFLOW_FILE</code> env &gt; saved path &gt;{" "}
          <code>app_data_dir/workflow.yaml</code>. Restart the App for the
          change to take effect.
        </p>
        {saveState.kind === "saved" && (
          <p className="workflow-path-saved">
            Saved. Restart the App to load this YAML.
          </p>
        )}
        {saveState.kind === "error" && (
          <p className="workflow-error">Save failed: {saveState.message}</p>
        )}
      </section>

      {status && (
        <>
          <dl className="workflow-meta">
            <dt>Status</dt>
            <dd>
              <span className={`workflow-badge ${stateClass}`}>{stateLabel}</span>
            </dd>
            <dt>Applied path</dt>
            <dd>
              <span
                className={`wf-dot ${
                  status.loaded
                    ? "wf-dot--ok"
                    : status.exists
                    ? "wf-dot--error"
                    : "wf-dot--missing"
                }`}
                aria-hidden="true"
              />
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
              No YAML at this path. Save a path above (and restart), set the{" "}
              <code>WORKFLOW_FILE</code> env var, or drop a{" "}
              <code>workflow.yaml</code> at <code>app_data_dir</code>.
            </p>
          )}
        </>
      )}
    </div>
  );
}
