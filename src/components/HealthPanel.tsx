import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { CheckIcon, XIcon } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { api } from "@/lib/ipc";
import { AppError, type Health } from "@/lib/schemas";
import { cn } from "@/lib/utils";

function StatusRow({ label, ok }: { label: string; ok: boolean }) {
  return (
    <div className="flex items-center justify-between border-b border-border py-2 text-sm last:border-b-0">
      <span>{label}</span>
      <Badge
        variant={ok ? "default" : "destructive"}
        className={cn(!ok && "bg-destructive/15 text-destructive")}
      >
        {ok ? <CheckIcon className="size-3" /> : <XIcon className="size-3" />}
        {ok ? "OK" : "Fail"}
      </Badge>
    </div>
  );
}

/** "Check connections" panel used by Settings (AC-20 UI part). */
export function HealthPanel() {
  const [health, setHealth] = useState<Health | null>(null);
  const [copied, setCopied] = useState(false);

  const checkMutation = useMutation({
    mutationFn: () => api.healthCheck(),
    onSuccess: (result) => setHealth(result),
    onError: (err) => {
      toast.error(err instanceof AppError ? err.message : "Health check failed.");
    },
  });

  async function copyPullCommand() {
    if (!health) return;
    try {
      await navigator.clipboard.writeText(health.pull_command);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      toast.error("Could not copy to clipboard.");
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Connections</CardTitle>
        <CardDescription>
          Check that the local Ollama model and the OpenRouter API key are reachable.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <Button
          onClick={() => checkMutation.mutate()}
          disabled={checkMutation.isPending}
          variant="outline"
        >
          {checkMutation.isPending ? "Checking…" : "Check connections"}
        </Button>

        {health && (
          <div>
            <StatusRow label="Ollama reachable" ok={health.ollama_reachable} />
            <StatusRow label={`Model present (${health.model})`} ok={health.model_present} />
            <StatusRow label="OpenRouter key" ok={health.openrouter === "ok"} />

            {!health.model_present && (
              <div className="mt-3 space-y-1.5">
                <p className="text-sm text-muted-foreground">
                  The configured model isn't pulled yet. Run:
                </p>
                <div className="flex items-center gap-2">
                  <code className="flex-1 overflow-x-auto rounded-lg bg-muted px-2.5 py-2 font-mono text-xs">
                    {health.pull_command}
                  </code>
                  <Button size="sm" variant="outline" onClick={copyPullCommand}>
                    {copied ? "Copied" : "Copy"}
                  </Button>
                </div>
              </div>
            )}

            {health.openrouter === "error" && health.openrouter_error && (
              <p className="mt-2 text-sm text-destructive">{health.openrouter_error}</p>
            )}
            {health.openrouter === "no_key" && (
              <p className="mt-2 text-sm text-muted-foreground">
                No OpenRouter API key is set yet.
              </p>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
