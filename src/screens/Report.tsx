import { useState } from "react";
import { useParams } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Alert, AlertDescription } from "@/components/ui/alert";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { ReportSummary } from "@/components/ReportSummary";
import { DecisionCard } from "@/components/DecisionCard";
import { ReviewDialog, type ReviewSubmitArgs } from "@/components/ReviewDialog";
import { api } from "@/lib/ipc";
import { AppError, type DecisionView, type SessionView } from "@/lib/schemas";
import "@/report.print.css";

type Filter = "all" | "needs_architect" | "unreviewed";

function filterDecisions(decisions: DecisionView[], filter: Filter): DecisionView[] {
  if (filter === "needs_architect") {
    return decisions.filter((d) => d.decision.route === "needs_architect");
  }
  if (filter === "unreviewed") {
    return decisions.filter((d) => d.status === "proposed_ai");
  }
  return decisions;
}

export function Report() {
  const { id } = useParams<{ id: string }>();
  const sessionId = id ?? "";
  const queryClient = useQueryClient();

  const [filter, setFilter] = useState<Filter>("all");
  const [reviewTarget, setReviewTarget] = useState<DecisionView | null>(null);
  const [adrConfirmOpen, setAdrConfirmOpen] = useState(false);

  const sessionQuery = useQuery({
    queryKey: ["session", sessionId],
    queryFn: () => api.getSession(sessionId),
    enabled: sessionId.length > 0,
  });
  const settingsQuery = useQuery({ queryKey: ["settings"], queryFn: api.getSettings });

  const reviewMutation = useMutation({
    mutationFn: (args: ReviewSubmitArgs) =>
      api.review({
        decisionId: args.decisionId,
        action: args.action,
        optionId: args.optionId,
        reason: args.reason,
      }),
    onSuccess: (updated) => {
      queryClient.setQueryData<SessionView | undefined>(["session", sessionId], (old) => {
        if (!old || !old.report) return old;
        return {
          ...old,
          report: {
            ...old.report,
            decisions: old.report.decisions.map((d) =>
              d.decision.id === updated.decision.id ? updated : d,
            ),
          },
        };
      });
      toast.success(`Review recorded: ${updated.status_label}`);
      setReviewTarget(null);
    },
    onError: () => {
      // Surfaced inline in ReviewDialog via reviewMutation.error below.
    },
  });

  const exportAdrsMutation = useMutation({
    mutationFn: async () => {
      const dir = await api.pickFolder();
      if (!dir) return null;
      const paths = await api.exportAdrs(sessionId, dir);
      return { dir, paths };
    },
    onSuccess: (result) => {
      setAdrConfirmOpen(false);
      if (!result) return;
      toast.success(
        `Exported ${result.paths.length} ADR${result.paths.length === 1 ? "" : "s"} to ${result.dir}`,
      );
    },
    onError: (err) => {
      toast.error(err instanceof AppError ? err.message : "Could not export ADRs.");
    },
  });

  const exportReportMutation = useMutation({
    mutationFn: async (format: "md" | "html") => {
      const dir = await api.pickFolder();
      if (!dir) return null;
      return api.exportReport(sessionId, dir, format);
    },
    onSuccess: (path) => {
      if (path) toast.success(`Report exported to ${path}`);
    },
    onError: (err) => {
      toast.error(err instanceof AppError ? err.message : "Could not export the report.");
    },
  });

  if (sessionQuery.isLoading) {
    return <div className="mx-auto max-w-4xl p-6 text-sm text-muted-foreground">Loading report…</div>;
  }

  const view = sessionQuery.data;

  if (!view || !view.report) {
    return (
      <div className="mx-auto max-w-4xl space-y-2 p-6">
        <h1 className="text-xl font-semibold">Report</h1>
        <p className="text-sm text-muted-foreground">
          This session doesn't have a report yet — it may still be running, or it stopped before
          reaching the report stage.
        </p>
      </div>
    );
  }

  const report = view.report;
  const decisions = report.decisions;
  const needsArchitectCount = decisions.filter((d) => d.decision.route === "needs_architect").length;
  const unreviewedCount = decisions.filter((d) => d.status === "proposed_ai").length;
  const filtered = filterDecisions(decisions, filter);
  const reviewerName = settingsQuery.data?.reviewer_name ?? "";
  const confidenceThreshold = settingsQuery.data?.confidence_threshold ?? 0.5;
  const serverError =
    reviewMutation.error instanceof AppError
      ? reviewMutation.error.message
      : reviewMutation.error
        ? "Could not record the review."
        : null;

  return (
    <div className="mx-auto max-w-4xl space-y-6 p-6">
      <h1 className="text-xl font-semibold">Report</h1>

      <ReportSummary report={report} brief={view.brief} />

      <div className="flex flex-wrap items-center justify-between gap-3 print:hidden">
        <Tabs value={filter} onValueChange={(value) => setFilter(value as Filter)}>
          <TabsList>
            <TabsTrigger value="all">All ({decisions.length})</TabsTrigger>
            <TabsTrigger value="needs_architect">
              Needs architect ({needsArchitectCount})
            </TabsTrigger>
            <TabsTrigger value="unreviewed">Unreviewed ({unreviewedCount})</TabsTrigger>
          </TabsList>
        </Tabs>

        <div className="flex flex-wrap gap-2">
          <Button variant="outline" size="sm" onClick={() => setAdrConfirmOpen(true)}>
            Export ADRs
          </Button>
          <Button
            variant="outline"
            size="sm"
            disabled={exportReportMutation.isPending}
            onClick={() => exportReportMutation.mutate("md")}
          >
            Export Markdown
          </Button>
          <Button
            variant="outline"
            size="sm"
            disabled={exportReportMutation.isPending}
            onClick={() => exportReportMutation.mutate("html")}
          >
            Export HTML
          </Button>
          <Button variant="outline" size="sm" onClick={() => window.print()}>
            Print / Save as PDF
          </Button>
        </div>
      </div>

      <div className="space-y-4">
        {filtered.length === 0 && (
          <p className="text-sm text-muted-foreground">No decisions match this filter.</p>
        )}
        {filtered.map((decisionView) => (
          <DecisionCard
            key={decisionView.decision.id}
            view={decisionView}
            criteria={report.criteria}
            confidenceThreshold={confidenceThreshold}
            onReview={() => setReviewTarget(decisionView)}
          />
        ))}
      </div>

      {report.not_applicable.length > 0 && (
        <details className="rounded-lg border border-border p-3 print:hidden">
          <summary className="cursor-pointer text-sm font-medium">
            Not applicable ({report.not_applicable.length})
          </summary>
          <ul className="mt-2 space-y-1 text-sm text-muted-foreground">
            {report.not_applicable.map((na) => (
              <li key={na.type_id}>
                {na.type_name} — p={na.probability.toFixed(2)}
              </li>
            ))}
          </ul>
        </details>
      )}

      <ReviewDialog
        decision={reviewTarget}
        onOpenChange={(open) => {
          if (!open) {
            setReviewTarget(null);
            reviewMutation.reset();
          }
        }}
        reviewerName={reviewerName}
        onSubmit={(args) => reviewMutation.mutate(args)}
        isPending={reviewMutation.isPending}
        serverError={serverError}
      />

      <Dialog open={adrConfirmOpen} onOpenChange={setAdrConfirmOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Export ADRs</DialogTitle>
            <DialogDescription>
              Writes one ADR file per decision to a folder you choose.
            </DialogDescription>
          </DialogHeader>
          {unreviewedCount > 0 && (
            <Alert variant="destructive">
              <AlertDescription>
                {unreviewedCount} decision{unreviewedCount === 1 ? "" : "s"} will export as
                Proposed (AI) — pending approval
              </AlertDescription>
            </Alert>
          )}
          <DialogFooter>
            <Button variant="outline" onClick={() => setAdrConfirmOpen(false)}>
              Cancel
            </Button>
            <Button
              disabled={exportAdrsMutation.isPending}
              onClick={() => exportAdrsMutation.mutate()}
            >
              {exportAdrsMutation.isPending ? "Choosing folder…" : "Choose folder & export"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

export default Report;
