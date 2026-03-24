import { Activity, Box, Cpu, Gauge, Wifi, WifiOff } from "lucide-react";
import { formatBytes, formatUptime } from "../utils";
import type { DaemonInfo, SessionStats } from "../types";

interface HeaderProps {
  daemon: DaemonInfo | null;
  stats: SessionStats | null;
  connected: boolean;
}

export default function Header({ daemon, stats, connected }: HeaderProps) {
  const status = daemon?.status ?? "degraded";

  return (
    <header className="flex items-center justify-between px-6 py-4 border-b border-surface-border bg-surface-800/60 backdrop-blur-sm">
      {/* Left: Logo + Status */}
      <div className="flex items-center gap-4">
        <div className="flex items-center gap-2.5">
          <Cpu className="w-6 h-6 text-accent" strokeWidth={1.5} />
          <span className="text-lg font-bold tracking-tight font-sans">
            Gatemini
          </span>
        </div>

        {/* Status indicator */}
        <div className="flex items-center gap-2 px-3 py-1 rounded-full bg-surface-700/50">
          <span
            className="w-2 h-2 rounded-full animate-pulse"
            style={{
              backgroundColor: connected
                ? status === "healthy"
                  ? "var(--color-healthy)"
                  : "var(--color-degraded)"
                : "var(--color-unhealthy)",
            }}
          />
          <span className="text-xs font-medium text-text-muted uppercase tracking-wider">
            {connected ? status : "disconnected"}
          </span>
        </div>

        {connected ? (
          <Wifi className="w-3.5 h-3.5 text-healthy" />
        ) : (
          <WifiOff className="w-3.5 h-3.5 text-unhealthy" />
        )}
      </div>

      {/* Center: Metrics */}
      {daemon && (
        <div className="flex items-center gap-6">
          <Metric
            icon={<Box className="w-3.5 h-3.5" />}
            label="backends"
            value={daemon.total_backends}
          />
          <Metric
            icon={<Activity className="w-3.5 h-3.5" />}
            label="tools"
            value={daemon.total_tools}
          />
          <Metric
            icon={<Gauge className="w-3.5 h-3.5" />}
            label="uptime"
            value={formatUptime(daemon.uptime_seconds)}
          />
        </div>
      )}

      {/* Right: Savings chip */}
      {stats && stats.reduction_pct > 0 && (
        <div className="flex items-center gap-2 px-3 py-1.5 rounded-lg bg-accent/10 border border-accent/20">
          <span className="text-xs text-text-muted">context saved</span>
          <span className="text-sm font-mono font-semibold text-accent">
            {stats.reduction_pct.toFixed(0)}%
          </span>
          <span className="text-xs text-text-dim">
            ({formatBytes(stats.total_bytes_returned)} / {formatBytes(stats.total_bytes_processed)})
          </span>
        </div>
      )}
    </header>
  );
}

function Metric({
  icon,
  label,
  value,
}: {
  icon: React.ReactNode;
  label: string;
  value: string | number;
}) {
  return (
    <div className="flex items-center gap-1.5 text-text-muted">
      {icon}
      <span className="font-mono text-sm font-semibold text-text-primary">
        {value}
      </span>
      <span className="text-xs">{label}</span>
    </div>
  );
}
