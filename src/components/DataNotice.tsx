import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";

interface DataNoticeProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  acked: boolean;
  text: string;
  onAcknowledge: () => void;
  acknowledging?: boolean;
}

/**
 * The first-run data-handling notice (AC-17). Blocks starting a session until
 * acknowledged; also reachable read-only from Settings once already acked.
 */
export function DataNotice({
  open,
  onOpenChange,
  acked,
  text,
  onAcknowledge,
  acknowledging = false,
}: DataNoticeProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent showCloseButton={acked}>
        <DialogHeader>
          <DialogTitle>Data notice</DialogTitle>
          <DialogDescription className="whitespace-pre-wrap text-foreground">
            {text}
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          {acked ? (
            <Button variant="outline" onClick={() => onOpenChange(false)}>
              Close
            </Button>
          ) : (
            <Button onClick={onAcknowledge} disabled={acknowledging}>
              {acknowledging ? "Acknowledging…" : "Acknowledge & continue"}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
