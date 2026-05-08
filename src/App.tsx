import { useCallback, useState } from "react";
import Header from "./components/Header";
import Sidebar, { type Repo } from "./components/Sidebar";
import Tabs, { type RepoCounts, type TabKey } from "./components/Tabs";
import SettingsDialog from "./components/SettingsDialog";
import HomeView from "./components/HomeView";
import PersistentView from "./components/PersistentView";
import OneShotView from "./components/OneShotView";
import WorkflowView from "./components/WorkflowView";
import "./App.css";

const ZERO_COUNTS: RepoCounts = { persistent: 0, oneShot: 0, cron: 0, workflow: 0 };

const TAB_LABELS: Record<TabKey, string> = {
  home: "Home",
  persistent: "Persistent",
  "one-shot": "One-Shot",
  cron: "Cron",
  workflow: "Workflow",
};

function App() {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);
  const [selectedRepo, setSelectedRepo] = useState<Repo | null>(null);
  const [activeTabByRepo, setActiveTabByRepo] = useState<Record<number, TabKey>>({});
  // Per-repo counts for the Persistent / One-Shot / Cron badges. Each feature
  // area reports back via its own onCountChange so the tab strip stays in sync
  // even when the user is on a different tab.
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
    ? activeTabByRepo[selectedRepo.id] ?? "home"
    : "home";
  const counts = selectedRepo
    ? countsByRepo[selectedRepo.id] ?? ZERO_COUNTS
    : ZERO_COUNTS;

  function changeTab(tab: TabKey) {
    if (!selectedRepo) return;
    setActiveTabByRepo((prev) => ({ ...prev, [selectedRepo.id]: tab }));
  }

  return (
    <div className="layout">
      <Header onOpenSettings={() => setSettingsOpen(true)} />
      <Sidebar
        reloadKey={reloadKey}
        selectedRepoId={selectedRepo?.id ?? null}
        onSelect={setSelectedRepo}
      />
      <Tabs
        active={activeTab}
        counts={counts}
        disabled={!selectedRepo}
        onChange={changeTab}
      />
      <main
        className="main"
        role="tabpanel"
        aria-label={`${TAB_LABELS[activeTab]} panel`}
      >
        {selectedRepo ? (
          <RepoView
            repo={selectedRepo}
            tab={activeTab}
            onPersistentCount={(n) => setCount(selectedRepo.id, "persistent", n)}
            onOneShotCount={(n) => setCount(selectedRepo.id, "oneShot", n)}
          />
        ) : (
          <p>Select a repository from the sidebar to begin.</p>
        )}
      </main>
      <SettingsDialog
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
        onSaved={() => setReloadKey((k) => k + 1)}
      />
    </div>
  );
}

function RepoView({
  repo,
  tab,
  onPersistentCount,
  onOneShotCount,
}: {
  repo: Repo;
  tab: TabKey;
  onPersistentCount: (n: number) => void;
  onOneShotCount: (n: number) => void;
}) {
  if (tab === "home") {
    return <HomeView repo={repo} />;
  }
  if (tab === "persistent") {
    return <PersistentView repo={repo} onCountChange={onPersistentCount} />;
  }
  if (tab === "one-shot") {
    return <OneShotView repo={repo} onCountChange={onOneShotCount} />;
  }
  if (tab === "workflow") {
    return <WorkflowView />;
  }
  return (
    <div>
      <h2>{repo.name}</h2>
      <p className="repo-meta">{repo.fullName}</p>
      <p className="placeholder">{TAB_LABELS[tab]} view — no items yet.</p>
    </div>
  );
}

export default App;
