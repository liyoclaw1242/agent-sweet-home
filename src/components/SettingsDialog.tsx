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
}

export default function SettingsDialog({ open, onClose, onSaved }: Props) {
  const [username, setUsername] = useState("");
  const [token, setToken] = useState("");
  const [localBasePath, setLocalBasePath] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    (async () => {
      try {
        const s = await invoke<Settings>("get_settings");
        if (cancelled) return;
        setUsername(s.githubUsername ?? "");
        setToken(s.githubToken ?? "");
        setLocalBasePath(s.localBasePath ?? "");
        setError(null);
      } catch (e) {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
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
