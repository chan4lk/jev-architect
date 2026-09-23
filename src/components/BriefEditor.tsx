import { useState } from "react";
import { XIcon } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import type { Brief, BriefItem, ContextAssessment } from "@/lib/schemas";

type Scale = ContextAssessment["scale"];
type Budget = ContextAssessment["budget"];
type Timeline = ContextAssessment["timeline"];
type TeamSize = ContextAssessment["team_size"];
type DataSensitivity = ContextAssessment["data_sensitivity"];
type Compliance = ContextAssessment["compliance"][number];

// ---------------------------------------------------------------------------
// Context assessment option catalogues (skill wording, see task brief FR-8)
// ---------------------------------------------------------------------------

const SCALE_OPTIONS: { value: Scale; label: string }[] = [
  { value: "small", label: "Small — <1K users" },
  { value: "medium", label: "Medium — <100K" },
  { value: "large", label: "Large — 100K+" },
  { value: "unknown", label: "Unknown" },
];

const BUDGET_OPTIONS: { value: Budget; label: string }[] = [
  { value: "tight", label: "Tight — <$500/mo" },
  { value: "moderate", label: "Moderate — <$5K/mo" },
  { value: "enterprise", label: "Enterprise — >$5K/mo" },
  { value: "unknown", label: "Unknown" },
];

const TIMELINE_OPTIONS: { value: Timeline; label: string }[] = [
  { value: "urgent", label: "Urgent — <4 weeks" },
  { value: "normal", label: "Normal — 1–3 months" },
  { value: "long_term", label: "Long-term — 3+ months" },
  { value: "unknown", label: "Unknown" },
];

const TEAM_SIZE_OPTIONS: { value: TeamSize; label: string }[] = [
  { value: "solo_pair", label: "Solo/Pair" },
  { value: "small", label: "Small — 3–5" },
  { value: "large", label: "Large — 5+" },
  { value: "unknown", label: "Unknown" },
];

const DATA_SENSITIVITY_OPTIONS: { value: DataSensitivity; label: string }[] = [
  { value: "public", label: "Public" },
  { value: "internal", label: "Internal" },
  { value: "confidential", label: "Confidential" },
  { value: "restricted", label: "Restricted" },
  { value: "unknown", label: "Unknown" },
];

const COMPLIANCE_OPTIONS: { value: Compliance; label: string }[] = [
  { value: "soc2", label: "SOC2" },
  { value: "gdpr", label: "GDPR" },
  { value: "hipaa", label: "HIPAA" },
  { value: "industry_specific", label: "Industry-specific" },
];

// ---------------------------------------------------------------------------
// Context assessment select field
// ---------------------------------------------------------------------------

function ContextSelectField<V extends string>({
  id,
  label,
  value,
  options,
  onChange,
}: {
  id: string;
  label: string;
  value: V;
  options: { value: V; label: string }[];
  onChange: (value: V) => void;
}) {
  return (
    <div className="space-y-1.5">
      <Label htmlFor={id}>{label}</Label>
      <Select value={value} onValueChange={(next) => onChange(next as V)}>
        <SelectTrigger id={id} className="w-full">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {options.map((option) => (
            <SelectItem key={option.value} value={option.value}>
              {option.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {value === "unknown" && (
        <p className="text-xs font-medium text-amber-600 dark:text-amber-500">
          Not stated — please fill in if you know.
        </p>
      )}
    </div>
  );
}

function CompliancePicker({
  value,
  onChange,
}: {
  value: Compliance[];
  onChange: (value: Compliance[]) => void;
}) {
  function toggle(item: Compliance) {
    onChange(value.includes(item) ? value.filter((v) => v !== item) : [...value, item]);
  }

  return (
    <div className="space-y-1.5">
      <span className="text-sm font-medium">Compliance</span>
      <div className="flex flex-wrap gap-2">
        {COMPLIANCE_OPTIONS.map((option) => {
          const active = value.includes(option.value);
          return (
            <Button
              key={option.value}
              type="button"
              size="sm"
              variant={active ? "default" : "outline"}
              aria-pressed={active}
              onClick={() => toggle(option.value)}
            >
              {option.label}
            </Button>
          );
        })}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Editable BriefItem lists (requirements / nfrs / constraints / team_skills)
// ---------------------------------------------------------------------------

function BriefItemListEditor({
  title,
  items,
  onChange,
  onCitationClick,
}: {
  title: string;
  items: BriefItem[];
  onChange: (items: BriefItem[]) => void;
  onCitationClick?: (sectionId: string) => void;
}) {
  const [draft, setDraft] = useState("");

  function updateText(index: number, text: string) {
    const next = items.slice();
    next[index] = { ...next[index], text };
    onChange(next);
  }

  function remove(index: number) {
    onChange(items.filter((_, i) => i !== index));
  }

  function add() {
    const text = draft.trim();
    if (!text) return;
    onChange([...items, { text, sources: [] }]);
    setDraft("");
  }

  return (
    <div className="space-y-2">
      <h3 className="text-sm font-semibold">{title}</h3>
      {items.length === 0 && <p className="text-sm text-muted-foreground">None yet.</p>}
      <ul className="space-y-2">
        {items.map((item, index) => (
          <li key={index} className="flex items-start gap-2">
            <Input
              aria-label={`${title} item ${index + 1}`}
              value={item.text}
              onChange={(event) => updateText(index, event.target.value)}
              className="flex-1"
            />
            {item.sources.length > 0 && (
              <div className="flex flex-wrap items-center gap-1 pt-1.5">
                {item.sources.map((sourceId) => (
                  <Badge
                    key={sourceId}
                    variant="outline"
                    role={onCitationClick ? "button" : undefined}
                    tabIndex={onCitationClick ? 0 : undefined}
                    aria-label={onCitationClick ? `Jump to section ${sourceId}` : undefined}
                    className={onCitationClick ? "cursor-pointer" : undefined}
                    onClick={onCitationClick ? () => onCitationClick(sourceId) : undefined}
                    onKeyDown={
                      onCitationClick
                        ? (event) => {
                            if (event.key === "Enter" || event.key === " ") {
                              event.preventDefault();
                              onCitationClick(sourceId);
                            }
                          }
                        : undefined
                    }
                  >
                    {sourceId}
                  </Badge>
                ))}
              </div>
            )}
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              aria-label={`Remove ${title.toLowerCase()} item ${index + 1}`}
              onClick={() => remove(index)}
            >
              <XIcon />
            </Button>
          </li>
        ))}
      </ul>
      <div className="flex gap-2">
        <Input
          aria-label={`Add a ${title.toLowerCase()} item`}
          value={draft}
          placeholder={`Add a ${title.toLowerCase()} item…`}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              add();
            }
          }}
        />
        <Button type="button" variant="outline" disabled={!draft.trim()} onClick={add}>
          Add
        </Button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Mentioned technologies tag editor
// ---------------------------------------------------------------------------

function TechTagsEditor({
  value,
  onChange,
}: {
  value: string[];
  onChange: (value: string[]) => void;
}) {
  const [draft, setDraft] = useState("");

  function add() {
    const tag = draft.trim();
    if (!tag || value.includes(tag)) {
      setDraft("");
      return;
    }
    onChange([...value, tag]);
    setDraft("");
  }

  function remove(tag: string) {
    onChange(value.filter((t) => t !== tag));
  }

  return (
    <div className="space-y-2">
      <h3 className="text-sm font-semibold">Mentioned technologies</h3>
      <div className="flex flex-wrap gap-2">
        {value.map((tag) => (
          <Badge key={tag} variant="secondary" className="gap-1">
            {tag}
            <button type="button" aria-label={`Remove ${tag}`} onClick={() => remove(tag)}>
              <XIcon className="size-3" />
            </button>
          </Badge>
        ))}
      </div>
      <div className="flex gap-2">
        <Input
          aria-label="Add a mentioned technology"
          value={draft}
          placeholder="Add a technology…"
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              add();
            }
          }}
        />
        <Button type="button" variant="outline" disabled={!draft.trim()} onClick={add}>
          Add
        </Button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// BriefEditor
// ---------------------------------------------------------------------------

export interface BriefEditorProps {
  brief: Brief;
  onChange: (brief: Brief) => void;
  /** Present only for upload sessions, where a SectionList is shown alongside. */
  onCitationClick?: (sectionId: string) => void;
  className?: string;
}

/**
 * The editable project brief (FR-8): summary, context assessment, compliance,
 * requirements/NFRs/constraints/team-skills lists, and mentioned technologies.
 * Fully controlled — the parent screen owns the draft state and persistence.
 */
export function BriefEditor({ brief, onChange, onCitationClick, className }: BriefEditorProps) {
  function patch(partial: Partial<Brief>) {
    onChange({ ...brief, ...partial });
  }

  function patchContext(partial: Partial<ContextAssessment>) {
    onChange({ ...brief, context: { ...brief.context, ...partial } });
  }

  return (
    <div className={cn("space-y-6", className)}>
      <div className="space-y-1.5">
        <Label htmlFor="brief-summary">Summary</Label>
        <Textarea
          id="brief-summary"
          rows={6}
          value={brief.summary}
          onChange={(event) => patch({ summary: event.target.value })}
          placeholder="A short summary of the project…"
        />
      </div>

      <div className="space-y-3">
        <h2 className="text-sm font-semibold">Context assessment</h2>
        <div className="grid gap-3 sm:grid-cols-2">
          <ContextSelectField
            id="ctx-scale"
            label="Scale"
            value={brief.context.scale}
            options={SCALE_OPTIONS}
            onChange={(value) => patchContext({ scale: value })}
          />
          <ContextSelectField
            id="ctx-budget"
            label="Budget"
            value={brief.context.budget}
            options={BUDGET_OPTIONS}
            onChange={(value) => patchContext({ budget: value })}
          />
          <ContextSelectField
            id="ctx-timeline"
            label="Timeline"
            value={brief.context.timeline}
            options={TIMELINE_OPTIONS}
            onChange={(value) => patchContext({ timeline: value })}
          />
          <ContextSelectField
            id="ctx-team-size"
            label="Team size"
            value={brief.context.team_size}
            options={TEAM_SIZE_OPTIONS}
            onChange={(value) => patchContext({ team_size: value })}
          />
          <ContextSelectField
            id="ctx-data-sensitivity"
            label="Data sensitivity"
            value={brief.context.data_sensitivity}
            options={DATA_SENSITIVITY_OPTIONS}
            onChange={(value) => patchContext({ data_sensitivity: value })}
          />
        </div>
        <CompliancePicker
          value={brief.context.compliance}
          onChange={(value) => patchContext({ compliance: value })}
        />
      </div>

      <BriefItemListEditor
        title="Requirements"
        items={brief.requirements}
        onChange={(items) => patch({ requirements: items })}
        onCitationClick={onCitationClick}
      />
      <BriefItemListEditor
        title="Non-functional requirements"
        items={brief.nfrs}
        onChange={(items) => patch({ nfrs: items })}
        onCitationClick={onCitationClick}
      />
      <BriefItemListEditor
        title="Constraints"
        items={brief.constraints}
        onChange={(items) => patch({ constraints: items })}
        onCitationClick={onCitationClick}
      />
      <BriefItemListEditor
        title="Team skills"
        items={brief.team_skills}
        onChange={(items) => patch({ team_skills: items })}
        onCitationClick={onCitationClick}
      />

      <TechTagsEditor
        value={brief.mentioned_technologies}
        onChange={(value) => patch({ mentioned_technologies: value })}
      />
    </div>
  );
}

export default BriefEditor;
