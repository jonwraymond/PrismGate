import BackendCard from "./BackendCard";
import type { BackendDetail } from "../types";

interface BackendGridProps {
  backends: BackendDetail[];
}

export default function BackendGrid({ backends }: BackendGridProps) {
  if (backends.length === 0) return null;

  const sorted = [...backends].sort((a, b) => {
    if (a.state === "Unhealthy" && b.state !== "Unhealthy") return -1;
    if (b.state === "Unhealthy" && a.state !== "Unhealthy") return 1;
    return b.calls - a.calls;
  });

  return (
    <div>
      <h2 className="text-xs font-semibold uppercase tracking-wider text-text-muted mb-3 px-1">
        Backends ({backends.length})
      </h2>
      <div className="grid grid-cols-1 xl:grid-cols-2 gap-3">
        {sorted.map((b) => (
          <BackendCard key={b.name} backend={b} />
        ))}
      </div>
    </div>
  );
}
