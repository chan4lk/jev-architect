import { useEffect, useState } from "react";
import { Link } from "react-router-dom";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Label } from "@/components/ui/label";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { RingBadge } from "@/components/RingBadge";
import type { DecisionView, ReviewAction } from "@/lib/schemas";

export interface ReviewSubmitArgs {
  decisionId: string;
  action: ReviewAction;
  optionId?: string;
  reason?: string;
}

interface ReviewDialogProps {
  decision: DecisionView | null;
  onOpenChange: (open: boolean) => void;
  reviewerName: string;
  onSubmit: (args: ReviewSubmitArgs) => void;
  isPending: boolean;
  /** Server-side (AppError) message from the last failed submit, if any. */
  serverError: string | null;
}

type Mode = "override" | "reject" | null;

/** Accept / Override / Reject actions for one decision (FR-18, AC-13). */
export function ReviewDialog({
  decision,
  onOpenChange,
  reviewerName,
  onSubmit,
  isPending,
  serverError,
}: ReviewDialogProps) {
  const [mode, setMode] = useState<Mode>(null);
  const [optionId, setOptionId] = useState("");
  const [reason, setReason] = useState("");
  const [formError, setFormError] = useState<string | null>(null);

  // Reset local form state every time a different decision is opened.
  useEffect(() => {
    setMode(null);
    setOptionId("");
    setReason("");
    setFormError(null);
  }, [decision?.decision.id]);

  const reviewerMissing = !reviewerName.trim();
  const alternatives = decision?.options.filter((o) => o.id !== decision.decision.choice) ?? [];
  const message = formError ?? serverError;

  function submit(action: ReviewAction) {
    if (!decision) return;
    if (reviewerMissing) {
      setFormError("Set your reviewer name to approve decisions.");
      return;
    }
    if (action === "override" && !optionId) {
      setFormError("Select an option to override to.");
      return;
    }
    if ((action === "override" || action === "reject") && !reason.trim()) {
      setFormError("A reason is required to override or reject a decision.");
      return;
    }
    setFormError(null);
    onSubmit({
      decisionId: decision.decision.id,
      action,
      optionId: action === "override" ? optionId : undefined,
      reason: reason.trim() || undefined,
    });
  }

  return (
    <Dialog open={decision !== null} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Review — {decision?.type_name}</DialogTitle>
          <DialogDescription>
            {reviewerMissing ? (
              <>
                Set your reviewer name to approve decisions.{" "}
                <Link to="/settings" className="underline underline-offset-2">
                  Go to Settings
                </Link>
                .
              </>
            ) : (
              `Reviewing as ${reviewerName}`
            )}
          </DialogDescription>
        </DialogHeader>

        {message && (
          <Alert variant="destructive">
            <AlertDescription>{message}</AlertDescription>
          </Alert>
        )}

        <div className="flex flex-wrap gap-2">
          <Button
            disabled={reviewerMissing || isPending}
            onClick={() => submit("accept")}
          >
            Accept
          </Button>
          <Button
            variant="outline"
            disabled={reviewerMissing || isPending}
            onClick={() => setMode(mode === "override" ? null : "override")}
          >
            Override
          </Button>
          <Button
            variant="outline"
            disabled={reviewerMissing || isPending}
            onClick={() => setMode(mode === "reject" ? null : "reject")}
          >
            Reject
          </Button>
        </div>

        {mode === "override" && (
          <div className="space-y-3 rounded-lg border border-border p-3">
            <div className="space-y-1.5">
              <span className="text-sm font-medium">Override to</span>
              <div className="space-y-1.5">
                {alternatives.map((option) => (
                  <label key={option.id} className="flex items-center gap-2 text-sm">
                    <input
                      type="radio"
                      name="override-option"
                      value={option.id}
                      checked={optionId === option.id}
                      onChange={() => setOptionId(option.id)}
                    />
                    {option.name}
                    <RingBadge ring={option.ring} />
                  </label>
                ))}
              </div>
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="override-reason">Reason</Label>
              <Textarea
                id="override-reason"
                value={reason}
                onChange={(event) => setReason(event.target.value)}
                placeholder="Why override the AI's choice?"
              />
            </div>
            <Button disabled={isPending} onClick={() => submit("override")}>
              Confirm override
            </Button>
          </div>
        )}

        {mode === "reject" && (
          <div className="space-y-3 rounded-lg border border-border p-3">
            <div className="space-y-1.5">
              <Label htmlFor="reject-reason">Reason</Label>
              <Textarea
                id="reject-reason"
                value={reason}
                onChange={(event) => setReason(event.target.value)}
                placeholder="Why reject this decision?"
              />
            </div>
            <Button variant="destructive" disabled={isPending} onClick={() => submit("reject")}>
              Confirm reject
            </Button>
          </div>
        )}

        <DialogFooter showCloseButton />
      </DialogContent>
    </Dialog>
  );
}
