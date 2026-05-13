import "./Sidebar.css";
import type { TabKey, RepoCounts } from "./Tabs";

export interface Repo {
  id: number;
  name: string;
  fullName: string;
  description: string | null;
  htmlUrl: string;
  private: boolean;
  defaultBranch: string;
  stargazersCount: number;
  language: string | null;
  updatedAt: string;
}

export interface SidebarSession {
  id: string;
  status: "running" | "frozen" | "exited";
  meta: string;
}

export interface SidebarRun {
  id: string;
  status: "running" | "completed" | "failed" | "killed";
  role: string | null;
}

interface Props {
  activeTab: TabKey;
  counts: RepoCounts;
  disabled: boolean;
  sessions: SidebarSession[];
  runs: SidebarRun[];
  selectedRunId?: string | null;
  onChangeTab: (tab: TabKey) => void;
  onSelectRun?: (id: string) => void;
  onNewTerminal?: () => void;
  onNewOneShot?: () => void;
}

type DotStatus = "run" | "ok" | "bad" | "zzz";

function sessionDot(s: SidebarSession["status"]): DotStatus {
  if (s === "running") return "run";
  if (s === "frozen")  return "zzz";
  return "ok";
}

function runDot(s: SidebarRun["status"]): DotStatus {
  if (s === "running")   return "run";
  if (s === "completed") return "ok";
  if (s === "failed")    return "bad";
  return "zzz";
}

function StatusDot({ kind }: { kind: DotStatus }) {
  return <span className={`dot dot-${kind}`} />;
}

function roleCls(role: string): string {
  const r = role.toLowerCase();
  if (r.includes("whitebox"))  return "role-whitebox";
  if (r.includes("blackbox"))  return "role-blackbox";
  if (r.includes("validator")) return "role-validator";
  if (r.includes("arbiter"))   return "role-arbiter";
  return "role-worker";
}

function roleShort(role: string): string {
  const map: Record<string, string> = {
    worker: "worker", implementer: "worker",
    "whitebox-validator": "whitebox", whitebox_validator: "whitebox",
    "blackbox-validator": "blackbox", blackbox_validator: "blackbox",
    validator: "valid", arbiter: "arbiter", dispatcher: "disp",
  };
  return map[role.toLowerCase()] ?? role.split(/[-_]/)[0];
}

export default function Sidebar({
  activeTab,
  counts,
  disabled,
  sessions,
  runs,
  selectedRunId,
  onChangeTab,
  onSelectRun,
  onNewTerminal,
  onNewOneShot,
}: Props) {
  function channel(tab: TabKey, label: string, dot?: DotStatus, meta?: string) {
    const isActive = activeTab === tab;
    return (
      <button
        key={`${tab}-${label}`}
        type="button"
        disabled={disabled}
        className={`channel ${isActive ? "is-selected" : ""}`}
        onClick={() => onChangeTab(tab)}
      >
        <div className="rail" />
        <div className="dot-wrap">{dot && <StatusDot kind={dot} />}</div>
        <span className="label">{label}</span>
        {meta && <span className="meta">{meta}</span>}
      </button>
    );
  }

  function runChannel(r: SidebarRun) {
    const isActive = activeTab === "one-shot" && r.id === selectedRunId;
    return (
      <button
        key={r.id}
        type="button"
        disabled={disabled}
        className={`channel ${isActive ? "is-selected" : ""}`}
        onClick={() => { onChangeTab("one-shot"); onSelectRun?.(r.id); }}
      >
        <div className="rail" />
        <div className="dot-wrap"><StatusDot kind={runDot(r.status)} /></div>
        <span className="label">{r.id}</span>
        {r.role && (
          <span className={`sidebar-role-tag ${roleCls(r.role)}`}>
            {roleShort(r.role)}
          </span>
        )}
      </button>
    );
  }

  return (
    <aside className="sidebar">

      {/* DETAIL */}
      <div className="sidebar-section">
        <div className="sidebar-section-header">
          <span className="title">Detail</span>
        </div>
        {channel("info", "info")}
        {channel("flow", "flow")}
      </div>

      {/* PERSISTENT */}
      <div className="sidebar-section">
        <div className="sidebar-section-header">
          <span className="title">Persistent</span>
          {counts.persistent > 0 && (
            <span className="section-count">{counts.persistent}</span>
          )}
          <button type="button" className="add" onClick={onNewTerminal} disabled={disabled} title="New terminal">+</button>
        </div>
        {sessions.length === 0
          ? channel("persistent", "no sessions")
          : sessions.map((s) => channel("persistent", s.id, sessionDot(s.status), s.meta))}
      </div>

      {/* ONE-SHOT */}
      <div className="sidebar-section">
        <div className="sidebar-section-header">
          <span className="title">One-shot</span>
          {counts.oneShot > 0 && (
            <span className="section-count">{counts.oneShot}</span>
          )}
          <button type="button" className="add" onClick={onNewOneShot} disabled={disabled} title="New one-shot run">+</button>
        </div>
        {runs.length === 0
          ? channel("one-shot", "no runs")
          : runs.map((r) => runChannel(r))}
      </div>

    </aside>
  );
}
