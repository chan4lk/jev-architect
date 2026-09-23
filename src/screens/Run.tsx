import { useEffect, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { CheckIcon } from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { DataNotice } from "@/components/DataNotice";
import { api } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { AppError, type Progress as ProgressDto, type SessionView } from "@/lib/schemas";

const STAGE_ORDER = ["gates", "platforms", "applicability", "decisions", "done"] as const;
type StageId = (typeof STAGE_ORDER)[number];

const STAGE_LABELS: Record<StageId, string> = {
  gates: "Gates",
  platforms: "Platforms",
  applicability: "Applicability",
  decisions: "Decisions",
  done: "Done",
};

function toAppError(err: unknown): AppError {
  if (err instanceof AppError) return err;
  return new AppError({ code: "internal", message: "Something went wrong." });
}

/**
 * The decision-run progress screen (FR-15). Subscribes to pipeline progress
 * events, kicks off `run_decisions` (or resumes with `retry` after a failure),
 * and routes onward to the report once decisions are done.
 */
export function Run() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const [progress, setProgress] = useState<ProgressDto | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [running, setRunning] = useState(true);
  const [outOfRemit, setOutOfRemit] = useState(false);
  const [noticeOpen, setNoticeOpen] = useState(false);
  const [acking, setAcking] = useState(false);
  const startedRef = useRef(false);

  const noticeQuery = useQuery({ queryKey: ["data-notice"], queryFn: api.dataNotice });

  function handleResult(view: SessionView) {
    queryClient.setQueryData(["session", id], view);
    if (view.session.stage === "out_of_remit") {
      setOutOfRemit(true);
      setRunning(false);
      return;
    }
    navigate(`/session/${id}/report`);
  }

  function handleFailure(err: unknown) {
    setError(toAppError(err));
    setRunning(false);
  }

  useEffect(() => {
    if (!id) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    api.onProgress((p) => {
      if (p.session_id === id) setProgress(p);
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [id]);

  useEffect(() => {
    if (!id || startedRef.current) return;
    startedRef.current = true;

    (async () => {
      setRunning(true);
      setError(null);
      setOutOfRemit(false);
      try {
        const view = await api.getSession(id);
        if (view.session.stage === "done" || view.session.stage === "out_of_remit") {
          handleResult(view);
          return;
        }
        const result = await api.runDecisions(id);
        handleResult(result);
      } catch (err) {
        handleFailure(err);
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id]);

  async function handleRetry() {
    if (!id) return;
    setRunning(true);
    setError(null);
    try {
      const result = await api.retry(id);
      handleResult(result);
    } catch (err) {
      handleFailure(err);
    }
  }

  async function handleAcknowledge() {
    setAcking(true);
    try {
      await api.ackDataNotice();
      queryClient.invalidateQueries({ queryKey: ["data-notice"] });
      setNoticeOpen(false);
    } finally {
      setAcking(false);
    }
  }

  useEffect(() => {
    setNoticeOpen(error?.code === "data_notice_required");
  }, [error]);

  if (!id) return null;

  const currentIndex = progress
    ? STAGE_ORDER.indexOf(progress.stage as StageId)
    : running
      ? 0
      : -1;

  return (
    <div className="mx-auto max-w-3xl space-y-6 p-6">
      <header>
        <h1 className="text-xl font-semibold">Running decisions</h1>
        <p className="text-sm text-muted-foreground">
          Gates, platform detection, applicability, then per-type decisions.
        </p>
      </header>

      <ol className="flex flex-wrap gap-4">
        {STAGE_ORDER.map((stageId, index) => {
          const state = index < currentIndex ? "done" : index === currentIndex ? "current" : "upcoming";
          return (
            <li key={stageId} className="flex items-center gap-2">
              <span
                className={cn(
                  "flex size-6 shrink-0 items-center justify-center rounded-full border text-xs font-medium",
                  state === "done" && "border-primary bg-primary text-primary-foreground",
                  state === "current" && "border-primary text-primary",
                  state === "upcoming" && "border-border text-muted-foreground",
                )}
              >
                {state === "done" ? <CheckIcon className="size-3.5" /> : index + 1}
              </span>
              <span
                className={cn(
                  "text-sm font-medium",
                  state === "upcoming" && "font-normal text-muted-foreground",
                )}
              >
                {STAGE_LABELS[stageId]}
              </span>
            </li>
          );
        })}
      </ol>

      {progress?.stage === "decisions" && progress.total > 0 && (
        <div className="space-y-1.5">
          <Progress value={(progress.done / progress.total) * 100} />
          <p className="text-xs text-muted-foreground">
            {progress.done} / {progress.total} decisions
          </p>
        </div>
      )}

      {running && !error && !outOfRemit && (
        <p className="text-sm text-muted-foreground">Working…</p>
      )}

      {outOfRemit && (
        <div className="space-y-3">
          <Alert>
            <AlertTitle>This doesn't look like a technology decision request</AlertTitle>
            <AlertDescription>
              The gates stage determined this session isn't a technical decision request that
              BISTEC Architect can help with.
            </AlertDescription>
          </Alert>
          <Button variant="outline" render={<Link to={`/session/${id}/brief`} />}>
            Back to brief
          </Button>
        </div>
      )}

      {error && (
        <div className="space-y-3">
          <Alert variant="destructive">
            <AlertTitle>
              Decision run failed{error.stage ? ` at stage "${error.stage}"` : ""}
            </AlertTitle>
            <AlertDescription>{error.message}</AlertDescription>
          </Alert>
          {error.code === "no_api_key" && (
            <p className="text-sm">
              <Link to="/settings" className="underline underline-offset-2">
                Go to Settings to set an OpenRouter API key
              </Link>
            </p>
          )}
          <Button onClick={handleRetry}>Retry</Button>
        </div>
      )}

      <DataNotice
        open={noticeOpen}
        onOpenChange={setNoticeOpen}
        acked={noticeQuery.data?.acked ?? false}
        text={noticeQuery.data?.text ?? ""}
        onAcknowledge={handleAcknowledge}
        acknowledging={acking}
      />
    </div>
  );
}

export default Run;
