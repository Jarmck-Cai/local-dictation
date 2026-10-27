import type { CloudStatus, DictationTiming } from "./dictationApi";

export function pasteErrorCode(
  payload: unknown,
): "clipboard_restore_failed" | "input_unconfirmed" | "failed" {
  const code =
    typeof payload === "object" && payload !== null
      ? (payload as { code?: unknown }).code
      : undefined;
  if (code === "clipboard_restore_failed" || code === "input_unconfirmed")
    return code;
  return "failed";
}

/** Prevent old snapshots from overwriting a live event, retry, or unmounted view. */
export class SnapshotGuard {
  private generation = 0;
  private disposed = false;

  begin(): number {
    return ++this.generation;
  }

  invalidate(): void {
    this.generation += 1;
  }

  isCurrent(ticket: number): boolean {
    return !this.disposed && ticket === this.generation;
  }

  isAlive(): boolean {
    return !this.disposed;
  }

  dispose(): void {
    this.disposed = true;
    this.invalidate();
  }
}

export function mergeTiming(
  previous: DictationTiming[],
  next: DictationTiming,
  limit = 100,
): DictationTiming[] {
  return [next, ...previous.filter((item) => item.id !== next.id)]
    .sort((left, right) => right.recorded_at_ms - left.recorded_at_ms)
    .slice(0, limit);
}

export function cloudState(
  status: CloudStatus,
  receivedAt: number,
  now: number,
): "manual" | "cooldown" | "nextRecording" | "active" {
  if (status.manual_paused) return "manual";
  if (status.cooldown_remaining_ms > 0) {
    return now - receivedAt < status.cooldown_remaining_ms
      ? "cooldown"
      : "nextRecording";
  }
  // A fresh snapshot after expiry has no remaining milliseconds, but failures
  // still mark the half-open state until a normal recording retries the cloud.
  if (status.consecutive_failures >= 3) return "nextRecording";
  return "active";
}

export function distinctCleanupReason(timing: DictationTiming): string | null {
  const reason = timing.cleanup_reason?.trim();
  return reason && reason !== timing.outcome ? reason : null;
}
