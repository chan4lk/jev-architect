import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { TriangleAlertIcon } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { DataNotice } from "@/components/DataNotice";
import { HealthPanel } from "@/components/HealthPanel";
import { api } from "@/lib/ipc";
import { AppError, type Settings as SettingsDto } from "@/lib/schemas";

export function Settings() {
  const queryClient = useQueryClient();

  const settingsQuery = useQuery({ queryKey: ["settings"], queryFn: api.getSettings });
  const criteriaQuery = useQuery({ queryKey: ["criteria"], queryFn: api.getCriteria });
  const hasKeyQuery = useQuery({ queryKey: ["has-api-key"], queryFn: api.hasApiKey });
  const noticeQuery = useQuery({ queryKey: ["data-notice"], queryFn: api.dataNotice });

  const [form, setForm] = useState<SettingsDto | null>(null);
  const [apiKeyInput, setApiKeyInput] = useState("");
  const [noticeOpen, setNoticeOpen] = useState(false);

  // Seed the local editable copy once, the first time settings load.
  useEffect(() => {
    if (settingsQuery.data && form === null) {
      setForm(settingsQuery.data);
    }
  }, [settingsQuery.data, form]);

  const saveMutation = useMutation({
    mutationFn: (next: SettingsDto) => api.saveSettings(next),
    onSuccess: (saved) => {
      setForm(saved);
      queryClient.setQueryData(["settings"], saved);
      toast.success("Settings saved.");
    },
    onError: (err) => {
      toast.error(err instanceof AppError ? err.message : "Could not save settings.");
    },
  });

  const setKeyMutation = useMutation({
    mutationFn: (key: string) => api.setApiKey(key),
    onSuccess: () => {
      setApiKeyInput("");
      queryClient.invalidateQueries({ queryKey: ["has-api-key"] });
      toast.success("API key saved.");
    },
    onError: (err) => {
      toast.error(err instanceof AppError ? err.message : "Could not save the API key.");
    },
  });

  const clearKeyMutation = useMutation({
    mutationFn: () => api.clearApiKey(),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["has-api-key"] });
      toast.success("API key cleared.");
    },
    onError: (err) => {
      toast.error(err instanceof AppError ? err.message : "Could not clear the API key.");
    },
  });

  const ackMutation = useMutation({
    mutationFn: () => api.ackDataNotice(),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["data-notice"] });
    },
    onError: (err) => {
      toast.error(err instanceof AppError ? err.message : "Could not acknowledge the notice.");
    },
  });

  if (!form) {
    return (
      <div className="mx-auto max-w-3xl p-6 text-sm text-muted-foreground">Loading settings…</div>
    );
  }

  function updateField<K extends keyof SettingsDto>(key: K, value: SettingsDto[K]) {
    setForm((current) => (current ? { ...current, [key]: value } : current));
  }

  function updateWeight(criterionId: string, value: number) {
    setForm((current) =>
      current ? { ...current, weights: { ...current.weights, [criterionId]: value } } : current,
    );
  }

  return (
    <div className="mx-auto max-w-3xl space-y-6 p-6">
      <h1 className="text-xl font-semibold">Settings</h1>

      <Card>
        <CardHeader>
          <CardTitle>OpenRouter API key</CardTitle>
          <CardDescription>
            Used to call the Jev model for architecture decisions. Once saved, the key is never
            displayed again — only whether one is set.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="space-y-1.5">
            <Label htmlFor="api-key">API key</Label>
            <Input
              id="api-key"
              type="password"
              autoComplete="off"
              value={apiKeyInput}
              onChange={(event) => setApiKeyInput(event.target.value)}
              placeholder={hasKeyQuery.data ? "Enter a new key to replace it" : "sk-or-…"}
            />
          </div>
          <div className="flex items-center gap-2">
            <Button
              size="sm"
              disabled={!apiKeyInput.trim() || setKeyMutation.isPending}
              onClick={() => setKeyMutation.mutate(apiKeyInput.trim())}
            >
              Save key
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={!hasKeyQuery.data || clearKeyMutation.isPending}
              onClick={() => clearKeyMutation.mutate()}
            >
              Clear
            </Button>
            {hasKeyQuery.data ? (
              <Badge>Key is set ✓</Badge>
            ) : (
              <Badge variant="outline">No key set</Badge>
            )}
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Jev</CardTitle>
          <CardDescription>The OpenRouter model used for architecture decisions.</CardDescription>
        </CardHeader>
        <CardContent className="grid gap-3 sm:grid-cols-2">
          <div className="space-y-1.5">
            <Label htmlFor="jev-model">Model</Label>
            <Input
              id="jev-model"
              value={form.jev_model}
              onChange={(event) => updateField("jev_model", event.target.value)}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="jev-base-url">Base URL</Label>
            <Input
              id="jev-base-url"
              value={form.jev_base_url}
              onChange={(event) => updateField("jev_base_url", event.target.value)}
            />
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Local model</CardTitle>
          <CardDescription>The Ollama instance used for local brief extraction.</CardDescription>
        </CardHeader>
        <CardContent className="grid gap-3 sm:grid-cols-2">
          <div className="space-y-1.5">
            <Label htmlFor="ollama-base-url">Ollama URL</Label>
            <Input
              id="ollama-base-url"
              value={form.ollama_base_url}
              onChange={(event) => updateField("ollama_base_url", event.target.value)}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="ollama-model">Model</Label>
            <Input
              id="ollama-model"
              value={form.ollama_model}
              onChange={(event) => updateField("ollama_model", event.target.value)}
            />
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Decision thresholds</CardTitle>
          <CardDescription>Tuning knobs for the decision pipeline.</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid gap-3 sm:grid-cols-2">
            <div className="space-y-1.5">
              <Label htmlFor="confidence-threshold">Confidence threshold</Label>
              <Input
                id="confidence-threshold"
                type="number"
                step="0.01"
                min="0"
                max="1"
                value={form.confidence_threshold}
                onChange={(event) => updateField("confidence_threshold", Number(event.target.value))}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="min-margin">Minimum margin</Label>
              <Input
                id="min-margin"
                type="number"
                step="0.01"
                min="0"
                max="1"
                value={form.min_margin}
                onChange={(event) => updateField("min_margin", Number(event.target.value))}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="token-budget">Token budget</Label>
              <Input
                id="token-budget"
                type="number"
                min="0"
                value={form.state_token_budget}
                onChange={(event) => updateField("state_token_budget", Number(event.target.value))}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="max-questions">Max questions per call</Label>
              <Input
                id="max-questions"
                type="number"
                min="0"
                value={form.max_questions_per_call}
                onChange={(event) => updateField("max_questions_per_call", Number(event.target.value))}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="max-concurrency">Max concurrent calls</Label>
              <Input
                id="max-concurrency"
                type="number"
                min="1"
                value={form.max_concurrent_calls}
                onChange={(event) => updateField("max_concurrent_calls", Number(event.target.value))}
              />
            </div>
          </div>

          <Alert>
            <TriangleAlertIcon />
            <AlertDescription>
              Uncalibrated defaults — run the calibration before relying on these.
            </AlertDescription>
          </Alert>

          <div className="space-y-2">
            <span className="text-sm font-medium">Criterion weights</span>
            <div className="grid gap-3 sm:grid-cols-2">
              {(criteriaQuery.data ?? []).map((criterion) => (
                <div key={criterion.id} className="space-y-1.5">
                  <Label htmlFor={`weight-${criterion.id}`}>{criterion.name}</Label>
                  <Input
                    id={`weight-${criterion.id}`}
                    type="number"
                    step="0.01"
                    min="0"
                    max="1"
                    value={form.weights[criterion.id] ?? criterion.weight}
                    onChange={(event) => updateWeight(criterion.id, Number(event.target.value))}
                  />
                </div>
              ))}
            </div>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Reviewer name</CardTitle>
          <CardDescription>
            Required to accept, override, or reject decisions — reviews are attributed to this
            name.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-1.5">
          <Label htmlFor="reviewer-name">Your name</Label>
          <Input
            id="reviewer-name"
            value={form.reviewer_name}
            onChange={(event) => updateField("reviewer_name", event.target.value)}
          />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Data notice</CardTitle>
          <CardDescription>
            {noticeQuery.data?.acked ? "Acknowledged." : "Not yet acknowledged."}
          </CardDescription>
        </CardHeader>
        <CardContent>
          <Button variant="outline" onClick={() => setNoticeOpen(true)}>
            View data notice
          </Button>
        </CardContent>
      </Card>

      <div className="flex justify-end">
        <Button onClick={() => saveMutation.mutate(form)} disabled={saveMutation.isPending}>
          {saveMutation.isPending ? "Saving…" : "Save settings"}
        </Button>
      </div>

      <HealthPanel />

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

export default Settings;
