import { useEffect, useRef, useState } from "react";
import type { Repo } from "./Sidebar";
import DocsModal from "./DocsModal";
import "./Header.css";

interface Props {
  selectedRepo: Repo | null;
  repos: Repo[];
  repoLoading: boolean;
  sessionCost?: number;
  sessionBudget?: number;
  onSelectRepo: (repo: Repo) => void;
  onOpenSettings: () => void;
}

export default function Header({
  selectedRepo,
  repos,
  repoLoading,
  sessionCost = 0,
  sessionBudget = 2,
  onSelectRepo,
  onOpenSettings,
}: Props) {
  const [dropdownOpen, setDropdownOpen] = useState(false);
  const [docsOpen, setDocsOpen] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!dropdownOpen) return;
    function handleClick(e: MouseEvent) {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setDropdownOpen(false);
      }
    }
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [dropdownOpen]);

  const pct = sessionBudget > 0 ? sessionCost / sessionBudget : 0;
  const costColor =
    pct > 0.8
      ? "var(--status-burn)"
      : pct > 0.5
        ? "var(--status-running)"
        : "var(--accent)";

  return (
    <header className="app-header">
      {/* Repo switcher */}
      <div className="topbar-repo-wrap" ref={dropdownRef}>
        <button
          type="button"
          className="topbar-repo"
          onClick={() => setDropdownOpen((o) => !o)}
          aria-expanded={dropdownOpen}
          aria-haspopup="listbox"
        >
          <span className="topbar-repo-name">
            {repoLoading
              ? "Loading…"
              : (selectedRepo?.fullName ?? "Select repo")}
          </span>
          <span className="topbar-chev">▾</span>
        </button>

        {dropdownOpen && repos.length > 0 && (
          <div className="topbar-repo-menu" role="listbox">
            {repos.map((r) => (
              <button
                key={r.id}
                type="button"
                role="option"
                aria-selected={selectedRepo?.id === r.id}
                className={`topbar-repo-item ${selectedRepo?.id === r.id ? "is-active" : ""}`}
                onClick={() => { onSelectRepo(r); setDropdownOpen(false); }}
              >
                <span className="topbar-repo-item-name">{r.fullName}</span>
                {r.language && (
                  <span className="topbar-repo-item-lang">{r.language}</span>
                )}
              </button>
            ))}
          </div>
        )}
        {dropdownOpen && repos.length === 0 && !repoLoading && (
          <div className="topbar-repo-menu">
            <p className="topbar-repo-empty">No repositories configured.</p>
          </div>
        )}
      </div>

      {/* Cmd+K */}
      <div className="topbar-cmdk">
        <span className="topbar-cmdk-icon">⌕</span>
        <span className="topbar-cmdk-placeholder">
          Search runs, sessions, issues…
        </span>
        <kbd className="topbar-cmdk-kbd">⌘K</kbd>
      </div>

      {/* Cost meter */}
      <div className="topbar-cost">
        <span className="topbar-cost-label">Session</span>
        <span className="topbar-cost-value" style={{ color: costColor }}>
          ${sessionCost.toFixed(4)}
        </span>
        <span className="topbar-cost-sep">/</span>
        <span className="topbar-cost-budget">${sessionBudget.toFixed(2)}</span>
        <div className="topbar-cost-bar">
          <div
            className="topbar-cost-fill"
            style={{ width: `${Math.min(pct * 100, 100)}%`, background: costColor }}
          />
        </div>
      </div>

      {/* Docs */}
      <button
        type="button"
        className="topbar-icon-btn"
        onClick={() => setDocsOpen(true)}
        aria-label="API reference"
        title="API reference"
      >
        <svg width="15" height="15" viewBox="0 0 15 15" fill="none" aria-hidden>
          <rect x="2.5" y="1.5" width="9" height="12" rx="0" stroke="currentColor" strokeWidth="1.2"/>
          <line x1="5" y1="5.5" x2="10" y2="5.5" stroke="currentColor" strokeWidth="1"/>
          <line x1="5" y1="7.8" x2="10" y2="7.8" stroke="currentColor" strokeWidth="1"/>
          <line x1="5" y1="10.1" x2="8"  y2="10.1" stroke="currentColor" strokeWidth="1"/>
        </svg>
      </button>

      {/* Settings */}
      <button
        type="button"
        className="topbar-icon-btn"
        onClick={onOpenSettings}
        aria-label="Open settings"
        title="Settings"
      >
        ⚙
      </button>

      {docsOpen && <DocsModal onClose={() => setDocsOpen(false)} />}
    </header>
  );
}
