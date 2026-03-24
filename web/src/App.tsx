import { useDashboardData } from "./api";
import Header from "./components/Header";
import Topology from "./components/Topology";
import BackendGrid from "./components/BackendGrid";
import RecentCalls from "./components/RecentCalls";

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
        {topology && (
          <Topology
            backends={topology.backends}
            recentCalls={topology.recent_calls}
            onSelectBackend={(name) => {
              document.getElementById(`backend-${name}`)?.scrollIntoView({ behavior: "smooth" });
            }}
          />
        )}
        {backends && <BackendGrid backends={backends} />}
        {recent && <RecentCalls calls={recent} />}
        <p className="text-text-muted font-mono text-sm">
          {connected
            ? `Monitoring ${topology?.daemon.total_backends ?? 0} backends...`
            : "Connecting to gatemini daemon..."}
        </p>
      </main>
    </div>
  );
}
