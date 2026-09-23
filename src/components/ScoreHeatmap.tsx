import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { RingBadge } from "@/components/RingBadge";
import { cn } from "@/lib/utils";
import type { Criterion, OptionScore, OptionView } from "@/lib/schemas";

interface ScoreHeatmapProps {
  options: OptionView[];
  optionScores: OptionScore[];
  criteria: Criterion[];
}

/** Background intensity per 0..4 rubric score, kept mild so the number stays readable. */
const SCORE_BG = ["bg-muted", "bg-primary/15", "bg-primary/30", "bg-primary/50", "bg-primary/70"];

/**
 * Options × criteria heatmap with a composite column. Hold options that
 * weren't asked Score questions (no matching entry in `optionScores`) render
 * "—" in every cell rather than a fabricated score.
 */
export function ScoreHeatmap({ options, optionScores, criteria }: ScoreHeatmapProps) {
  const scoresByOption = new Map(optionScores.map((s) => [s.option_id, s]));

  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>Option</TableHead>
          {criteria.map((criterion) => (
            <TableHead key={criterion.id} className="text-center">
              {criterion.name}
            </TableHead>
          ))}
          <TableHead className="text-center">Composite</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {options.map((option) => {
          const score = scoresByOption.get(option.id);
          return (
            <TableRow key={option.id}>
              <TableCell className="font-medium whitespace-normal">
                <span className="flex items-center gap-1.5">
                  {option.name}
                  <RingBadge ring={option.ring} />
                </span>
              </TableCell>
              {criteria.map((criterion) => {
                const value = score?.criterion_scores[criterion.id];
                return (
                  <TableCell
                    key={criterion.id}
                    className={cn(
                      "text-center tabular-nums",
                      value !== undefined && SCORE_BG[value],
                    )}
                  >
                    {value !== undefined ? value : "—"}
                  </TableCell>
                );
              })}
              <TableCell className="text-center font-medium tabular-nums">
                {score ? `${Math.round(score.composite * 100)}%` : "—"}
              </TableCell>
            </TableRow>
          );
        })}
      </TableBody>
    </Table>
  );
}
