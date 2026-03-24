import { Area, AreaChart, ResponsiveContainer } from "recharts";

interface LatencySparklineProps {
  p50: number | null;
  p95: number | null;
  calls: number;
  id: string;
}

export default function LatencySparkline({
  p50,
  p95,
  calls,
  id,
}: LatencySparklineProps) {
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
          <linearGradient id={`sparkGrad-${id}`} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="var(--color-accent)" stopOpacity={0.3} />
            <stop offset="100%" stopColor="var(--color-accent)" stopOpacity={0} />
          </linearGradient>
        </defs>
        <Area
          type="monotone"
          dataKey="v"
          stroke="var(--color-accent)"
          strokeWidth={1.5}
          fill={`url(#sparkGrad-${id})`}
          dot={false}
          isAnimationActive={false}
        />
      </AreaChart>
    </ResponsiveContainer>
  );
}
