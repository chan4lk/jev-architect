import { cn } from "@/lib/utils";

interface ConfidenceMeterProps {
  /** 0..1 */
  confidence: number;
  /** 0..1, from settings.confidence_threshold */
  threshold: number;
}

/** A 0–1 confidence meter with a tick mark at the configured threshold. */
export function ConfidenceMeter({ confidence, threshold }: ConfidenceMeterProps) {
  const pct = Math.round(confidence * 100);
  const thresholdPct = Math.round(threshold * 100);
  const meetsThreshold = confidence >= threshold;

  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span>Confidence</span>
        <span className="tabular-nums">
          {pct}% (threshold {thresholdPct}%)
        </span>
      </div>
      <div
        className="relative h-2 w-full overflow-hidden rounded-full bg-muted"
        role="meter"
        aria-label="Confidence"
        aria-valuenow={pct}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuetext={`${pct}% confidence, threshold ${thresholdPct}%`}
      >
        <div
          className={cn("h-full rounded-full", meetsThreshold ? "bg-primary" : "bg-destructive/70")}
          style={{ width: `${pct}%` }}
        />
        <div
          aria-hidden="true"
          className="absolute top-0 h-full w-0.5 bg-foreground/70"
          style={{ left: `${thresholdPct}%` }}
        />
      </div>
    </div>
  );
}
