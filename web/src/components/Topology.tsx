import { useRef, useMemo } from "react";
import { stateColor } from "../utils";
import type { TopologyBackend, CallEvent } from "../types";

interface TopologyProps {
  backends: TopologyBackend[];
  recentCalls: CallEvent[];
  onSelectBackend?: (name: string) => void;
}

const DAEMON_RADIUS = 28;
const NODE_MIN_RADIUS = 14;
const NODE_MAX_RADIUS = 24;
const PADDING = 60;

export default function Topology({
  backends,
  recentCalls,
  onSelectBackend,
}: TopologyProps) {
  const svgRef = useRef<SVGSVGElement>(null);

  // Backends with recent activity (within last 5s)
  const activeBackends = useMemo(() => {
    const active = new Set<string>();
    for (const call of recentCalls) {
      if (call.seconds_ago < 5) active.add(call.backend_name);
    }
    return active;
  }, [recentCalls]);

  // Max calls for scaling node radius
  const maxCalls = useMemo(
    () => Math.max(1, ...backends.map((b) => b.calls)),
    [backends],
  );

  // Layout: daemon center, backends in ellipse around it
  const width = 800;
  const height = Math.max(300, 140 + backends.length * 8);
  const cx = width / 2;
  const cy = height / 2;

  const nodes = useMemo(() => {
    const count = backends.length;
    if (count === 0) return [];

    const rx = (width - PADDING * 2) / 2 - NODE_MAX_RADIUS;
    const ry = (height - PADDING * 2) / 2 - NODE_MAX_RADIUS;

    return backends.map((b, i) => {
      const angle = (2 * Math.PI * i) / count - Math.PI / 2;
      const nodeRadius =
        NODE_MIN_RADIUS +
        (NODE_MAX_RADIUS - NODE_MIN_RADIUS) * (b.calls / maxCalls);
      return {
        ...b,
        x: cx + rx * Math.cos(angle),
        y: cy + ry * Math.sin(angle),
        radius: nodeRadius,
        active: activeBackends.has(b.name),
      };
    });
  }, [backends, maxCalls, activeBackends, cx, cy, width, height]);

  return (
    <div className="rounded-xl border border-surface-border bg-surface-800/40 overflow-hidden">
      <div className="px-4 py-2.5 border-b border-surface-border">
        <h2 className="text-xs font-semibold uppercase tracking-wider text-text-muted">
          Topology
        </h2>
      </div>
      <svg
        ref={svgRef}
        viewBox={`0 0 ${width} ${height}`}
        className="w-full"
        style={{ maxHeight: "420px" }}
      >
        <defs>
          {/* Glow filter for active edges */}
          <filter id="glow">
            <feGaussianBlur stdDeviation="3" result="blur" />
            <feMerge>
              <feMergeNode in="blur" />
              <feMergeNode in="SourceGraphic" />
            </feMerge>
          </filter>
          {/* Particle along edge */}
          <circle id="particle" r="3" fill="var(--color-accent)" />
        </defs>

        {/* Edges: daemon → backend */}
        {nodes.map((node) => (
          <g key={`edge-${node.name}`}>
            <line
              x1={cx}
              y1={cy}
              x2={node.x}
              y2={node.y}
              stroke={
                node.active ? "var(--color-accent)" : "var(--color-surface-500)"
              }
              strokeWidth={node.active ? 2 : 1}
              opacity={node.active ? 0.8 : 0.3}
              filter={node.active ? "url(#glow)" : undefined}
            />
            {/* Animated particle for active edges */}
            {node.active && (
              <circle r="3" fill="var(--color-accent)" opacity="0.9">
                <animateMotion
                  dur="1.5s"
                  repeatCount="indefinite"
                  path={`M${cx},${cy} L${node.x},${node.y}`}
                />
              </circle>
            )}
          </g>
        ))}

        {/* Daemon center node */}
        <g>
          <circle
            cx={cx}
            cy={cy}
            r={DAEMON_RADIUS}
            fill="var(--color-surface-700)"
            stroke="var(--color-accent)"
            strokeWidth={2}
          />
          <text
            x={cx}
            y={cy}
            textAnchor="middle"
            dominantBaseline="central"
            fill="var(--color-accent)"
            fontSize="10"
            fontFamily="var(--font-mono)"
            fontWeight="600"
          >
            DAEMON
          </text>
        </g>

        {/* Backend nodes */}
        {nodes.map((node) => (
          <g
            key={node.name}
            className="cursor-pointer"
            onClick={() => onSelectBackend?.(node.name)}
          >
            <circle
              cx={node.x}
              cy={node.y}
              r={node.radius}
              fill="var(--color-surface-700)"
              stroke={stateColor(node.state)}
              strokeWidth={node.active ? 2.5 : 1.5}
              filter={node.active ? "url(#glow)" : undefined}
            />
            {/* Backend label */}
            <text
              x={node.x}
              y={node.y + node.radius + 14}
              textAnchor="middle"
              fill="var(--color-text-muted)"
              fontSize="10"
              fontFamily="var(--font-mono)"
            >
              {node.name}
            </text>
            {/* Tool count inside node */}
            <text
              x={node.x}
              y={node.y}
              textAnchor="middle"
              dominantBaseline="central"
              fill="var(--color-text-primary)"
              fontSize="10"
              fontFamily="var(--font-mono)"
              fontWeight="600"
            >
              {node.tool_count}
            </text>
          </g>
        ))}
      </svg>
    </div>
  );
}
