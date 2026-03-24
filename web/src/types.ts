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
