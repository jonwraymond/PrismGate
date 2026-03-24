# React Dashboard for Gatemini

**Date:** 2026-03-24
**Status:** Approved
**Goal:** Replace the embedded HTML dashboard with a proper React TypeScript app

## Stack

- Vite + React 19 + TypeScript
- Tailwind CSS v4
- Recharts (sparklines, latency charts)
- Lucide React (icons)
- No routing needed (single page)

## Architecture

- Source: `web/` directory (Vite project)
- Build output: `web/dist/`
- Embedded in binary via `include_dir!` crate or served from filesystem
- Admin API already exists at `127.0.0.1:19999` with these endpoints:
  - `GET /api/topology` — daemon info + backends + recent calls
  - `GET /api/backends` — detailed backend list with PID, RSS, latency, stderr
  - `GET /api/stats` — session byte tracking
  - `GET /api/recent` — last 50 call events
  - `GET /api/health` — overall health status
  - `GET /api/discovery` — all tools with schemas
- Dashboard polls every 2s

## Pages / Sections

### 1. Header Bar
- Gatemini logo/name + status dot (green/yellow)
- Backend count, tool count, uptime
- Context savings chip (bytes returned vs processed, % saved)

### 2. Live Topology (top section)
- SVG/Canvas node graph: daemon center → backend nodes radiating out
- Animated data flow particles along edges when calls happen
- Backend nodes colored by state (green=healthy, blue=starting, red=unhealthy, gray=stopped)
- Node size proportional to call volume
- Click node → scroll to detail card

### 3. Backend Detail Cards (grid)
- State badge (Healthy/Starting/Unhealthy/Stopped)
- Tool count, call count, p50/p95 latency
- RSS memory bar with peak indicator
- PID display
- Sparkline of recent call latency (last 20 calls)
- Stderr log viewer (last 20 lines, monospace, auto-scroll)
- Expandable/collapsible

### 4. Recent Calls Table
- Tool name, backend, duration, success/fail, time ago
- Color-coded rows (green=success, red=fail)
- Auto-updating with new calls highlighted

### 5. Stats Footer
- Total calls, bytes returned, bytes processed
- Savings ratio, reduction %, estimated tokens saved
- Per-tool breakdown (expandable)

## Design

- Dark theme (slate-900 bg, slate-800 cards)
- Teal/cyan accent for active connections
- Green for healthy, amber for warnings, red for errors
- Inter font family
- Responsive: works on 1024px+ (not mobile-optimized)
- Smooth 200ms transitions on state changes
- Reduced motion support

## Build Integration

- `web/package.json` with build script
- CI: `cd web && npm ci && npm run build` before cargo build
- Rust serves static files from `web/dist/` or embeds via `include_dir`
- Fallback: if `web/dist/` doesn't exist, serve the current `web/dashboard.html`

## Existing API Response Shapes

### /api/topology
```json
{
  "daemon": { "total_tools": 150, "total_backends": 43, "status": "healthy", "uptime_seconds": 3600 },
  "backends": [{ "name": "exa", "state": "Healthy", "available": true, "tool_count": 5, "rss_mb": 45, "calls": 120 }],
  "recent_calls": [{ "tool_name": "web_search_exa", "backend_name": "exa", "duration_ms": 450, "success": true, "seconds_ago": 2.5 }]
}
```

### /api/backends
```json
[{
  "name": "exa", "state": "Healthy", "available": true, "tool_count": 5,
  "pid": 12345, "rss_mb": 45, "peak_rss_mb": 60,
  "p50_ms": 200, "p95_ms": 800, "calls": 120,
  "recent_stderr": ["line1", "line2"]
}]
```

### /api/stats
```json
{
  "uptime_seconds": 3600, "total_calls": 500,
  "total_bytes_returned": 50000, "total_bytes_processed": 500000,
  "savings_ratio": 10.0, "reduction_pct": 90.0,
  "estimated_tokens_saved": 112500,
  "per_tool": [{ "name": "web_search_exa", "calls": 50, "bytes_returned": 5000 }]
}
```
