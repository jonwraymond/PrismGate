import { useState } from "react";
import { ChevronDown, ChevronUp } from "lucide-react";
import { formatBytes } from "../utils";
import type { SessionStats } from "../types";

interface StatsFooterProps {
  stats: SessionStats;
}

export default function StatsFooter({ stats }: StatsFooterProps) {
  const [showTools, setShowTools] = useState(false);

  return (
    <div className="rounded-xl border border-surface-border bg-surface-800/40 overflow-hidden">
      <div className="px-4 py-3 flex items-center justify-between flex-wrap gap-4">
        <StatChip label="Total Calls" value={stats.total_calls.toLocaleString()} />
        <StatChip label="Returned" value={formatBytes(stats.total_bytes_returned)} />
        <StatChip label="Processed" value={formatBytes(stats.total_bytes_processed)} />
        <StatChip
          label="Savings"
          value={`${stats.reduction_pct.toFixed(0)}%`}
          accent
        />
        <StatChip
          label="Tokens Saved"
          value={stats.estimated_tokens_saved.toLocaleString()}
          accent
        />

        {stats.per_tool.length > 0 && (
          <button
            onClick={() => setShowTools(!showTools)}
            className="flex items-center gap-1 text-xs text-text-muted hover:text-text-primary transition-colors"
          >
            Per-tool breakdown
            {showTools ? (
              <ChevronUp className="w-3.5 h-3.5" />
            ) : (
              <ChevronDown className="w-3.5 h-3.5" />
            )}
          </button>
        )}
      </div>

      {showTools && stats.per_tool.length > 0 && (
        <div className="border-t border-surface-border px-4 py-3">
          <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-2">
            {stats.per_tool
              .sort((a, b) => b.calls - a.calls)
              .map((t) => (
                <div
                  key={t.name}
                  className="text-xs bg-surface-700/30 rounded-md px-2.5 py-1.5 flex justify-between"
                >
                  <span className="font-mono text-text-muted truncate mr-2">
                    {t.name}
                  </span>
                  <span className="font-mono text-text-primary shrink-0">
                    {t.calls}×
                  </span>
                </div>
              ))}
          </div>
        </div>
      )}
    </div>
  );
}

function StatChip({
  label,
  value,
  accent,
}: {
  label: string;
  value: string;
  accent?: boolean;
}) {
  return (
    <div className="flex items-center gap-1.5">
      <span className="text-xs text-text-dim">{label}</span>
      <span
        className={`font-mono text-sm font-semibold ${accent ? "text-accent" : "text-text-primary"}`}
      >
        {value}
      </span>
    </div>
  );
}
