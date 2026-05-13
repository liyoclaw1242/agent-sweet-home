// Tabs.tsx is kept only for shared type exports — the tab bar UI was removed.

export type TabKey = "info" | "flow" | "persistent" | "one-shot";

export interface RepoCounts {
  persistent: number;
  oneShot: number;
}
