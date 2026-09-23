import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { RingBadge } from "@/components/RingBadge";
import { ProbBars } from "@/components/ProbBars";
import { ConfidenceMeter } from "@/components/ConfidenceMeter";
import { ScoreHeatmap } from "@/components/ScoreHeatmap";
import type { AdrStatus, Criterion, DecisionView, OptionView, ReasonCode, Review, Route } from "@/lib/schemas";

export const REASON_LABEL: Record<ReasonCode, string> = {
  low_confidence: "Low confidence",
  disagreement: "Scores disagree with Jev's choice",
  close_margin: "Close call",
  hold_option: "BISTEC says avoid",
  possible_injection: "Possible prompt injection",
};

export const ROUTE_LABEL: Record<Route, string> = {
  proposed: "Proposed",
  needs_architect: "Needs architect",
};

function statusBadgeVariant(status: AdrStatus): "default" | "secondary" | "outline" | "destructive" {
  switch (status) {
    case "accepted":
      return "default";
    case "accepted_override":
      return "secondary";
    case "rejected":
      return "destructive";
    case "proposed_ai":
    default:
      return "outline";
  }
}

function reviewSummary(review: Review, options: OptionView[]): string {
  switch (review.action) {
    case "accept":
      return "accepted this decision.";
    case "override": {
      const option = options.find((o) => o.id === review.option_id);
      const target = option?.name ?? review.option_id ?? "another option";
      return `overrode to ${target}${review.reason ? ` — ${review.reason}` : ""}.`;
    }
    case "reject":
      return `rejected this decision${review.reason ? ` — ${review.reason}` : ""}.`;
  }
}

interface DecisionCardProps {
  view: DecisionView;
  criteria: Criterion[];
  confidenceThreshold: number;
  onReview: () => void;
}

/** One card per decision (FR-16): choice, ring, route/reasons, probabilities,
 * confidence, score heatmap, cited sections, evidence footer, and review history. */
export function DecisionCard({ view, criteria, confidenceThreshold, onReview }: DecisionCardProps) {
  const { decision, options, type_name, reviews, status, status_label } = view;
  const chosenOption = options.find((o) => o.id === decision.choice);

  return (
    <Card className="break-inside-avoid report-decision-card">
      <CardHeader>
        <CardTitle>
          <h3 className="font-heading text-base leading-snug font-medium">{type_name}</h3>
        </CardTitle>
        <CardDescription className="flex flex-wrap items-center gap-2 text-foreground">
          <span className="font-medium">{chosenOption?.name ?? decision.choice}</span>
          {chosenOption && <RingBadge ring={chosenOption.ring} />}
        </CardDescription>
        <CardAction className="flex flex-col items-end gap-1.5">
          <Badge variant={decision.route === "proposed" ? "default" : "secondary"}>
            {ROUTE_LABEL[decision.route]}
          </Badge>
          <Badge variant={statusBadgeVariant(status)}>{status_label}</Badge>
        </CardAction>
      </CardHeader>

      <CardContent className="space-y-4">
        {decision.reasons.length > 0 && (
          <ul className="flex flex-wrap gap-1.5">
            {decision.reasons.map((reason) => (
              <li key={reason}>
                <Badge variant="outline">{REASON_LABEL[reason]}</Badge>
              </li>
            ))}
          </ul>
        )}

        <ProbBars options={options} probabilities={decision.probabilities} choice={decision.choice} />
        <ConfidenceMeter confidence={decision.confidence} threshold={confidenceThreshold} />
        <ScoreHeatmap options={options} optionScores={decision.option_scores} criteria={criteria} />

        {decision.cited_sections.length > 0 && (
          <div className="flex flex-wrap items-center gap-1.5">
            <span className="text-xs text-muted-foreground">Cited sections:</span>
            {decision.cited_sections.map((section) => (
              <Badge key={section} variant="outline">
                {section}
              </Badge>
            ))}
          </div>
        )}

        {reviews.length > 0 && (
          <div className="space-y-1 border-t border-border pt-3">
            <h4 className="text-xs font-medium text-muted-foreground">Review history</h4>
            <ul className="space-y-1 text-sm">
              {reviews.map((review) => (
                <li key={review.id}>
                  <span className="font-medium">{review.reviewer}</span>{" "}
                  {reviewSummary(review, options)}{" "}
                  <span className="text-xs text-muted-foreground">
                    {new Date(review.at_utc).toLocaleString()}
                  </span>
                </li>
              ))}
            </ul>
          </div>
        )}
      </CardContent>

      <CardFooter className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-xs text-muted-foreground">
          Evidence — model {decision.model_snapshot} · request {decision.request_hash}
        </p>
        <Button size="sm" variant="outline" onClick={onReview} className="print:hidden">
          Review
        </Button>
      </CardFooter>
    </Card>
  );
}
