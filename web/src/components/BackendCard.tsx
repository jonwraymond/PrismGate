import { useState } from "react";
import {
  ChevronDown,
  ChevronUp,
  MemoryStick,
  Terminal,
} from "lucide-react";
import { formatMs, stateColor } from "../utils";
import LatencySparkline from "./LatencySparkline";
import type { BackendDetail } from "../types";

interface BackendCardProps {
  backend: BackendDetail;
}

export default function BackendCard({ backend }: BackendCardProps) {
  const [expanded, setExpanded] = useState(false);
  const b = backend;

  const memPct =
    b.rss_mb !== null && b.peak_rss_mb !== null && b.peak_rss_mb > 0
      ? (b.rss_mb / b.peak_rss_mb) * 100
      : 0;

  const memColor =
    memPct > 80
      ? "var(--color-unhealthy)"
      : memPct > 60
        ? "var(--color-degraded)"
        : "var(--color-healthy)";

  return (
    <div
      id={`backend-${b.name}`}
      className="rounded-lg border border-surface-border bg-surface-800/50 overflow-hidden transition-all duration-200"
    >
      {/* Card header */}
      <div
        className="flex items-center justify-between px-4 py-3 cursor-pointer hover:bg-surface-700/30 transition-colors"
        onClick={() => setExpanded(!expanded)}
      >
        <div className="flex items-center gap-3">
          <span
            className="w-2.5 h-2.5 rounded-full shrink-0"
            style={{ backgroundColor: stateColor(b.state) }}
          />
          <span className="font-mono font-semibold text-sm">{b.name}</span>
          <span
            className="text-[10px] uppercase tracking-wider font-medium px-1.5 py-0.5 rounded"
            style={{
              color: stateColor(b.state),
              backgroundColor: `color-mix(in srgb, ${stateColor(b.state)} 15%, transparent)`,
            }}
          >
            {b.state}
          </span>
        </div>
        <div className="flex items-center gap-4 text-xs text-text-muted">
          <span className="font-mono">{b.tool_count} tools</span>
          <span className="font-mono">{b.calls} calls</span>
          <span className="font-mono">{formatMs(b.p50_ms)} p50</span>
          {expanded ? (
            <ChevronUp className="w-4 h-4" />
          ) : (
            <ChevronDown className="w-4 h-4" />
          )}
        </div>
      </div>

      {/* Expanded details */}
      {expanded && (
        <div className="px-4 pb-4 space-y-3 border-t border-surface-border pt-3">
          <div className="grid grid-cols-4 gap-3 text-xs">
            <div>
              <span className="text-text-dim block">PID</span>
              <span className="font-mono font-medium">{b.pid ?? "—"}</span>
            </div>
            <div>
              <span className="text-text-dim block">p50</span>
              <span className="font-mono font-medium">{formatMs(b.p50_ms)}</span>
            </div>
            <div>
              <span className="text-text-dim block">p95</span>
              <span className="font-mono font-medium">{formatMs(b.p95_ms)}</span>
            </div>
            <div>
              <span className="text-text-dim block">Calls</span>
              <span className="font-mono font-medium">{b.calls}</span>
            </div>
          </div>

          {b.rss_mb !== null && (
            <div className="space-y-1">
              <div className="flex items-center justify-between text-xs">
                <span className="text-text-dim flex items-center gap-1">
                  <MemoryStick className="w-3 h-3" /> RSS
                </span>
                <span className="font-mono text-text-muted">
                  {b.rss_mb}MB / {b.peak_rss_mb ?? "?"}MB peak
                </span>
              </div>
              <div className="h-1.5 bg-surface-600 rounded-full overflow-hidden">
                <div
                  className="h-full rounded-full transition-all duration-500"
                  style={{
                    width: `${Math.min(memPct, 100)}%`,
                    backgroundColor: memColor,
                  }}
                />
              </div>
            </div>
          )}

          <div>
            <span className="text-xs text-text-dim block mb-1">Latency trend</span>
            <LatencySparkline p50={b.p50_ms} p95={b.p95_ms} calls={b.calls} />
          </div>

          {b.recent_stderr.length > 0 && (
            <div>
              <span className="text-xs text-text-dim flex items-center gap-1 mb-1">
                <Terminal className="w-3 h-3" /> stderr
              </span>
              <pre className="text-[11px] font-mono leading-relaxed text-text-muted bg-surface-900 rounded-md p-2 max-h-32 overflow-y-auto">
                {b.recent_stderr.join("\n")}
              </pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
