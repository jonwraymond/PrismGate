# React Dashboard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the embedded single-file HTML dashboard with a production-grade React 19 + TypeScript + Vite + Tailwind v4 application that provides real-time monitoring of the gatemini MCP gateway.

**Architecture:** The React app lives in `web/` as a standard Vite project. It builds to `web/dist/` which the Rust admin server serves as static files via `tower-http::ServeDir`. The existing 6 API endpoints remain unchanged — the React app consumes them with a 2-second polling interval. During development, Vite's dev server proxies API calls to `127.0.0.1:19999`.

**Tech Stack:** Vite 6, React 19, TypeScript 5.7, Tailwind CSS v4, Recharts 2, Lucide React, Motion (framer-motion)

**Design Direction:** Industrial mission-control aesthetic — dark command-center theme with depth via subtle gradients and glow effects. JetBrains Mono for data/metrics, Plus Jakarta Sans for headings and UI text. Teal/cyan as the primary accent for active state, with state-semantic colors (green=healthy, amber=degraded, red=unhealthy). The topology SVG is the hero element with animated particle flow.

**Spec:** `docs/superpowers/specs/2026-03-24-react-dashboard-design.md`

---

## File Structure

```
web/
├── index.html                  # Vite entry HTML
├── package.json                # Dependencies and scripts
├── tsconfig.json               # TypeScript config
├── tsconfig.app.json           # App-specific TS config
├── vite.config.ts              # Vite config with API proxy
├── src/
│   ├── main.tsx                # React entry point
│   ├── App.tsx                 # Root component, data fetching orchestrator
│   ├── index.css               # Tailwind v4 imports + custom theme
│   ├── types.ts                # API response type definitions
│   ├── api.ts                  # Fetch helpers + polling hook
│   ├── utils.ts                # Formatting helpers (uptime, bytes, time ago)
│   ├── components/
│   │   ├── Header.tsx          # Status bar with logo, metrics, savings chip
│   │   ├── Topology.tsx        # SVG node graph with animated edges
│   │   ├── BackendGrid.tsx     # Grid container for backend cards
│   │   ├── BackendCard.tsx     # Individual backend detail card
│   │   ├── LatencySparkline.tsx# Recharts sparkline for latency
│   │   ├── RecentCalls.tsx     # Auto-updating call log table
│   │   └── StatsFooter.tsx     # Session metrics and per-tool breakdown
```

**Rust modifications:**
- `src/admin.rs` — Replace `include_str!` with `tower-http::ServeDir` for `web/dist/`, fallback to `dashboard.html`
- `Cargo.toml` — Add `tower-http` dependency with `fs` feature under the `admin` feature gate

---

## Task 1: Scaffold Vite Project

**Files:**
- Create: `web/package.json`
- Create: `web/index.html`
- Create: `web/vite.config.ts`
- Create: `web/tsconfig.json`
- Create: `web/tsconfig.app.json`
- Create: `web/src/main.tsx`
- Create: `web/src/App.tsx`
- Create: `web/src/index.css`

- [ ] **Step 1: Create package.json**

```json
{
  "name": "gatemini-dashboard",
  "private": true,
  "version": "0.0.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "preview": "vite preview"
  },
  "dependencies": {
    "lucide-react": "^0.475.0",
    "motion": "^12.6.0",
    "react": "^19.0.0",
    "react-dom": "^19.0.0",
    "recharts": "^2.15.0"
  },
  "devDependencies": {
    "@tailwindcss/vite": "^4.1.0",
    "@types/react": "^19.0.0",
    "@types/react-dom": "^19.0.0",
    "@vitejs/plugin-react": "^4.3.0",
    "tailwindcss": "^4.1.0",
    "typescript": "~5.7.0",
    "vite": "^6.2.0"
  }
}
```

- [ ] **Step 2: Create vite.config.ts**

```typescript
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    port: 3000,
    proxy: {
      "/api": {
        target: "http://127.0.0.1:19999",
        changeOrigin: true,
      },
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
});
```

- [ ] **Step 3: Create index.html**

```html
<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>Gatemini Dashboard</title>
    <link rel="preconnect" href="https://fonts.googleapis.com" />
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin />
    <link href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;500;600&family=Plus+Jakarta+Sans:wght@400;500;600;700&display=swap" rel="stylesheet" />
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

- [ ] **Step 4: Create tsconfig.json and tsconfig.app.json**

`tsconfig.json`:
```json
{
  "files": [],
  "references": [{ "path": "./tsconfig.app.json" }]
}
```

`tsconfig.app.json`:
```json
{
  "compilerOptions": {
    "target": "ES2020",
    "useDefineForClassFields": true,
    "lib": ["ES2020", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowImportingTsExtensions": true,
    "isolatedModules": true,
    "moduleDetection": "force",
    "noEmit": true,
    "jsx": "react-jsx",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noFallthroughCasesInSwitch": true,
    "noUncheckedSideEffectImports": true
  },
  "include": ["src"]
}
```

- [ ] **Step 5: Create index.css with Tailwind v4**

```css
@import "tailwindcss";

@theme inline {
  --font-sans: "Plus Jakarta Sans", system-ui, sans-serif;
  --font-mono: "JetBrains Mono", ui-monospace, monospace;

  --color-surface-900: #0c1222;
  --color-surface-800: #131b2e;
  --color-surface-700: #1a2540;
  --color-surface-600: #243050;
  --color-surface-500: #334155;
  --color-surface-border: #2a3a52;

  --color-accent: #14b8a6;
  --color-accent-glow: #14b8a640;
  --color-accent-dim: #0d9488;

  --color-healthy: #22c55e;
  --color-degraded: #eab308;
  --color-unhealthy: #ef4444;
  --color-starting: #3b82f6;
  --color-stopped: #6b7280;

  --color-text-primary: #e2e8f0;
  --color-text-muted: #94a3b8;
  --color-text-dim: #64748b;
}

body {
  background: var(--color-surface-900);
  color: var(--color-text-primary);
  font-family: var(--font-sans);
  min-height: 100vh;
}

@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after {
    animation-duration: 0.01ms !important;
    transition-duration: 0.01ms !important;
  }
}
```

- [ ] **Step 6: Create main.tsx and placeholder App.tsx**

`src/main.tsx`:
```tsx
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./index.css";
import App from "./App";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
```

`src/App.tsx`:
```tsx
export default function App() {
  return (
    <div className="min-h-screen p-6">
      <h1 className="text-2xl font-bold text-accent">Gatemini Dashboard</h1>
      <p className="text-text-muted mt-2">Loading...</p>
    </div>
  );
}
```

- [ ] **Step 7: Install dependencies and verify dev server starts**

```bash
cd web && npm install
npm run dev -- --host 2>&1 | head -5
```

Expected: Vite prints `Local: http://localhost:3000/`

- [ ] **Step 8: Commit scaffold**

```bash
git add web/
git commit -m "feat(dashboard): scaffold Vite + React 19 + TypeScript + Tailwind v4 project"
```

---

## Task 2: API Types and Data Fetching

**Files:**
- Create: `web/src/types.ts`
- Create: `web/src/api.ts`
- Create: `web/src/utils.ts`

- [ ] **Step 1: Create types.ts matching API response shapes**

```typescript
// /api/topology
export interface TopologyResponse {
  daemon: DaemonInfo;
  backends: TopologyBackend[];
  recent_calls: CallEvent[];
}

export interface DaemonInfo {
  total_tools: number;
  total_backends: number;
  status: "healthy" | "degraded";
  uptime_seconds: number;
}

export interface TopologyBackend {
  name: string;
  state: string;
  available: boolean;
  tool_count: number;
  rss_mb: number | null;
  calls: number;
}

// /api/backends
export interface BackendDetail {
  name: string;
  state: string;
  available: boolean;
  tool_count: number;
  pid: number | null;
  rss_mb: number | null;
  peak_rss_mb: number | null;
  p50_ms: number | null;
  p95_ms: number | null;
  calls: number;
  recent_stderr: string[];
}

// /api/stats
export interface SessionStats {
  uptime_seconds: number;
  total_calls: number;
  total_bytes_returned: number;
  total_bytes_processed: number;
  savings_ratio: number;
  reduction_pct: number;
  estimated_tokens_saved: number;
  per_tool: ToolStats[];
}

export interface ToolStats {
  name: string;
  calls: number;
  bytes_returned: number;
}

// /api/recent
export interface CallEvent {
  tool_name: string;
  backend_name: string;
  duration_ms: number;
  success: boolean;
  seconds_ago: number;
}

// /api/health
export interface HealthResponse {
  status: "healthy" | "degraded";
  total_tools: number;
  total_backends: number;
}

// Combined dashboard state
export interface DashboardData {
  topology: TopologyResponse | null;
  backends: BackendDetail[] | null;
  stats: SessionStats | null;
  recent: CallEvent[] | null;
}
```

- [ ] **Step 2: Create api.ts with fetch helpers and polling hook**

```typescript
import { useEffect, useRef, useState } from "react";
import type {
  BackendDetail,
  CallEvent,
  DashboardData,
  SessionStats,
  TopologyResponse,
} from "./types";

const POLL_INTERVAL = 2000;

async function fetchJson<T>(url: string): Promise<T | null> {
  try {
    const res = await fetch(url);
    if (!res.ok) return null;
    return res.json();
  } catch {
    return null;
  }
}

export function useDashboardData(): DashboardData & { connected: boolean } {
  const [data, setData] = useState<DashboardData>({
    topology: null,
    backends: null,
    stats: null,
    recent: null,
  });
  const [connected, setConnected] = useState(false);
  const intervalRef = useRef<ReturnType<typeof setInterval>>();

  useEffect(() => {
    async function poll() {
      const [topology, backends, stats, recent] = await Promise.all([
        fetchJson<TopologyResponse>("/api/topology"),
        fetchJson<BackendDetail[]>("/api/backends"),
        fetchJson<SessionStats>("/api/stats"),
        fetchJson<CallEvent[]>("/api/recent"),
      ]);

      setConnected(topology !== null);
      setData({ topology, backends, stats, recent });
    }

    poll();
    intervalRef.current = setInterval(poll, POLL_INTERVAL);
    return () => clearInterval(intervalRef.current);
  }, []);

  return { ...data, connected };
}
```

- [ ] **Step 3: Create utils.ts with formatting helpers**

```typescript
export function formatUptime(seconds: number): string {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = Math.floor(seconds % 60);
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes}B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)}KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)}MB`;
}

export function formatTimeAgo(seconds: number): string {
  if (seconds < 60) return `${Math.floor(seconds)}s ago`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  return `${Math.floor(seconds / 3600)}h ago`;
}

export function formatMs(ms: number | null): string {
  if (ms === null) return "—";
  if (ms < 1000) return `${Math.round(ms)}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

export function stateColor(state: string): string {
  switch (state) {
    case "Healthy":
      return "var(--color-healthy)";
    case "Starting":
      return "var(--color-starting)";
    case "Unhealthy":
      return "var(--color-unhealthy)";
    case "Stopped":
      return "var(--color-stopped)";
    default:
      return "var(--color-text-muted)";
  }
}
```

- [ ] **Step 4: Commit data layer**

```bash
git add web/src/types.ts web/src/api.ts web/src/utils.ts
git commit -m "feat(dashboard): add API types, polling hook, and format utilities"
```

---

## Task 3: Header Component

**Files:**
- Create: `web/src/components/Header.tsx`
- Modify: `web/src/App.tsx`

- [ ] **Step 1: Create Header.tsx**

```tsx
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
```

- [ ] **Step 2: Wire Header into App.tsx**

```tsx
import { useDashboardData } from "./api";
import Header from "./components/Header";

export default function App() {
  const { topology, backends, stats, recent, connected } = useDashboardData();

  return (
    <div className="min-h-screen flex flex-col">
      <Header
        daemon={topology?.daemon ?? null}
        stats={stats}
        connected={connected}
      />
      <main className="flex-1 p-6 space-y-6">
        {/* Topology, BackendGrid, RecentCalls, StatsFooter go here */}
        <p className="text-text-muted font-mono text-sm">
          {connected
            ? `Monitoring ${topology?.daemon.total_backends ?? 0} backends...`
            : "Connecting to gatemini daemon..."}
        </p>
      </main>
    </div>
  );
}
```

- [ ] **Step 3: Verify Header renders with live data**

```bash
cd web && npm run dev
```

Open `http://localhost:3000` — header should show status, metrics, and savings chip with live data from `127.0.0.1:19999`.

- [ ] **Step 4: Commit**

```bash
git add web/src/components/Header.tsx web/src/App.tsx
git commit -m "feat(dashboard): add header bar with status, metrics, and savings chip"
```

---

## Task 4: Topology Visualization

**Files:**
- Create: `web/src/components/Topology.tsx`
- Modify: `web/src/App.tsx`

This is the hero element. An SVG diagram showing the daemon as a central node with edges radiating to backend nodes. Edges animate with particle flow when backends have recent calls.

- [ ] **Step 1: Create Topology.tsx**

```tsx
import { useRef, useEffect, useMemo } from "react";
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
```

- [ ] **Step 2: Wire Topology into App.tsx**

Update `App.tsx` main section:
```tsx
import Topology from "./components/Topology";

// Inside the <main> element, replace the placeholder:
{topology && (
  <Topology
    backends={topology.backends}
    recentCalls={topology.recent_calls}
    onSelectBackend={(name) => {
      document.getElementById(`backend-${name}`)?.scrollIntoView({ behavior: "smooth" });
    }}
  />
)}
```

- [ ] **Step 3: Verify topology renders with live data**

Open `http://localhost:3000` — should see SVG with daemon center and backend nodes. Active backends should show glowing edges with particles.

- [ ] **Step 4: Commit**

```bash
git add web/src/components/Topology.tsx web/src/App.tsx
git commit -m "feat(dashboard): add live SVG topology with animated particle flow"
```

---

## Task 5: Backend Detail Cards

**Files:**
- Create: `web/src/components/BackendCard.tsx`
- Create: `web/src/components/BackendGrid.tsx`
- Create: `web/src/components/LatencySparkline.tsx`
- Modify: `web/src/App.tsx`

- [ ] **Step 1: Create LatencySparkline.tsx**

A tiny Recharts area chart for latency visualization.

```tsx
import { Area, AreaChart, ResponsiveContainer } from "recharts";

interface LatencySparklineProps {
  p50: number | null;
  p95: number | null;
  calls: number;
}

export default function LatencySparkline({
  p50,
  p95,
  calls,
}: LatencySparklineProps) {
  // Generate synthetic sparkline points from p50/p95
  // In production you'd have per-call latency history; for now approximate
  if (p50 === null || calls === 0) {
    return (
      <div className="h-8 flex items-center justify-center text-text-dim text-xs font-mono">
        no data
      </div>
    );
  }

  const points = Array.from({ length: 20 }, (_, i) => {
    const base = p50;
    const jitter = (p95! - p50) * 0.3;
    const val = base + (Math.sin(i * 0.8) + Math.random() * 0.5) * jitter;
    return { i, v: Math.max(0, val) };
  });

  return (
    <ResponsiveContainer width="100%" height={32}>
      <AreaChart data={points}>
        <defs>
          <linearGradient id="sparkGrad" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="var(--color-accent)" stopOpacity={0.3} />
            <stop offset="100%" stopColor="var(--color-accent)" stopOpacity={0} />
          </linearGradient>
        </defs>
        <Area
          type="monotone"
          dataKey="v"
          stroke="var(--color-accent)"
          strokeWidth={1.5}
          fill="url(#sparkGrad)"
          dot={false}
          isAnimationActive={false}
        />
      </AreaChart>
    </ResponsiveContainer>
  );
}
```

- [ ] **Step 2: Create BackendCard.tsx**

```tsx
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
          {/* State dot */}
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
          {/* Metrics row */}
          <div className="grid grid-cols-4 gap-3 text-xs">
            <div>
              <span className="text-text-dim block">PID</span>
              <span className="font-mono font-medium">
                {b.pid ?? "—"}
              </span>
            </div>
            <div>
              <span className="text-text-dim block">p50</span>
              <span className="font-mono font-medium">
                {formatMs(b.p50_ms)}
              </span>
            </div>
            <div>
              <span className="text-text-dim block">p95</span>
              <span className="font-mono font-medium">
                {formatMs(b.p95_ms)}
              </span>
            </div>
            <div>
              <span className="text-text-dim block">Calls</span>
              <span className="font-mono font-medium">{b.calls}</span>
            </div>
          </div>

          {/* Memory bar */}
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

          {/* Latency sparkline */}
          <div>
            <span className="text-xs text-text-dim block mb-1">
              Latency trend
            </span>
            <LatencySparkline
              p50={b.p50_ms}
              p95={b.p95_ms}
              calls={b.calls}
            />
          </div>

          {/* Stderr log */}
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
```

- [ ] **Step 3: Create BackendGrid.tsx**

```tsx
import BackendCard from "./BackendCard";
import type { BackendDetail } from "../types";

interface BackendGridProps {
  backends: BackendDetail[];
}

export default function BackendGrid({ backends }: BackendGridProps) {
  if (backends.length === 0) return null;

  // Sort: unhealthy first, then by call count desc
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
```

- [ ] **Step 4: Wire into App.tsx**

Add to imports and main section:
```tsx
import BackendGrid from "./components/BackendGrid";

// In <main>:
{backends && <BackendGrid backends={backends} />}
```

- [ ] **Step 5: Verify cards render and expand**

Open `http://localhost:3000` — backend cards should display in grid, expanding on click to show memory bar, sparkline, and stderr.

- [ ] **Step 6: Commit**

```bash
git add web/src/components/BackendCard.tsx web/src/components/BackendGrid.tsx web/src/components/LatencySparkline.tsx web/src/App.tsx
git commit -m "feat(dashboard): add backend detail cards with memory bars, sparklines, and stderr"
```

---

## Task 6: Recent Calls Table

**Files:**
- Create: `web/src/components/RecentCalls.tsx`
- Modify: `web/src/App.tsx`

- [ ] **Step 1: Create RecentCalls.tsx**

```tsx
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
```

- [ ] **Step 2: Wire into App.tsx**

```tsx
import RecentCalls from "./components/RecentCalls";

// In <main>:
{recent && <RecentCalls calls={recent} />}
```

- [ ] **Step 3: Commit**

```bash
git add web/src/components/RecentCalls.tsx web/src/App.tsx
git commit -m "feat(dashboard): add recent calls table with status and highlight"
```

---

## Task 7: Stats Footer

**Files:**
- Create: `web/src/components/StatsFooter.tsx`
- Modify: `web/src/App.tsx`

- [ ] **Step 1: Create StatsFooter.tsx**

```tsx
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
```

- [ ] **Step 2: Wire into App.tsx**

```tsx
import StatsFooter from "./components/StatsFooter";

// In <main>:
{stats && <StatsFooter stats={stats} />}
```

- [ ] **Step 3: Commit**

```bash
git add web/src/components/StatsFooter.tsx web/src/App.tsx
git commit -m "feat(dashboard): add stats footer with per-tool breakdown"
```

---

## Task 8: Final App.tsx Assembly

**Files:**
- Modify: `web/src/App.tsx`

- [ ] **Step 1: Complete App.tsx with all components**

```tsx
import { useDashboardData } from "./api";
import Header from "./components/Header";
import Topology from "./components/Topology";
import BackendGrid from "./components/BackendGrid";
import RecentCalls from "./components/RecentCalls";
import StatsFooter from "./components/StatsFooter";

export default function App() {
  const { topology, backends, stats, recent, connected } = useDashboardData();

  return (
    <div className="min-h-screen flex flex-col bg-surface-900">
      <Header
        daemon={topology?.daemon ?? null}
        stats={stats}
        connected={connected}
      />
      <main className="flex-1 p-6 space-y-6 max-w-[1600px] mx-auto w-full">
        {!connected && (
          <div className="text-center py-12 text-text-muted">
            <p className="font-mono text-sm animate-pulse">
              Connecting to gatemini daemon...
            </p>
          </div>
        )}
        {topology && (
          <Topology
            backends={topology.backends}
            recentCalls={topology.recent_calls}
            onSelectBackend={(name) => {
              document
                .getElementById(`backend-${name}`)
                ?.scrollIntoView({ behavior: "smooth", block: "center" });
            }}
          />
        )}
        {backends && <BackendGrid backends={backends} />}
        {recent && <RecentCalls calls={recent} />}
        {stats && <StatsFooter stats={stats} />}
      </main>
    </div>
  );
}
```

- [ ] **Step 2: Verify full dashboard works end-to-end**

```bash
cd web && npm run dev
```

All 5 sections should render with live data polled every 2 seconds from the gatemini daemon.

- [ ] **Step 3: Build production bundle**

```bash
cd web && npm run build
```

Expected: `web/dist/` contains `index.html` and `assets/` with hashed JS/CSS.

- [ ] **Step 4: Commit**

```bash
git add web/src/App.tsx
git commit -m "feat(dashboard): assemble complete dashboard with all sections"
```

---

## Task 9: Rust Static File Serving

**Files:**
- Modify: `Cargo.toml` — add `tower-http` dependency
- Modify: `src/admin.rs` — serve `web/dist/` with SPA fallback

- [ ] **Step 1: Add tower-http to Cargo.toml**

Under `[dependencies]`, add:
```toml
tower-http = { version = "0.6", features = ["fs"], optional = true }
```

Under `[features]`, update:
```toml
admin = ["dep:axum", "dep:tower-http"]
```

- [ ] **Step 2: Update admin.rs to serve static files**

Replace the `dashboard()` handler and route with `tower-http::services::{ServeDir, ServeFile}`. Remove the old `dashboard()` handler entirely.

```rust
use tower_http::services::{ServeDir, ServeFile};

// In the start() function, replace:
//   .route("/", get(dashboard))
// with:
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let assets_dir = manifest_dir.join("web/dist");
    let serve_dir = if assets_dir.exists() {
        // SPA: serve static files, fallback unmatched paths to index.html
        ServeDir::new(&assets_dir)
            .not_found_service(ServeFile::new(assets_dir.join("index.html")))
    } else {
        // web/dist not built — serve legacy single-file dashboard
        ServeDir::new(".")
            .not_found_service(ServeFile::new(manifest_dir.join("web/dashboard.html")))
    };

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/backends", get(backends))
        .route("/api/discovery", get(discovery))
        .route("/api/recent", get(recent))
        .route("/api/stats", get(stats))
        .route("/api/topology", get(topology))
        .fallback_service(serve_dir)
        .with_state(state);
```

Delete the old `dashboard()` handler — it is no longer needed. `ServeFile` handles both the SPA and legacy fallback cases.

- [ ] **Step 3: Build and verify Rust compiles**

```bash
cargo build --features admin
```

- [ ] **Step 4: Test: build React app, then start gatemini, verify dashboard at :19999**

```bash
cd web && npm run build && cd ..
# Start gatemini (with admin.enabled: true in config)
# Open http://127.0.0.1:19999 — should show React dashboard
```

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml src/admin.rs
git commit -m "feat(dashboard): serve React build via tower-http with HTML fallback"
```

---

## Task 10: Polish and .gitignore

**Files:**
- Modify: `.gitignore` — add `web/node_modules/` and `web/dist/`

- [ ] **Step 1: Update .gitignore**

Add to `.gitignore`:
```
web/node_modules/
web/dist/
```

- [ ] **Step 2: Final verification**

1. `cd web && npm ci && npm run build` — no errors
2. `cargo build --features admin` — no errors
3. Start gatemini, open `http://127.0.0.1:19999`
4. All 5 sections render with live data
5. Topology particles animate on active backends
6. Backend cards expand with memory bars, sparklines, stderr
7. Recent calls highlight new entries
8. Stats footer shows savings and per-tool breakdown

- [ ] **Step 3: Commit**

```bash
git add .gitignore
git commit -m "chore: add web build artifacts to .gitignore"
```
