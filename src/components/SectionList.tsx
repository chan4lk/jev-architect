import { useEffect, useRef, useState } from "react";
import { ChevronDownIcon, ChevronRightIcon } from "lucide-react";

import { cn } from "@/lib/utils";
import type { Section } from "@/lib/schemas";

interface SectionListProps {
  sections: Section[];
  /** Section id to open, scroll to, and highlight (e.g. from clicking a citation chip). */
  highlightedId?: string | null;
  className?: string;
}

/**
 * The document-sections panel shown beside the brief editor for upload sessions
 * (FR-8). Each section is collapsible; `highlightedId` opens and scrolls to the
 * matching section so citation chips in the brief editor can jump to their source.
 */
export function SectionList({ sections, highlightedId, className }: SectionListProps) {
  const [openIds, setOpenIds] = useState<Set<string>>(new Set());
  const nodeRefs = useRef<Record<string, HTMLDivElement | null>>({});

  useEffect(() => {
    if (!highlightedId) return;
    setOpenIds((prev) => (prev.has(highlightedId) ? prev : new Set(prev).add(highlightedId)));
    nodeRefs.current[highlightedId]?.scrollIntoView?.({ behavior: "smooth", block: "center" });
  }, [highlightedId]);

  function toggle(id: string) {
    setOpenIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  return (
    <div className={cn("space-y-2", className)}>
      <h2 className="text-sm font-semibold">Document sections</h2>
      <div className="space-y-1.5">
        {sections.map((section) => {
          const open = openIds.has(section.id);
          const highlighted = highlightedId === section.id;
          return (
            <div
              key={section.id}
              id={`section-${section.id}`}
              ref={(el) => {
                nodeRefs.current[section.id] = el;
              }}
              className={cn(
                "rounded-lg border border-border p-2 transition-colors",
                highlighted && "border-primary ring-2 ring-ring",
              )}
            >
              <button
                type="button"
                className="flex w-full items-center justify-between gap-2 text-left text-sm"
                aria-expanded={open}
                onClick={() => toggle(section.id)}
              >
                <span className="flex min-w-0 items-center gap-1.5">
                  {open ? (
                    <ChevronDownIcon className="size-3.5 shrink-0 text-muted-foreground" />
                  ) : (
                    <ChevronRightIcon className="size-3.5 shrink-0 text-muted-foreground" />
                  )}
                  <span className="shrink-0 font-mono text-xs text-muted-foreground">{section.id}</span>
                  <span className="truncate font-medium">{section.heading ?? "Untitled section"}</span>
                </span>
                <span className="shrink-0 text-xs text-muted-foreground">{section.tokens} tokens</span>
              </button>
              {open && <p className="mt-2 text-sm text-muted-foreground">{section.text}</p>}
            </div>
          );
        })}
      </div>
    </div>
  );
}

export default SectionList;
