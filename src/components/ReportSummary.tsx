import { TriangleAlertIcon } from "lucide-react";

import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Alert, AlertDescription } from "@/components/ui/alert";
import type { Brief, Report } from "@/lib/schemas";

interface ReportSummaryProps {
  report: Report;
  brief: Brief | null;
}

/** Brief summary, key context values, counts, cost, and gate banners (FR-16). */
export function ReportSummary({ report, brief }: ReportSummaryProps) {
  const decisions = report.decisions;
  const proposedCount = decisions.filter((d) => d.decision.route === "proposed").length;
  const needsArchitectCount = decisions.filter((d) => d.decision.route === "needs_architect").length;
  const acceptedCount = decisions.filter((d) => d.status === "accepted").length;
  const overrideCount = decisions.filter((d) => d.status === "accepted_override").length;
  const rejectedCount = decisions.filter((d) => d.status === "rejected").length;
  const unreviewedCount = decisions.filter((d) => d.status === "proposed_ai").length;
  const cost = report.cost_usd === null ? "n/a" : `$${report.cost_usd.toFixed(4)}`;

  return (
    <Card>
      <CardHeader>
        <CardTitle>
          <h2 className="font-heading text-base leading-snug font-medium">Report summary</h2>
        </CardTitle>
        {brief && <CardDescription>{brief.summary}</CardDescription>}
      </CardHeader>
      <CardContent className="space-y-4">
        {brief && (
          <dl className="grid grid-cols-2 gap-x-4 gap-y-1 text-sm sm:grid-cols-3">
            <div>
              <dt className="text-muted-foreground">Scale</dt>
              <dd className="capitalize">{brief.context.scale}</dd>
            </div>
            <div>
              <dt className="text-muted-foreground">Budget</dt>
              <dd className="capitalize">{brief.context.budget}</dd>
            </div>
            <div>
              <dt className="text-muted-foreground">Timeline</dt>
              <dd className="capitalize">{brief.context.timeline}</dd>
            </div>
            <div>
              <dt className="text-muted-foreground">Team size</dt>
              <dd className="capitalize">{brief.context.team_size}</dd>
            </div>
            <div>
              <dt className="text-muted-foreground">Data sensitivity</dt>
              <dd className="capitalize">{brief.context.data_sensitivity}</dd>
            </div>
            <div>
              <dt className="text-muted-foreground">Compliance</dt>
              <dd>
                {brief.context.compliance.length
                  ? brief.context.compliance.join(", ").toUpperCase()
                  : "None stated"}
              </dd>
            </div>
          </dl>
        )}

        <div className="flex flex-wrap gap-2">
          <Badge variant="outline">{decisions.length} decisions</Badge>
          <Badge variant="outline">{report.not_applicable.length} not applicable</Badge>
          <Badge>{proposedCount} Proposed</Badge>
          <Badge variant="secondary">{needsArchitectCount} Needs architect</Badge>
          <Badge variant="outline">{acceptedCount} Accepted</Badge>
          <Badge variant="outline">{overrideCount} Override</Badge>
          <Badge variant="outline">{rejectedCount} Rejected</Badge>
          <Badge variant="outline">{unreviewedCount} Unreviewed</Badge>
        </div>

        <div className="flex flex-wrap gap-4 text-sm">
          <span>
            Jev cost: <span className="font-medium tabular-nums">{cost}</span>
          </span>
          <span>
            Input tokens:{" "}
            <span className="font-medium tabular-nums">{report.input_tokens.toLocaleString()}</span>
          </span>
        </div>

        {report.gates.has_enough_context < 0.5 && (
          <Alert variant="destructive">
            <TriangleAlertIcon />
            <AlertDescription>
              Jev judged the brief thin — decisions may be unreliable
            </AlertDescription>
          </Alert>
        )}
        {report.truncated && (
          <Alert variant="destructive">
            <TriangleAlertIcon />
            <AlertDescription>
              Some evidence was dropped to fit the token budget
            </AlertDescription>
          </Alert>
        )}
        {report.gates.injection >= 0.3 && (
          <Alert variant="destructive">
            <TriangleAlertIcon />
            <AlertDescription>
              Possible instructions to the model were found in the input — every decision needs an
              architect
            </AlertDescription>
          </Alert>
        )}

        <div className="space-y-0.5 text-xs text-muted-foreground">
          <p>
            Criterion weights:{" "}
            {report.criteria.map((c) => `${c.name} ${Math.round(c.weight * 100)}%`).join(" · ")}
          </p>
          <p>Weights and thresholds are uncalibrated defaults</p>
        </div>
      </CardContent>
    </Card>
  );
}
