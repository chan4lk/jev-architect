import type { OptionView } from "@/lib/schemas";
import { cn } from "@/lib/utils";
import { RingDot } from "@/components/RingBadge";

interface ProbBarsProps {
  options: OptionView[];
  probabilities: Record<string, number>;
  /** The option id Jev chose — its bar is visually emphasised. */
  choice: string;
}

/** One horizontal bar per option, sorted by probability, highest first. */
export function ProbBars({ options, probabilities, choice }: ProbBarsProps) {
  const sorted = [...options].sort(
    (a, b) => (probabilities[b.id] ?? 0) - (probabilities[a.id] ?? 0),
  );

  return (
    <div className="space-y-1.5">
      <span className="text-xs font-medium text-muted-foreground">Option probabilities</span>
      {sorted.map((option) => {
        const value = probabilities[option.id] ?? 0;
        const pct = Math.round(value * 100);
        const isChosen = option.id === choice;
        return (
          <div key={option.id} className="flex items-center gap-2">
            <RingDot ring={option.ring} />
            <span
              className={cn("w-36 shrink-0 truncate text-sm", isChosen && "font-semibold")}
              title={option.name}
            >
              {option.name}
            </span>
            <div
              className="h-2 flex-1 overflow-hidden rounded-full bg-muted"
              role="img"
              aria-label={`${option.name}: ${pct}%`}
            >
              <div
                className={cn(
                  "h-full rounded-full",
                  isChosen ? "bg-primary" : "bg-muted-foreground/40",
                )}
                style={{ width: `${pct}%` }}
              />
            </div>
            <span className="w-10 shrink-0 text-right text-xs tabular-nums">{pct}%</span>
          </div>
        );
      })}
    </div>
  );
}
