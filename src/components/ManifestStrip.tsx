import "./ManifestStrip.css";

export interface ManifestItem {
  id: string;
  repo: string;
  status: "running" | "healthy" | "failed" | "frozen";
  label: string;
  meta?: string;
}

interface Props {
  items: ManifestItem[];
}

const DOT_CLASS: Record<ManifestItem["status"], string> = {
  running: "mdot-run",
  healthy: "mdot-ok",
  failed:  "mdot-bad",
  frozen:  "mdot-zzz",
};

export default function ManifestStrip({ items }: Props) {
  return (
    <div className="manifest-strip">
      <span className="manifest-label">Fleet</span>
      {items.length === 0 ? (
        <span className="manifest-empty">No active processes</span>
      ) : (
        items.map((item) => (
          <span key={item.id} className="manifest-pill">
            <span className={`manifest-dot ${DOT_CLASS[item.status]}`} />
            <span className="manifest-repo">{item.repo}</span>
            <span className="manifest-sep">·</span>
            <span className="manifest-name">{item.label}</span>
            {item.meta && <span className="manifest-meta">{item.meta}</span>}
          </span>
        ))
      )}
    </div>
  );
}
