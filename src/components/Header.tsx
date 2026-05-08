import "./Header.css";

interface Props {
  onOpenSettings: () => void;
}

export default function Header({ onOpenSettings }: Props) {
  return (
    <header className="app-header">
      <h1 className="app-title">Agent Sweet Home</h1>
      <button
        type="button"
        className="settings-button"
        onClick={onOpenSettings}
        aria-label="Open settings"
        title="Settings"
      >
        ⚙
      </button>
    </header>
  );
}
