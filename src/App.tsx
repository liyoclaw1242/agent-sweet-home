import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import Header from "./components/Header";
import Sidebar, {
  type Repo,
  type SidebarSession,
  type SidebarRun,
} from "./components/Sidebar";
import { type RepoCounts, type TabKey } from "./components/Tabs";
import ManifestStrip from "./components/ManifestStrip";
import SettingsDialog from "./components/SettingsDialog";
import HomeView from "./components/HomeView";
import PersistentView from "./components/PersistentView";
import OneShotView from "./components/OneShotView";
import WorkflowView from "./components/WorkflowView";
import "./App.css";

const ZERO_COUNTS: RepoCounts = { persistent: 0, oneShot: 0 };

function App() {
  // ── Repo state (lifted from Sidebar) ──────────────────────────────────
  const [repos, setRepos] = useState<Repo[]>([]);
  const [repoLoading, setRepoLoading] = useState(false);
  const [selectedRepo, setSelectedRepo] = useState<Repo | null>(null);

  const loadRepos = useCallback(async () => {
    setRepoLoading(true);
    try {
      const data = await invoke<Repo[]>("fetch_repos");
      setRepos(data);
    } catch {
      setRepos([]);
    } finally {
      setRepoLoading(false);
    }
  }, []);

  useEffect(() => { void loadRepos(); }, [loadRepos]);

  // ── Settings ───────────────────────────────────────────────────────────
  const [settingsOpen, setSettingsOpen] = useState(false);

  // ── Tab / section navigation ───────────────────────────────────────────
  const [activeTabByRepo, setActiveTabByRepo] = useState<Record<number, TabKey>>({});
  const [countsByRepo, setCountsByRepo] = useState<Record<number, RepoCounts>>({});

  const setCount = useCallback(
    (repoId: number, key: keyof RepoCounts, value: number) => {
      setCountsByRepo((prev) => {
        const current = prev[repoId] ?? ZERO_COUNTS;
        if (current[key] === value) return prev;
        return { ...prev, [repoId]: { ...current, [key]: value } };
      });
    },
    [],
  );

  const activeTab: TabKey = selectedRepo
    ? activeTabByRepo[selectedRepo.id] ?? "info"
    : "info";
  const counts = selectedRepo
    ? countsByRepo[selectedRepo.id] ?? ZERO_COUNTS
    : ZERO_COUNTS;

  function changeTab(tab: TabKey) {
    if (!selectedRepo) return;
    setActiveTabByRepo((prev) => ({ ...prev, [selectedRepo.id]: tab }));
  }

  // ── Sidebar item summaries ─────────────────────────────────────────────
  const [sessionsByRepo, setSessionsByRepo] = useState<Record<number, SidebarSession[]>>({});
  const [runsByRepo, setRunsByRepo] = useState<Record<number, SidebarRun[]>>({});

  // Pre-load sidebar lists whenever the selected repo changes, so the
  // sidebar is populated even before the child view mounts.
  useEffect(() => {
    if (!selectedRepo) return;
    const id = selectedRepo.id;

    void invoke<Array<{ id: string; exitCode: number | null; frozen: boolean; uptimeSecs: number }>>(
      "pty_list", { args: { repoId: id } },
    ).then((list) => {
      setSessionsByRepo((prev) => ({
        ...prev,
        [id]: (list ?? []).map((s) => {
          if (s.exitCode !== null) return { id: s.id, status: "exited" as const,  meta: `exit ${s.exitCode}` };
          if (s.frozen)            return { id: s.id, status: "frozen" as const,  meta: "idle" };
          const m = Math.floor(s.uptimeSecs / 60), sec = s.uptimeSecs % 60;
          return { id: s.id, status: "running" as const, meta: m > 0 ? `${m}m${sec}s` : `${sec}s` };
        }),
      }));
    }).catch(() => {});

    void invoke<Array<{ id: string; status: SidebarRun["status"]; argv: string[] }>>(
      "one_shot_list", { args: { repoId: id } },
    ).then((list) => {
      setRunsByRepo((prev) => ({
        ...prev,
        [id]: (list ?? []).map((r) => {
          const nameIdx = (r.argv ?? []).indexOf("--name");
          const name = nameIdx !== -1 ? r.argv[nameIdx + 1] ?? "" : "";
          const role = name
            ? (name.replace(/-issue\d+$/, "").replace(/-[^-]+$/, "") || null)
            : null;
          return { id: r.id, status: r.status, role };
        }),
      }));
    }).catch(() => {});
  }, [selectedRepo]);

  const sessions = selectedRepo ? sessionsByRepo[selectedRepo.id] ?? [] : [];
  const runs     = selectedRepo ? runsByRepo[selectedRepo.id] ?? []     : [];

  // ── Selected run (driven by sidebar click) ────────────────────────────
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);

  useEffect(() => { setSelectedRunId(null); }, [selectedRepo?.id]);

  // ── New-item triggers from sidebar ────────────────────────────────────
  const newTerminalRef = useRef<(() => void) | null>(null);
  const newOneShotRef  = useRef<(() => void) | null>(null);

  return (
    <div className="layout">
      <Header
        selectedRepo={selectedRepo}
        repos={repos}
        repoLoading={repoLoading}
        onSelectRepo={setSelectedRepo}
        onOpenSettings={() => setSettingsOpen(true)}
      />
      <ManifestStrip items={[]} />

      <div className="app-body">
        <Sidebar
          activeTab={activeTab}
          counts={counts}
          disabled={!selectedRepo}
          sessions={sessions}
          runs={runs}
          selectedRunId={selectedRunId}
          onChangeTab={changeTab}
          onSelectRun={(id) => { changeTab("one-shot"); setSelectedRunId(id); }}
          onNewTerminal={() => {
            changeTab("persistent");
            setTimeout(() => newTerminalRef.current?.(), 50);
          }}
          onNewOneShot={() => {
            changeTab("one-shot");
            setTimeout(() => newOneShotRef.current?.(), 50);
          }}
        />

        <main className="main">
          <div className="main-content">
            {selectedRepo ? (
              <RepoView
                repo={selectedRepo}
                tab={activeTab}
                selectedRunId={selectedRunId}
                onPersistentCount={(n) => setCount(selectedRepo.id, "persistent", n)}
                onOneShotCount={(n) => setCount(selectedRepo.id, "oneShot", n)}
                onSessionsChange={(s) =>
                  setSessionsByRepo((prev) => ({ ...prev, [selectedRepo.id]: s }))
                }
                onRunsChange={(r) =>
                  setRunsByRepo((prev) => ({ ...prev, [selectedRepo.id]: r }))
                }
                onRunCreated={setSelectedRunId}
                newTerminalRef={newTerminalRef}
                newOneShotRef={newOneShotRef}
              />
            ) : (
              <p className="placeholder">
                Select a repository from the top bar to begin.
              </p>
            )}
          </div>
        </main>
      </div>

      <SettingsDialog
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
        onSaved={() => { void loadRepos(); }}
      />
    </div>
  );
}

function RepoView({
  repo,
  tab,
  selectedRunId,
  onPersistentCount,
  onOneShotCount,
  onSessionsChange,
  onRunsChange,
  onRunCreated,
  newTerminalRef,
  newOneShotRef,
}: {
  repo: Repo;
  tab: TabKey;
  selectedRunId: string | null;
  onPersistentCount: (n: number) => void;
  onOneShotCount: (n: number) => void;
  onSessionsChange: (s: SidebarSession[]) => void;
  onRunsChange: (r: SidebarRun[]) => void;
  onRunCreated: (id: string) => void;
  newTerminalRef: React.MutableRefObject<(() => void) | null>;
  newOneShotRef: React.MutableRefObject<(() => void) | null>;
}) {
  return (
    <>
      {tab === "info" && <HomeView repo={repo} />}
      {tab === "flow" && <WorkflowView repo={repo} />}
      {tab === "one-shot" && (
        <OneShotView
          repo={repo}
          selectedRunId={selectedRunId}
          onCountChange={onOneShotCount}
          onRunsChange={onRunsChange}
          onRunCreated={onRunCreated}
          newOneShotRef={newOneShotRef}
        />
      )}
      {/* Always mounted — hiding with CSS preserves xterm instances and scrollback */}
      <div style={tab === "persistent" ? { display: "contents" } : { display: "none" }}>
        <PersistentView
          repo={repo}
          onCountChange={onPersistentCount}
          onSessionsChange={onSessionsChange}
          newTerminalRef={newTerminalRef}
        />
      </div>
    </>
  );
}

export default App;
