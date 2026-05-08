import "./Tabs.css";

export type TabKey = "home" | "persistent" | "one-shot" | "cron";

export interface RepoCounts {
  persistent: number;
  oneShot: number;
  cron: number;
}

interface Props {
  active: TabKey;
  counts: RepoCounts;
  disabled: boolean;
  onChange: (tab: TabKey) => void;
}

interface Spec {
  key: TabKey;
  label: string;
  count?: (c: RepoCounts) => number;
}

const TABS: Spec[] = [
  { key: "home", label: "Home" },
  { key: "persistent", label: "Persistent", count: (c) => c.persistent },
  { key: "one-shot", label: "One-Shot", count: (c) => c.oneShot },
  { key: "cron", label: "Cron", count: (c) => c.cron },
];

export default function Tabs({ active, counts, disabled, onChange }: Props) {
  return (
    <nav className="tabs" role="tablist" aria-label="Repository views">
      {TABS.map(({ key, label, count }) => {
        const isActive = key === active;
        return (
          <button
            key={key}
            type="button"
            role="tab"
            aria-selected={isActive}
            disabled={disabled}
            className={`tab ${isActive ? "is-active" : ""}`}
            onClick={() => onChange(key)}
          >
            <span className="tab-label">{label}</span>
            {count && (
              <span className="tab-count">({count(counts)})</span>
            )}
          </button>
        );
      })}
    </nav>
  );
}
