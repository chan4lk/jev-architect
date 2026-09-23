import { useEffect, useMemo, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { BriefEditor } from "@/components/BriefEditor";
import { SectionList } from "@/components/SectionList";
import { api } from "@/lib/ipc";
import { AppError, type Brief as BriefDto } from "@/lib/schemas";

/**
 * The brief review screen (FR-8, AC-7 UI part). Nothing reaches Jev until the
 * user confirms here — the summary, context assessment, and requirement/NFR/
 * constraint/team-skill lists are all editable, with document sections shown
 * alongside for upload sessions.
 */
export function Brief() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const sessionQuery = useQuery({
    queryKey: ["session", id],
    queryFn: () => api.getSession(id as string),
    enabled: !!id,
  });

  const [draft, setDraft] = useState<BriefDto | null>(null);
  const [highlightedSectionId, setHighlightedSectionId] = useState<string | null>(null);

  // Seed the local editable copy once, the first time the brief loads.
  useEffect(() => {
    if (sessionQuery.data?.brief && draft === null) {
      setDraft(sessionQuery.data.brief);
    }
  }, [sessionQuery.data, draft]);

  const dirty = useMemo(() => {
    if (!sessionQuery.data?.brief || !draft) return false;
    return JSON.stringify(sessionQuery.data.brief) !== JSON.stringify(draft);
  }, [sessionQuery.data, draft]);

  const saveMutation = useMutation({
    mutationFn: (brief: BriefDto) => api.updateBrief(id as string, brief),
    onSuccess: (view) => {
      queryClient.setQueryData(["session", id], view);
      setDraft(view.brief);
      toast.success("Brief saved.");
    },
    onError: (err) => {
      toast.error(err instanceof AppError ? err.message : "Could not save the brief.");
    },
  });

  const confirmMutation = useMutation({
    mutationFn: async () => {
      if (dirty && draft) {
        await api.updateBrief(id as string, draft);
      }
      return api.confirmBrief(id as string);
    },
    onSuccess: (view) => {
      queryClient.setQueryData(["session", id], view);
      navigate(`/session/${id}/run`);
    },
    onError: (err) => {
      toast.error(err instanceof AppError ? err.message : "Could not confirm the brief.");
    },
  });

  if (!id) return null;

  if (sessionQuery.isLoading) {
    return (
      <div className="mx-auto max-w-5xl p-6 text-sm text-muted-foreground">Loading brief…</div>
    );
  }

  if (sessionQuery.isError) {
    const err = sessionQuery.error;
    return (
      <div className="mx-auto max-w-5xl p-6">
        <Alert variant="destructive">
          <AlertDescription>
            {err instanceof AppError ? err.message : "Could not load this session."}
          </AlertDescription>
        </Alert>
      </div>
    );
  }

  const view = sessionQuery.data;

  if (!view || !draft) {
    return (
      <div className="mx-auto max-w-5xl p-6 text-sm text-muted-foreground">
        There is no brief to review for this session yet.
      </div>
    );
  }

  const isUpload = view.session.mode === "upload";
  const summaryEmpty = draft.summary.trim().length === 0;

  return (
    <div className="mx-auto max-w-6xl space-y-6 p-6">
      <header className="space-y-1">
        <div className="flex flex-wrap items-center gap-2">
          <h1 className="text-xl font-semibold">{view.session.title}</h1>
          <Badge variant="outline" className="capitalize">
            {view.session.mode}
          </Badge>
          {dirty && <Badge variant="secondary">Unsaved changes</Badge>}
        </div>
        {isUpload && (
          <p className="text-sm text-muted-foreground">
            Document: <span className="font-medium text-foreground">{view.session.doc_name}</span>
            {" — "}
            {view.doc_path === "large"
              ? `Large document — brief built locally by MiniCPM from ${view.sections?.length ?? 0} sections.`
              : "Small document — sent to Jev whole."}
          </p>
        )}
        <p className="text-xs text-muted-foreground">Nothing is sent to Jev until you confirm.</p>
      </header>

      <div className={isUpload ? "grid gap-6 lg:grid-cols-[minmax(0,1fr)_320px]" : undefined}>
        <BriefEditor
          brief={draft}
          onChange={setDraft}
          onCitationClick={isUpload ? setHighlightedSectionId : undefined}
        />
        {isUpload && view.sections && (
          <SectionList
            sections={view.sections}
            highlightedId={highlightedSectionId}
            className="lg:sticky lg:top-6 lg:self-start"
          />
        )}
      </div>

      <div className="flex flex-wrap items-center gap-3 border-t border-border pt-4">
        <Button
          variant="outline"
          disabled={!dirty || saveMutation.isPending}
          onClick={() => draft && saveMutation.mutate(draft)}
        >
          {saveMutation.isPending ? "Saving…" : "Save changes"}
        </Button>
        <Button
          disabled={summaryEmpty || confirmMutation.isPending}
          onClick={() => confirmMutation.mutate()}
        >
          {confirmMutation.isPending ? "Confirming…" : "Confirm brief & run decisions"}
        </Button>
      </div>
    </div>
  );
}

export default Brief;
