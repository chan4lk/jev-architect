import { useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { DataNotice } from "@/components/DataNotice";
import { api, describeStartError } from "@/lib/ipc";
import { AppError, type SessionSummary } from "@/lib/schemas";

const MIN_DESCRIBE_LENGTH = 20;

function sessionRoute(session: SessionSummary): string {
  if (session.stage === "review") return `/session/${session.id}/brief`;
  if (session.stage === "done" || session.stage === "out_of_remit") {
    return `/session/${session.id}/report`;
  }
  return `/session/${session.id}/run`;
}

export function Home() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const [tab, setTab] = useState<"describe" | "upload">("describe");
  const [text, setText] = useState("");
  const [noticeOpen, setNoticeOpen] = useState(false);
  const pendingActionRef = useRef<(() => void) | null>(null);

  const noticeQuery = useQuery({ queryKey: ["data-notice"], queryFn: api.dataNotice });
  const sessionsQuery = useQuery({ queryKey: ["sessions"], queryFn: api.listSessions });

  const ackMutation = useMutation({
    mutationFn: () => api.ackDataNotice(),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["data-notice"] });
      setNoticeOpen(false);
      const action = pendingActionRef.current;
      pendingActionRef.current = null;
      action?.();
    },
    onError: (err) => {
      toast.error(err instanceof AppError ? err.message : "Could not acknowledge the notice.");
    },
  });

  const describeMutation = useMutation({
    mutationFn: (value: string) => api.startDescribe(value),
    onSuccess: (view) => navigate(`/session/${view.session.id}/brief`),
    onError: (err) => {
      toast.error(err instanceof AppError ? describeStartError(err) : "Could not start the session.");
    },
  });

  const uploadMutation = useMutation({
    mutationFn: (path: string) => api.startUpload(path),
    onSuccess: (view) => navigate(`/session/${view.session.id}/brief`),
    onError: (err) => {
      toast.error(err instanceof AppError ? describeStartError(err) : "Could not start the session.");
    },
  });

  function gate(action: () => void) {
    if (noticeQuery.data?.acked) {
      action();
    } else {
      pendingActionRef.current = action;
      setNoticeOpen(true);
    }
  }

  function handleDescribeSubmit() {
    gate(() => describeMutation.mutate(text));
  }

  function handleUploadClick() {
    gate(async () => {
      const path = await api.pickDocument();
      if (path) uploadMutation.mutate(path);
    });
  }

  const trimmedLength = text.trim().length;
  const canSubmitDescribe = trimmedLength >= MIN_DESCRIBE_LENGTH;
  const sessions = sessionsQuery.data ?? [];

  return (
    <div className="mx-auto max-w-4xl space-y-8 p-6">
      <header>
        <h1 className="text-xl font-semibold">New decision session</h1>
        <p className="text-sm text-muted-foreground">
          Describe a project, or upload a requirements document, and BISTEC Architect will
          propose the technology decisions.
        </p>
      </header>

      <Tabs value={tab} onValueChange={(value) => setTab(value as "describe" | "upload")}>
        <TabsList>
          <TabsTrigger value="describe">Describe</TabsTrigger>
          <TabsTrigger value="upload">Upload requirements</TabsTrigger>
        </TabsList>

        <TabsContent value="describe" className="mt-4 space-y-3">
          <Label htmlFor="describe-text">Project description</Label>
          <Textarea
            id="describe-text"
            rows={8}
            value={text}
            onChange={(event) => setText(event.target.value)}
            placeholder="Describe the project: what it does, who uses it, scale, budget, team skills, compliance needs…"
          />
          {!canSubmitDescribe && (
            <p className="text-xs text-muted-foreground">
              Add at least {MIN_DESCRIBE_LENGTH} characters ({trimmedLength}/{MIN_DESCRIBE_LENGTH}) so
              there's enough to extract a brief from.
            </p>
          )}
          <Button onClick={handleDescribeSubmit} disabled={!canSubmitDescribe || describeMutation.isPending}>
            {describeMutation.isPending ? "Extracting brief…" : "Extract brief"}
          </Button>
        </TabsContent>

        <TabsContent value="upload" className="mt-4 space-y-3">
          <p className="text-sm text-muted-foreground">
            Accepts PDF, Word (.docx), Markdown, and plain text documents.
          </p>
          <Button onClick={handleUploadClick} disabled={uploadMutation.isPending}>
            {uploadMutation.isPending ? "Extracting brief…" : "Choose a document…"}
          </Button>
        </TabsContent>
      </Tabs>

      <section>
        <h2 className="mb-3 text-lg font-semibold">Session history</h2>
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Title</TableHead>
              <TableHead>Date</TableHead>
              <TableHead>Mode</TableHead>
              <TableHead>Stage</TableHead>
              <TableHead className="text-right">Decisions</TableHead>
              <TableHead className="text-right">Needs architect</TableHead>
              <TableHead className="text-right">Unreviewed</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {sessions.length === 0 && (
              <TableRow>
                <TableCell colSpan={7} className="text-center text-muted-foreground">
                  No sessions yet.
                </TableCell>
              </TableRow>
            )}
            {sessions.map((session) => (
              <TableRow
                key={session.id}
                className="cursor-pointer"
                onClick={() => navigate(sessionRoute(session))}
              >
                <TableCell className="font-medium">{session.title}</TableCell>
                <TableCell>{new Date(session.created_at).toLocaleString()}</TableCell>
                <TableCell className="capitalize">{session.mode}</TableCell>
                <TableCell>{session.stage}</TableCell>
                <TableCell className="text-right tabular-nums">{session.decision_count}</TableCell>
                <TableCell className="text-right tabular-nums">
                  {session.needs_architect_count}
                </TableCell>
                <TableCell className="text-right tabular-nums">{session.unreviewed_count}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </section>

      <DataNotice
        open={noticeOpen}
        onOpenChange={setNoticeOpen}
        acked={noticeQuery.data?.acked ?? false}
        text={noticeQuery.data?.text ?? ""}
        onAcknowledge={() => ackMutation.mutate()}
        acknowledging={ackMutation.isPending}
      />
    </div>
  );
}

export default Home;
