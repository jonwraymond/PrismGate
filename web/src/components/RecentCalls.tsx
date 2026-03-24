import { CheckCircle, XCircle } from "lucide-react";
import { formatMs, formatTimeAgo } from "../utils";
import type { CallEvent } from "../types";

interface RecentCallsProps {
  calls: CallEvent[];
}

export default function RecentCalls({ calls }: RecentCallsProps) {
  if (calls.length === 0) return null;

  return (
    <div className="rounded-xl border border-surface-border bg-surface-800/40 overflow-hidden">
      <div className="px-4 py-2.5 border-b border-surface-border">
        <h2 className="text-xs font-semibold uppercase tracking-wider text-text-muted">
          Recent Calls
        </h2>
      </div>
      <div className="overflow-x-auto">
        <table className="w-full text-xs">
          <thead>
            <tr className="border-b border-surface-border text-text-dim">
              <th className="text-left px-4 py-2 font-medium">Tool</th>
              <th className="text-left px-4 py-2 font-medium">Backend</th>
              <th className="text-right px-4 py-2 font-medium">Duration</th>
              <th className="text-center px-4 py-2 font-medium">Status</th>
              <th className="text-right px-4 py-2 font-medium">When</th>
            </tr>
          </thead>
          <tbody>
            {calls.map((call, i) => (
              <tr
                key={`${call.tool_name}-${call.seconds_ago}-${i}`}
                className={`border-b border-surface-border/50 transition-colors duration-300 ${
                  call.seconds_ago < 3
                    ? "bg-accent/5"
                    : "hover:bg-surface-700/20"
                }`}
              >
                <td className="px-4 py-2 font-mono font-medium text-text-primary">
                  {call.tool_name}
                </td>
                <td className="px-4 py-2 font-mono text-text-muted">
                  {call.backend_name}
                </td>
                <td className="px-4 py-2 font-mono text-right text-text-muted">
                  {formatMs(call.duration_ms)}
                </td>
                <td className="px-4 py-2 text-center">
                  {call.success ? (
                    <CheckCircle className="w-3.5 h-3.5 text-healthy inline" />
                  ) : (
                    <XCircle className="w-3.5 h-3.5 text-unhealthy inline" />
                  )}
                </td>
                <td className="px-4 py-2 font-mono text-right text-text-dim">
                  {formatTimeAgo(call.seconds_ago)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
