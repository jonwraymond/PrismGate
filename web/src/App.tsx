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
