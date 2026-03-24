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
  const intervalRef = useRef<ReturnType<typeof setInterval>>(undefined);

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
