import { useState } from "react";
import Header from "./components/Header";
import Sidebar, { type Repo } from "./components/Sidebar";
import Tabs, { type RepoCounts, type TabKey } from "./components/Tabs";
import SettingsDialog from "./components/SettingsDialog";
import HomeView from "./components/HomeView";
import "./App.css";

const ZERO_COUNTS: RepoCounts = { persistent: 0, oneShot: 0, cron: 0 };

const TAB_LABELS: Record<TabKey, string> = {
  home: "Home",
  persistent: "Persistent",
  "one-shot": "One-Shot",
  cron: "Cron",
};

function App() {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);
  const [selectedRepo, setSelectedRepo] = useState<Repo | null>(null);
  const [activeTabByRepo, setActiveTabByRepo] = useState<Record<number, TabKey>>({});
  // Per-repo counts for the Persistent / One-Shot / Cron badges. Empty for now —
  // wire up real data sources here when those feature areas land.
  const [countsByRepo] = useState<Record<number, RepoCounts>>({});

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
          <RepoView repo={selectedRepo} tab={activeTab} />
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

function RepoView({ repo, tab }: { repo: Repo; tab: TabKey }) {
  if (tab === "home") {
    return <HomeView repo={repo} />;
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
