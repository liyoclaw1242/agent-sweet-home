import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Repo } from "./Sidebar";
import "./WorkflowView.css";

interface WorkflowStatus {
  loaded: boolean;
  exists: boolean;
  error: string | null;
}

interface Props {
  repo: Repo;
}

export default function WorkflowView({ repo }: Props) {
  const [status, setStatus] = useState<WorkflowStatus | null>(null);
  const [active, setActive] = useState(true);
  const [toggling, setToggling] = useState(false);
  const [fetchError, setFetchError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setFetchError(null);
    try {
      const [ws, isActive] = await Promise.all([
        invoke<WorkflowStatus>("workflow_status"),
        invoke<boolean>("workflow_get_repo_active", {
          repoFullName: repo.fullName,
        }),
      ]);
      setStatus(ws);
      setActive(isActive);
    } catch (e) {
      setFetchError(e instanceof Error ? e.message : String(e));
    }
  }, [repo.fullName]);

  useEffect(() => {
    void load();
  }, [load]);

  async function toggle() {
    setToggling(true);
    try {
      const next = !active;
      await invoke("workflow_set_repo_active", {
        repoFullName: repo.fullName,
        active: next,
      });
      setActive(next);
    } catch (e) {
      setFetchError(e instanceof Error ? e.message : String(e));
    } finally {
      setToggling(false);
    }
  }

  const engineLoaded = status?.loaded ?? false;

  return (
    <div className="workflow-view">
      <header className="workflow-header">
        <h2>Workflow</h2>
      </header>

      {fetchError && <p className="workflow-error">Error: {fetchError}</p>}

      {!status ? (
        <p className="workflow-empty">Loading…</p>
      ) : !engineLoaded ? (
        <div className="workflow-empty">
          <p>No workflow loaded.</p>
          <p>
            Configure the YAML path in <strong>Settings → Workflow</strong>{" "}
            and restart the app.
          </p>
          {status.error && (
            <pre className="workflow-error-block">{status.error}</pre>
          )}
        </div>
      ) : (
        <section className="wf-toggle-section">
          <div className="wf-toggle-row">
            <div className="wf-toggle-info">
              <code className="wf-repo-name">{repo.fullName}</code>
              <span
                className={`wf-toggle-state ${active ? "wf-active" : "wf-inactive"}`}
              >
                {active ? "active" : "excluded"}
              </span>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={active}
              aria-label={`Toggle workflow for ${repo.fullName}`}
              disabled={toggling}
              className={`wf-switch ${active ? "wf-switch--on" : ""}`}
              onClick={() => void toggle()}
            >
              <span className="wf-switch-thumb" />
            </button>
          </div>
          <p className="wf-toggle-hint">
            {active
              ? "Engine will scan this repo each tick if a local git clone exists."
              : "Excluded — engine skips this repo even if a local clone exists."}
          </p>
        </section>
      )}
    </div>
  );
}
