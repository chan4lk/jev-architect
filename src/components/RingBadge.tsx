import type { Ring } from "@/lib/schemas";
import { cn } from "@/lib/utils";

const RING_LABEL: Record<Ring, string> = {
  adopt: "Adopt",
  trial: "Trial",
  hold: "Hold",
};

const RING_HINT: Record<Ring, string> = {
  adopt: "BISTEC default",
  trial: "Alternative",
  hold: "Avoid",
};

const RING_CLASS: Record<Ring, string> = {
  adopt: "bg-ring-adopt text-ring-adopt-foreground",
  trial: "bg-ring-trial text-ring-trial-foreground",
  hold: "bg-ring-hold text-ring-hold-foreground",
};

/**
 * Ring badge (Adopt / Trial / Hold). The label is always rendered as text —
 * never colour alone — so it reads correctly for colour-blind users and in
 * print. Adopt = BISTEC default, Trial = alternative, Hold = avoid.
 */
export function RingBadge({ ring, className }: { ring: Ring; className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex w-fit shrink-0 items-center gap-1 rounded-full px-2 py-0.5 text-xs font-medium whitespace-nowrap",
        RING_CLASS[ring],
        className,
      )}
      title={RING_HINT[ring]}
    >
      {RING_LABEL[ring]}
    </span>
  );
}

/** Small colour dot used inline in ProbBars, tagged with the same ring class. */
export function RingDot({ ring, className }: { ring: Ring; className?: string }) {
  return (
    <span
      aria-hidden="true"
      title={RING_LABEL[ring]}
      className={cn("size-2 shrink-0 rounded-full", RING_CLASS[ring].split(" ")[0], className)}
    />
  );
}
