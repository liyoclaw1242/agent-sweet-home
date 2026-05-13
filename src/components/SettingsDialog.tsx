import { useEffect, useState, type FormEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./SettingsDialog.css";

interface Props {
  open: boolean;
  onClose: () => void;
  onSaved: () => void;
}

interface Settings {
  githubUsername: string;
  githubToken: string;
  localBasePath: string;
  workflowPath: string;
}

interface WorkflowStatus {
  loaded: boolean;
  exists: boolean;
  error: string | null;
}

export default function SettingsDialog({ open, onClose, onSaved }: Props) {
  const [username, setUsername] = useState("");
  const [token, setToken] = useState("");
  const [localBasePath, setLocalBasePath] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Workflow path — separate save (requires restart)
  const [wfPath, setWfPath] = useState("");
  const [savedWfPath, setSavedWfPath] = useState("");
  const [wfStatus, setWfStatus] = useState<WorkflowStatus | null>(null);
  const [wfSaving, setWfSaving] = useState(false);
  const [wfSaveMsg, setWfSaveMsg] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    (async () => {
      try {
        const [s, ws] = await Promise.all([
          invoke<Settings>("get_settings"),
          invoke<WorkflowStatus>("workflow_status"),
        ]);
        if (cancelled) return;
        setUsername(s.githubUsername ?? "");
        setToken(s.githubToken ?? "");
        setLocalBasePath(s.localBasePath ?? "");
        setWfPath(s.workflowPath ?? "");
        setSavedWfPath(s.workflowPath ?? "");
        setWfStatus(ws);
        setError(null);
      } catch (e) {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => { cancelled = true; };
  }, [open]);

  if (!open) return null;

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    setSaving(true);
    setError(null);
    try {
      await invoke("save_settings", {
        githubUsername: username,
        githubToken: token,
        localBasePath: localBasePath,
      });
      onSaved();
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  }

  async function handleSaveWfPath() {
    setWfSaving(true);
    setWfSaveMsg(null);
    try {
      await invoke("save_workflow_path", { path: wfPath.trim() });
      setSavedWfPath(wfPath.trim());
      setWfSaveMsg("Saved — restart the app to apply.");
    } catch (err) {
      setWfSaveMsg(`Error: ${err instanceof Error ? err.message : String(err)}`);
    } finally {
      setWfSaving(false);
    }
  }

  return (
    <div
      className="dialog-backdrop"
      onClick={onClose}
      role="presentation"
    >
      <div
        className="dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-title"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="dialog-header">
          <h2 id="settings-title">Settings</h2>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close settings"
            className="dialog-close"
          >
            ×
          </button>
        </header>
        <form onSubmit={handleSubmit} className="dialog-form">
          <label className="dialog-field">
            <span>GitHub Username</span>
            <input
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              autoComplete="off"
              spellCheck={false}
              placeholder="octocat"
            />
          </label>
          <label className="dialog-field">
            <span>Auth Token</span>
            <input
              type="password"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              autoComplete="off"
              spellCheck={false}
              placeholder="ghp_…"
            />
            <small>
              Personal access token with <code>repo</code> scope. Stored locally
              in SQLite under your app data directory.
            </small>
          </label>
          <label className="dialog-field">
            <span>Default Local Path</span>
            <input
              value={localBasePath}
              onChange={(e) => setLocalBasePath(e.target.value)}
              autoComplete="off"
              spellCheck={false}
              placeholder="~/Projects"
            />
            <small>
              Each repo is expected at <code>{`{this path}/{repoName}`}</code>.
              The Home tab uses it to resolve the local path and run{" "}
              <code>git status</code>.
            </small>
          </label>
          <div className="dialog-section-divider" />
          <div className="dialog-section-title">Workflow</div>
          <label className="dialog-field">
            <span>YAML path</span>
            <input
              value={wfPath}
              onChange={(e) => {
                setWfPath(e.target.value);
                setWfSaveMsg(null);
              }}
              autoComplete="off"
              spellCheck={false}
              placeholder="/abs/path/to/workflow.yaml"
            />
            <small>
              Leave empty to use <code>app_data_dir/workflow.yaml</code>.
              Override with <code>WORKFLOW_FILE</code> env var.
            </small>
          </label>
          <div className="dialog-wf-row">
            {wfStatus && (
              <span className={`dialog-wf-dot ${wfStatus.loaded ? "dialog-wf-ok" : wfStatus.exists ? "dialog-wf-error" : "dialog-wf-missing"}`} />
            )}
            {wfStatus && (
              <span className="dialog-wf-status">
                {wfStatus.loaded ? "loaded" : wfStatus.exists ? "parse error" : "not found"}
              </span>
            )}
            <button
              type="button"
              className="dialog-wf-save"
              onClick={() => void handleSaveWfPath()}
              disabled={wfSaving || wfPath.trim() === savedWfPath}
            >
              {wfSaving ? "Saving…" : "Save path"}
            </button>
          </div>
          {wfSaveMsg && (
            <p className={`dialog-wf-msg ${wfSaveMsg.startsWith("Error") ? "dialog-error" : "dialog-wf-ok-msg"}`}>
              {wfSaveMsg}
            </p>
          )}

          {error && <p className="dialog-error">{error}</p>}
          <div className="dialog-actions">
            <button type="button" onClick={onClose} disabled={saving}>
              Cancel
            </button>
            <button type="submit" disabled={saving} className="dialog-primary">
              {saving ? "Saving…" : "Save"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
